#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutlineRegion {
    pub name: Option<String>,
    pub line: usize,
    pub end_line: Option<usize>,
}

impl OutlineRegion {
    /// An unclosed `#Область` spans to the end of the file for containment, but
    /// its reported `end_line` stays unknown.
    pub fn effective_end(&self) -> usize {
        self.end_line.unwrap_or(usize::MAX)
    }
}

use crate::{CodeOutlineMethod, CodeOutlineMethodKind, CodeOutlineParameter};
use bsl_syntax::ast::{AstNode, FunctionDef, Param, ProcedureDef};
use bsl_syntax::{SyntaxKind, SyntaxNode, SyntaxToken};

fn parse_bsl_syntax(text: &str) -> bsl_syntax::Parse<SyntaxNode> {
    bsl_parser::parse(text)
}

pub fn parse_module(text: &str) -> Result<(Vec<CodeOutlineMethod>, Vec<OutlineRegion>), String> {
    if text.len() > u32::MAX as usize {
        return Err("BSL module is too large for the analyzer parser".to_string());
    }
    let parsed = parse_bsl_syntax(text);
    if !parsed.errors().is_empty() {
        return Err(format!(
            "BSL module cannot be outlined because the parser reported {} diagnostic(s)",
            parsed.errors().len()
        ));
    }
    let root = parsed.syntax_node();
    let lines = LineIndex::new(text);
    let mut methods = Vec::new();
    let mut markers = Vec::new();
    for node in root.descendants() {
        if node.kind() == SyntaxKind::PRE_REGION_DIR {
            markers.push(node);
            continue;
        }
        if let Some(procedure) = ProcedureDef::cast(node.clone()) {
            methods.push(method_outline(
                procedure.syntax(),
                &lines,
                procedure
                    .name_or_keyword()
                    .map(|token| token.text().to_string()),
                procedure.export_keyword().is_some(),
                CodeOutlineMethodKind::Procedure,
                SyntaxKind::KW_PROCEDURE,
                SyntaxKind::KW_END_PROCEDURE,
            )?);
        } else if let Some(function) = FunctionDef::cast(node) {
            methods.push(method_outline(
                function.syntax(),
                &lines,
                function
                    .name_or_keyword()
                    .map(|token| token.text().to_string()),
                function.export_keyword().is_some(),
                CodeOutlineMethodKind::Function,
                SyntaxKind::KW_FUNCTION,
                SyntaxKind::KW_END_FUNCTION,
            )?);
        }
    }
    methods.sort_by_key(|method| (method.line, method.end_line));
    Ok((methods, pair_regions(&markers, &lines)))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BslMethodFacts {
    pub is_procedure: bool,
    pub is_export: bool,
    pub parameter_count: usize,
}

/// Resolve one exact method from the parser AST. BSL identifiers are
/// case-insensitive, including Cyrillic identifiers, so Unicode lowercasing is
/// used instead of ASCII-only comparison.
pub fn exact_bsl_method_facts(
    text: &str,
    method_name: &str,
) -> Result<Option<BslMethodFacts>, String> {
    let (methods, _) = parse_module(text)?;
    let folded = method_name.to_lowercase();
    Ok(methods
        .iter()
        .find(|method| method.name.to_lowercase() == folded)
        .map(|method| BslMethodFacts {
            is_procedure: method.kind == CodeOutlineMethodKind::Procedure,
            is_export: method.is_export,
            parameter_count: method.parameters.len(),
        }))
}

/// Declared parameters of one module method: names and whether each has a
/// default value. `Знач` does not change either.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BslMethodParameters {
    pub name: String,
    pub parameters: Vec<(String, bool)>,
}

/// Parameters of every method of one module from the same parser AST as the
/// outline. A module the parser cannot read yields an error, not a guess.
pub fn bsl_method_parameters(text: &str) -> Result<Vec<BslMethodParameters>, String> {
    let (methods, _) = parse_module(text)?;
    Ok(methods
        .into_iter()
        .map(|method| BslMethodParameters {
            parameters: method
                .parameters
                .into_iter()
                .map(|parameter| (parameter.name, parameter.default_value.is_some()))
                .collect(),
            name: method.name,
        })
        .collect())
}

#[derive(Clone, Copy)]
pub enum BslProcedureRequirement {
    ExactArity(usize),
    CallableWithoutArguments,
}

impl BslProcedureRequirement {
    fn matches(self, method: &CodeOutlineMethod) -> bool {
        method.kind == CodeOutlineMethodKind::Procedure
            && method.is_export
            && match self {
                Self::ExactArity(count) => method.parameters.len() == count,
                Self::CallableWithoutArguments => method
                    .parameters
                    .iter()
                    .all(|parameter| parameter.default_value.is_some()),
            }
    }
}

pub fn exported_bsl_procedure_matches(
    text: &str,
    name: &str,
    requirement: BslProcedureRequirement,
) -> Result<bool, String> {
    let (methods, _) = parse_module(text)?;
    let folded = name.to_lowercase();
    Ok(methods
        .iter()
        .any(|method| method.name.to_lowercase() == folded && requirement.matches(method)))
}

/// Select an exported procedure through the same AST used by outline and
/// exact handler validation. Scheduled jobs may omit defaulted parameters;
/// event subscriptions retain their exact positional signature.
pub fn first_exported_bsl_procedure(
    text: &str,
    requirement: BslProcedureRequirement,
) -> Result<Option<String>, String> {
    let (methods, _) = parse_module(text)?;
    Ok(methods
        .into_iter()
        .find(|method| requirement.matches(method))
        .map(|method| method.name))
}

fn method_outline(
    syntax: &SyntaxNode,
    lines: &LineIndex,
    name: Option<String>,
    is_export: bool,
    kind: CodeOutlineMethodKind,
    start_kind: SyntaxKind,
    end_kind: SyntaxKind,
) -> Result<CodeOutlineMethod, String> {
    let name = name.ok_or_else(|| "BSL method is missing a name".to_string())?;
    let start = first_token(syntax, start_kind)
        .ok_or_else(|| format!("BSL method `{name}` is missing its opening keyword"))?;
    let end = first_token(syntax, end_kind)
        .ok_or_else(|| format!("BSL method `{name}` is missing its closing keyword"))?;
    Ok(CodeOutlineMethod {
        name,
        kind,
        parameters: parameters(syntax)?,
        is_export,
        line: lines.line_of(usize::from(start.text_range().start())),
        end_line: lines.line_of(usize::from(end.text_range().start())),
    })
}

/// Declared parameters of one method node, in order, from the parser AST.
/// The module projection reads them through the same rule as the outline.
pub(super) fn parameters(syntax: &SyntaxNode) -> Result<Vec<CodeOutlineParameter>, String> {
    let Some(list) = syntax
        .children()
        .find(|child| child.kind() == SyntaxKind::PARAM_LIST)
    else {
        return Ok(Vec::new());
    };
    list.children()
        .filter_map(Param::cast)
        .map(|parameter| {
            let name = parameter
                .name()
                .map(|token| token.text().to_string())
                .ok_or_else(|| "BSL method parameter is missing a name".to_string())?;
            let default_value = parameter
                .default_value_expr()
                .map(|expression| inline_expression(&expression));
            Ok(CodeOutlineParameter {
                name,
                by_value: parameter.val_keyword().is_some(),
                default_value,
            })
        })
        .collect()
}

/// The default value is reported as the source text of its expression node.
///
/// Joining the tokens with a separator instead would have to know where a
/// separator belongs: `-1` would become `- 1` and `Новый Структура("к", 1)`
/// would become `Новый Структура ( "к" , 1 )`. A signed numeric literal is the
/// only multi-token default the platform accepts, and it is exactly the case a
/// token join gets wrong, so the node text is both simpler and correct. It also
/// cannot corrupt whitespace inside a string literal.
fn inline_expression(expression: &SyntaxNode) -> String {
    expression.text().to_string().trim().to_string()
}

fn first_token(syntax: &SyntaxNode, kind: SyntaxKind) -> Option<SyntaxToken> {
    syntax
        .descendants_with_tokens()
        .filter_map(|element| element.into_token())
        .find(|token| token.kind() == kind)
}

/// Pairs `#Область` with `#КонецОбласти` over the flat directive markers.
///
/// The vendored analyzer parses region directives as flat leaf nodes precisely
/// because a region may overlap a control-flow block without nesting, so the
/// pairing is a stack over source order rather than syntactic nesting.
///
/// The kind of each marker is read from its own token (`PRE_REGION` /
/// `PRE_END_REGION`), never from `PreRegionDir::is_start`/`is_end`/`name`: those
/// helpers compare the node text against four literal spellings, while 1C
/// preprocessor directives are case-insensitive and the lexer accepts a space
/// after `#`. See itrous/bsl-analyzer#11 — on a real vendor configuration the
/// helpers misclassify 19 `#КонецОбласти` markers in 15 modules, each of which
/// would open a region instead of closing one.
fn pair_regions(markers: &[SyntaxNode], lines: &LineIndex) -> Vec<OutlineRegion> {
    let mut regions: Vec<OutlineRegion> = Vec::new();
    let mut open: Vec<usize> = Vec::new();
    let mut ordered: Vec<&SyntaxNode> = markers.iter().collect();
    ordered.sort_by_key(|node| node.text_range().start());
    for node in ordered {
        let Some(marker) = region_marker(node) else {
            continue;
        };
        let line = lines.line_of(usize::from(marker.text_range().start()));
        match marker.kind() {
            SyntaxKind::PRE_REGION => {
                regions.push(OutlineRegion {
                    name: region_name(node),
                    line,
                    end_line: None,
                });
                open.push(regions.len() - 1);
            }
            // An unpaired `#КонецОбласти` closes nothing rather than opening a
            // region or failing the call: the file still compiles in 1C.
            _ => {
                if let Some(index) = open.pop() {
                    regions[index].end_line = Some(line);
                }
            }
        }
    }
    regions
}

fn region_marker(node: &SyntaxNode) -> Option<SyntaxToken> {
    node.descendants_with_tokens()
        .filter_map(|element| element.into_token())
        .find(|token| {
            matches!(
                token.kind(),
                SyntaxKind::PRE_REGION | SyntaxKind::PRE_END_REGION
            )
        })
}

fn region_name(node: &SyntaxNode) -> Option<String> {
    node.descendants_with_tokens()
        .filter_map(|element| element.into_token())
        .find(|token| token.kind() == SyntaxKind::IDENT)
        .map(|token| token.text().to_string())
        .filter(|name| !name.is_empty())
}

/// One pass over the text yields every line start, so translating byte offsets
/// to line numbers never rescans the beginning of the file per method.
struct LineIndex {
    starts: Vec<usize>,
}

impl LineIndex {
    fn new(text: &str) -> Self {
        let bytes = text.as_bytes();
        let mut starts = vec![0usize];
        let mut index = 0;
        while index < bytes.len() {
            match bytes[index] {
                b'\n' => starts.push(index + 1),
                b'\r' => {
                    if bytes.get(index + 1) == Some(&b'\n') {
                        index += 1;
                    }
                    starts.push(index + 1);
                }
                _ => {}
            }
            index += 1;
        }
        Self { starts }
    }

    fn line_of(&self, offset: usize) -> usize {
        match self.starts.binary_search(&offset) {
            Ok(index) => index + 1,
            Err(index) => index,
        }
    }
}

/// Текст ошибки компиляции платформы для объявления не на своём месте.
pub const MISPLACED_VARIABLE_MESSAGE: &str =
    "Объявления переменных должны быть расположены в начале модуля, процедуры или функции";

/// Объявления `Перем`/`Var`, которые платформа не компилирует (#1460).
///
/// Модуль платформы — раздел переменных, затем методы, затем основная
/// программа; метод — свои переменные, затем операторы. Объявление уровня
/// модуля после первого метода или оператора основной программы и объявление
/// в методе после его первого оператора (в том числе внутри блока) платформа
/// отвергает, а анализатор такой модуль пропускает. Диапазоны — от нуля,
/// как у находок анализатора. Текст, который парсер не разобрал без ошибок,
/// здесь не судится: его синтаксис называет анализатор.
pub fn misplaced_variable_declarations(text: &str) -> Vec<crate::DiagnosticRange> {
    if text.len() > u32::MAX as usize {
        return Vec::new();
    }
    let parsed = parse_bsl_syntax(text);
    if !parsed.errors().is_empty() {
        return Vec::new();
    }
    let root = parsed.syntax_node();
    let is_method =
        |kind: SyntaxKind| matches!(kind, SyntaxKind::PROCEDURE_DEF | SyntaxKind::FUNCTION_DEF);
    let is_statement = |kind: SyntaxKind| {
        matches!(
            kind,
            SyntaxKind::ASSIGN_STMT
                | SyntaxKind::CALL_STMT
                | SyntaxKind::RETURN_STMT
                | SyntaxKind::IF_STMT
                | SyntaxKind::WHILE_STMT
                | SyntaxKind::FOR_STMT
                | SyntaxKind::FOR_EACH_STMT
                | SyntaxKind::TRY_STMT
                | SyntaxKind::RAISE_STMT
                | SyntaxKind::EXECUTE_STMT
                | SyntaxKind::BREAK_STMT
                | SyntaxKind::CONTINUE_STMT
                | SyntaxKind::GOTO_STMT
                | SyntaxKind::LABEL_STMT
                | SyntaxKind::ADD_HANDLER_STMT
                | SyntaxKind::REMOVE_HANDLER_STMT
        )
    };
    // Метод, которому принадлежит узел; `None` — уровень модуля.
    let owner = |node: &SyntaxNode| node.ancestors().find(|ancestor| is_method(ancestor.kind()));
    let lines = LineIndex::new(text);
    let position = |offset: usize| {
        let line = lines.line_of(offset);
        let start = lines.starts[line - 1];
        let column = text[start..offset]
            .chars()
            .filter(|character| *character != '\u{feff}')
            .count();
        (line - 1, column)
    };
    // Раздел объявлений закрывает первый метод или оператор вне методов для
    // модуля и первый оператор для метода. Ветви `#Если`/`#ИначеЕсли`/`#Иначе`
    // взаимоисключающие: каждая начинает с состояния до `#Если`, а после
    // `#КонецЕсли` раздел закрыт, если его закрыла хоть одна ветвь.
    let mut sections: std::collections::HashMap<Option<SyntaxNode>, Section> =
        std::collections::HashMap::new();
    let mut misplaced = Vec::new();
    for element in root.descendants_with_tokens() {
        let node = match element {
            bsl_syntax::NodeOrToken::Node(node) => node,
            bsl_syntax::NodeOrToken::Token(token) => {
                let Some(parent) = token.parent() else {
                    continue;
                };
                let section = sections.entry(owner(&parent)).or_default();
                match token.kind() {
                    SyntaxKind::PRE_IF => section.enter(),
                    SyntaxKind::PRE_ELSIF | SyntaxKind::PRE_ELSE => section.alternative(),
                    SyntaxKind::PRE_END_IF => section.leave(),
                    _ => {}
                }
                continue;
            }
        };
        let kind = node.kind();
        if is_method(kind) {
            sections.entry(None).or_default().closed = true;
        } else if is_statement(kind) {
            let context = node.parent().and_then(|parent| owner(&parent));
            sections.entry(context).or_default().closed = true;
        } else if kind == SyntaxKind::VAR_DEF {
            let context = node.parent().and_then(|parent| owner(&parent));
            if sections.get(&context).is_some_and(|section| section.closed) {
                let (start_line, start_column) = position(usize::from(node.text_range().start()));
                let (end_line, end_column) = position(usize::from(node.text_range().end()));
                misplaced.push(crate::DiagnosticRange {
                    start_line,
                    start_column,
                    end_line,
                    end_column,
                });
            }
        }
    }
    misplaced
}

/// Закрыт ли раздел объявлений модуля или метода, с учётом ветвей
/// препроцессора: стек хранит состояние до `#Если` и итог пройденных ветвей.
#[derive(Default)]
struct Section {
    closed: bool,
    branches: Vec<(bool, bool)>,
}

impl Section {
    fn enter(&mut self) {
        self.branches.push((self.closed, false));
    }

    fn alternative(&mut self) {
        if let Some((before, merged)) = self.branches.last_mut() {
            *merged |= self.closed;
            self.closed = *before;
        }
    }

    fn leave(&mut self) {
        if let Some((_, merged)) = self.branches.pop() {
            self.closed |= merged;
        }
    }
}

pub fn outline_regions(text: &str) -> Vec<OutlineRegion> {
    let parsed = parse_bsl_syntax(text);
    let markers = parsed
        .syntax_node()
        .descendants()
        .filter(|node| node.kind() == SyntaxKind::PRE_REGION_DIR)
        .collect::<Vec<_>>();
    pair_regions(&markers, &LineIndex::new(text))
}

#[cfg(test)]
mod tests {
    use super::{exact_bsl_method_facts, pair_regions, BslMethodFacts, LineIndex};
    use bsl_syntax::SyntaxKind;
    fn regions_of(text: &str) -> Vec<(Option<String>, usize, Option<usize>)> {
        let parsed = bsl_parser::parse(text);
        let markers: Vec<_> = parsed
            .syntax_node()
            .descendants()
            .filter(|node| node.kind() == SyntaxKind::PRE_REGION_DIR)
            .collect();
        pair_regions(&markers, &LineIndex::new(text))
            .into_iter()
            .map(|region| (region.name, region.line, region.end_line))
            .collect()
    }
    /// Строки (от единицы) объявлений, которые платформа отвергла бы.
    fn misplaced_lines(text: &str) -> Vec<usize> {
        super::misplaced_variable_declarations(text)
            .iter()
            .map(|range| range.start_line + 1)
            .collect()
    }
    #[test]
    fn procedure_selection_distinguishes_optional_arguments_from_exact_arity() {
        use super::{first_exported_bsl_procedure, BslProcedureRequirement};
        let callable = BslProcedureRequirement::CallableWithoutArguments;
        for source in [
            "Procedure Run() Export\nEndProcedure",
            "Procedure Run(Val Flag = False, Text = \"a,b=c\") Export\nEndProcedure",
            "Процедура Run(\nЗнач Профиль = Неопределено,\nРучной = Ложь) Экспорт\nКонецПроцедуры",
        ] {
            assert_eq!(
                first_exported_bsl_procedure(source, callable)
                    .unwrap()
                    .as_deref(),
                Some("Run")
            );
        }
        for source in [
            "Procedure Run(Required) Export\nEndProcedure",
            "Procedure Run(Required, Optional = False) Export\nEndProcedure",
            "Procedure Run(Optional = False)\nEndProcedure",
            "Function Run(Optional = False) Export\nReturn False;\nEndFunction",
            "// Procedure Run(Optional = False) Export\n",
        ] {
            assert_eq!(
                first_exported_bsl_procedure(source, callable).unwrap(),
                None,
                "{source}"
            );
        }
        assert!(first_exported_bsl_procedure(
            "Procedure Run(Value =) Export\nEndProcedure",
            callable
        )
        .is_err());
        let three_optional = "Procedure Run(One = 1, Two = 2, Three = 3) Export\nEndProcedure";
        assert_eq!(
            first_exported_bsl_procedure(three_optional, callable)
                .unwrap()
                .as_deref(),
            Some("Run")
        );
        assert_eq!(
            first_exported_bsl_procedure(three_optional, BslProcedureRequirement::ExactArity(2))
                .unwrap(),
            None
        );
        let two_optional = "Procedure Run(One = 1, Two = 2) Export\nEndProcedure";
        assert_eq!(
            first_exported_bsl_procedure(two_optional, BslProcedureRequirement::ExactArity(2))
                .unwrap()
                .as_deref(),
            Some("Run")
        );
    }

    #[test]
    fn region_markers_are_classified_by_token_kind_not_by_spelling() {
        // itrous/bsl-analyzer#11: PreRegionDir::is_end() is false for these
        // spellings, so a helper-based tree would open a region here instead of
        // closing one. 1C compiles all of them: directives are case-insensitive.
        for tail in [
            "#КонецОбласти",
            "#Конецобласти",
            "#КОнецОбласти",
            "#КОНЕЦОБЛАСТИ",
        ] {
            let text = format!("#Область Р\nПроцедура П()\nКонецПроцедуры\n{tail}\n");
            assert_eq!(
                regions_of(&text),
                vec![(Some("Р".to_string()), 1, Some(4))],
                "tail {tail}"
            );
        }
        for head in ["#Область Р", "#ОБЛАСТЬ Р", "# Область Р"] {
            let text = format!("{head}\n#КонецОбласти\n");
            assert_eq!(
                regions_of(&text),
                vec![(Some("Р".to_string()), 1, Some(2))],
                "head {head}"
            );
        }
    }

    #[test]
    fn english_and_nested_and_unclosed_and_in_method_regions_are_paired() {
        assert_eq!(
            regions_of("#Region Api\n#EndRegion\n"),
            vec![(Some("Api".to_string()), 1, Some(2))]
        );
        assert_eq!(
            regions_of("#Область Внешняя\n#Область Внутренняя\n#КонецОбласти\n#КонецОбласти\n"),
            vec![
                (Some("Внешняя".to_string()), 1, Some(4)),
                (Some("Внутренняя".to_string()), 2, Some(3)),
            ]
        );
        assert_eq!(
            regions_of("#Область Незакрытая\nПроцедура П()\nКонецПроцедуры\n"),
            vec![(Some("Незакрытая".to_string()), 1, None)]
        );
        // An extra end marker closes nothing instead of failing the call.
        assert_eq!(regions_of("#КонецОбласти\n"), Vec::new());
        assert_eq!(
            regions_of(
                "Процедура П()\n\t#Область Внутри\n\tА = 1;\n\t#КонецОбласти\nКонецПроцедуры\n"
            ),
            vec![(Some("Внутри".to_string()), 2, Some(4))]
        );
    }

    #[test]
    fn the_line_index_agrees_with_a_naive_scan() {
        for text in [
            "а\nб\r\nв\rг",
            "\u{feff}Процедура П()\r\nКонецПроцедуры\r\n",
            "",
            "\n\n\n",
        ] {
            let index = LineIndex::new(text);
            let bytes = text.as_bytes();
            for offset in 0..=text.len() {
                if !text.is_char_boundary(offset) {
                    continue;
                }
                // A `CRLF` is one terminator, so the offset between its two bytes
                // is not the start of anything and has no line of its own.
                if offset > 0 && bytes[offset - 1] == b'\r' && bytes.get(offset) == Some(&b'\n') {
                    continue;
                }
                let expected = text[..offset]
                    .replace("\r\n", "\n")
                    .replace('\r', "\n")
                    .matches('\n')
                    .count()
                    + 1;
                assert_eq!(index.line_of(offset), expected, "text {text:?} at {offset}");
            }
        }
    }

    #[test]
    fn exact_method_facts_come_from_ast_not_text_fragments() {
        let text = concat!(
            "// Procedure Shadow(Source) Export\n",
            "Function Handle(Source) Export\nEndFunction\n",
            "Procedure Обработать(Source, Cancel) Export\nEndProcedure\n",
        );

        assert_eq!(exact_bsl_method_facts(text, "Shadow").unwrap(), None);
        assert_eq!(
            exact_bsl_method_facts(text, "Handle").unwrap(),
            Some(BslMethodFacts {
                is_procedure: false,
                is_export: true,
                parameter_count: 1,
            })
        );
        assert_eq!(
            exact_bsl_method_facts(text, "обработать").unwrap(),
            Some(BslMethodFacts {
                is_procedure: true,
                is_export: true,
                parameter_count: 2,
            })
        );
    }

    #[test]
    fn a_module_variable_after_a_method_is_reported_where_it_stands() {
        let text = "Процедура А()\n\tХ = 1;\nКонецПроцедуры\n\nПерем Х;\n";
        let ranges = super::misplaced_variable_declarations(text);
        assert_eq!(ranges.len(), 1, "{ranges:?}");
        assert_eq!((ranges[0].start_line, ranges[0].start_column), (4, 0));
        assert_eq!(ranges[0].end_line, 4);
        assert!(ranges[0].is_non_empty());
        // Английский синтаксис и объявление между методами — то же место.
        assert_eq!(
            misplaced_lines("Procedure A()\nEndProcedure\nVar X;\nProcedure B()\nEndProcedure\n"),
            [3]
        );
        // После оператора основной программы раздел переменных тоже закрыт.
        assert_eq!(misplaced_lines("Перем А;\nА = 1;\nПерем Б;\n"), [3]);
    }

    #[test]
    fn a_local_variable_after_a_statement_is_reported_even_inside_a_block() {
        let text = "Процедура А()\n\tПерем Б;\n\tБ = 1;\n\tПерем В;\n\tЕсли Б = 1 Тогда\n\t\tПерем Г;\n\tКонецЕсли;\nКонецПроцедуры\n";
        assert_eq!(misplaced_lines(text), [4, 6]);
    }

    #[test]
    fn declarations_at_the_start_of_the_module_and_methods_are_accepted() {
        let text = "\u{feff}#Если Клиент Тогда\n// Состояние формы.\n&НаКлиенте\nПерем Кеш Экспорт;\n#КонецЕсли\nПерем Счётчик;\n\n#Область СлужебныеПроцедурыИФункции\n\nФункция Б(Параметр)\n\tПерем Локальная, Вторая;\n\tПерем Третья;\n\tЛокальная = Параметр;\n\tВозврат Локальная;\nКонецФункции\n\n#КонецОбласти\n\nСчётчик = 0;\n";
        assert_eq!(misplaced_lines(text), Vec::<usize>::new());
        // Модуль, который парсер не разобрал, здесь не судится.
        assert_eq!(
            misplaced_lines("Процедура А(\nПерем Х;\n"),
            Vec::<usize>::new()
        );
    }

    #[test]
    fn exclusive_preprocessor_branches_each_start_their_own_declarations() {
        // Ветви `#Если` взаимоисключающие: `Перем Б` в `#Иначе` стоит в начале
        // своего варианта модуля, и платформа его компилирует.
        let module = "#Если Сервер Тогда\nПерем А;\nПроцедура Х()\nКонецПроцедуры\n#ИначеЕсли Клиент Тогда\nПерем В;\n#Иначе\nПерем Б;\nПроцедура У()\nКонецПроцедуры\n#КонецЕсли\n";
        assert_eq!(misplaced_lines(module), Vec::<usize>::new());
        // После `#КонецЕсли` раздел закрыт, если его закрыла хоть одна ветвь,
        // а метод до `#Если` закрывает раздел в каждой ветви.
        assert_eq!(
            misplaced_lines("#Если Сервер Тогда\nПроцедура Х()\nКонецПроцедуры\n#Иначе\nПерем Б;\n#КонецЕсли\nПерем Г;\n"),
            [7]
        );
        assert_eq!(
            misplaced_lines(
                "Процедура Х()\nКонецПроцедуры\n#Если Сервер Тогда\n#Иначе\nПерем Б;\n#КонецЕсли\n"
            ),
            [5]
        );
        // Так же внутри метода.
        let method = "Процедура Х()\n#Если Сервер Тогда\n\tПерем А;\n\tА = 1;\n#Иначе\n\tПерем Б;\n\tБ = 1;\n#КонецЕсли\n\tПерем В;\nКонецПроцедуры\n";
        assert_eq!(misplaced_lines(method), [9]);
    }

    /// Справка платформы («Структура программного модуля»): раздел
    /// определения переменных длится до первого оператора `Процедура` или
    /// `Функция`; исключения для директив компиляции в ней нет. Клиентская
    /// переменная после серверной процедуры формы стоит не на месте.
    #[test]
    fn a_compilation_directive_does_not_reopen_the_declaration_section() {
        let form = "&НаСервере\nПроцедура Х()\nКонецПроцедуры\n\n&НаКлиенте\nПерем Б;\n";
        assert_eq!(misplaced_lines(form).len(), 1);
    }
}
