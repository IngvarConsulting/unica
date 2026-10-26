//! Symbols a module's own text proves: what name stands under a position, where
//! a module-local name is declared and used, and which calls a method makes.
//!
//! These answers need no index and no saved file, so they hold for an unsaved
//! buffer as well. A name the module does not declare is left to the caller.

use bsl_syntax::{SyntaxKind, SyntaxNode, SyntaxToken, TextSize, TokenAtOffset};
use std::ops::Range;

use crate::{byte_range, ParsedModule};

/// Byte offsets and (line, UTF-16 column) positions of one text, both 0-based.
pub struct LineIndex<'a> {
    text: &'a str,
    line_starts: Vec<usize>,
}

impl<'a> LineIndex<'a> {
    pub fn new(text: &'a str) -> Self {
        let line_starts = std::iter::once(0)
            .chain(text.match_indices('\n').map(|(index, _)| index + 1))
            .collect();
        Self { text, line_starts }
    }

    /// The byte offset of a position; a column past the end of its line, or a
    /// line past the end of the text, is clamped to that end.
    pub fn offset(&self, line: usize, column: usize) -> usize {
        let Some(&start) = self.line_starts.get(line) else {
            return self.text.len();
        };
        let end = self
            .line_starts
            .get(line + 1)
            .map_or(self.text.len(), |next| next - 1);
        let mut units = 0;
        for (index, character) in self.text[start..end].char_indices() {
            if units >= column {
                return start + index;
            }
            units += character.len_utf16();
        }
        end
    }

    /// The (line, UTF-16 column) of a byte offset that lies on a character
    /// boundary.
    pub fn position(&self, offset: usize) -> (usize, usize) {
        let line = self.line_starts.partition_point(|start| *start <= offset) - 1;
        let start = self.line_starts[line];
        let column = self.text[start..offset.min(self.text.len())]
            .chars()
            .map(char::len_utf16)
            .sum();
        (line, column)
    }
}

/// A dotted name in code: `Справочники.Валюты.ПустаяСсылка` with the segment
/// under the position. `is_call` tells whether the whole path is called.
/// A name inside a path that is not plain names alone (`Запрос.Выполнить().
/// Выбрать`, `Массив[0].Поле`) is reported as its own segment with `plain`
/// unset: the module's text cannot tell what it names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamePath {
    pub segments: Vec<(String, Range<usize>)>,
    pub cursor: usize,
    pub is_call: bool,
    pub plain: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalSymbolKind {
    Method,
    Parameter,
    Variable,
    ModuleVariable,
}

/// A name the module itself declares, with the range of its declaring name
/// and, for a parameter or local variable, the method that owns it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalSymbol {
    pub name: String,
    pub kind: LocalSymbolKind,
    pub range: Range<usize>,
    pub method: Option<Range<usize>>,
}

/// A method of the module: its name, the range of that name and the whole
/// declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalMethod {
    pub name: String,
    pub name_range: Range<usize>,
    pub range: Range<usize>,
    pub is_export: bool,
}

/// A call written in a method: the called path and the range of the last name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalCall {
    pub path: Vec<String>,
    pub range: Range<usize>,
}

/// BSL names are case-insensitive.
pub fn same_name(left: &str, right: &str) -> bool {
    left.chars()
        .flat_map(char::to_lowercase)
        .eq(right.chars().flat_map(char::to_lowercase))
}

/// A token that names something: an identifier, or a keyword spelled as a
/// member after a dot (`Запрос.Выполнить`).
fn is_name(token: &SyntaxToken) -> bool {
    token.kind() == SyntaxKind::IDENT
        || (token.kind().is_keyword()
            && token
                .parent()
                .is_some_and(|parent| parent.kind() == SyntaxKind::FIELD_EXPR))
}

fn is_method(node: &SyntaxNode) -> bool {
    matches!(
        node.kind(),
        SyntaxKind::PROCEDURE_DEF | SyntaxKind::FUNCTION_DEF
    )
}

/// The methods of a module: a method may sit inside a region or a
/// preprocessor branch, never inside another method.
fn module_methods(root: &SyntaxNode) -> impl Iterator<Item = SyntaxNode> {
    root.descendants().filter(is_method)
}

/// The names a module-level `Перем` declares, outside every method.
fn module_variables(root: &SyntaxNode) -> impl Iterator<Item = SyntaxToken> {
    root.descendants()
        .filter(|node| node.kind() == SyntaxKind::VAR_DEF)
        .filter(|node| !node.ancestors().any(|ancestor| is_method(&ancestor)))
        .flat_map(|node| {
            node.children_with_tokens()
                .filter_map(|element| element.into_token())
                .filter(|token| token.kind() == SyntaxKind::IDENT)
                .collect::<Vec<_>>()
        })
}

/// The direct IDENT name token of a declaration node.
fn declared_name(node: &SyntaxNode) -> Option<SyntaxToken> {
    node.children_with_tokens()
        .filter_map(|element| element.into_token())
        .find(|token| token.kind() == SyntaxKind::IDENT)
}

/// The plain segments of a dotted path node, or `None` when the path contains
/// a call, an index or anything but names.
fn flatten(node: &SyntaxNode) -> Option<Vec<SyntaxToken>> {
    match node.kind() {
        SyntaxKind::IDENT => declared_name(node).map(|token| vec![token]),
        SyntaxKind::FIELD_EXPR => {
            let mut segments = flatten(&node.first_child()?)?;
            segments.push(
                node.children_with_tokens()
                    .filter_map(|element| element.into_token())
                    .filter(is_name)
                    .last()?,
            );
            Some(segments)
        }
        _ => None,
    }
}

/// The topmost dotted path node that `token` belongs to.
fn path_root(token: &SyntaxToken) -> Option<SyntaxNode> {
    let mut node = token.parent()?;
    if node.kind() == SyntaxKind::IDENT {
        if let Some(parent) = node
            .parent()
            .filter(|parent| parent.kind() == SyntaxKind::FIELD_EXPR)
        {
            node = parent;
        }
    }
    if !matches!(node.kind(), SyntaxKind::IDENT | SyntaxKind::FIELD_EXPR) {
        return None;
    }
    while let Some(parent) = node
        .parent()
        .filter(|parent| parent.kind() == SyntaxKind::FIELD_EXPR)
    {
        node = parent;
    }
    Some(node)
}

impl ParsedModule<'_> {
    fn ident_at(&self, offset: usize) -> Option<SyntaxToken> {
        let offset = TextSize::try_from(offset).ok()?;
        let root = self.parsed.syntax_node();
        if offset > root.text_range().end() {
            return None;
        }
        let pick = is_name;
        match root.token_at_offset(offset) {
            TokenAtOffset::None => None,
            TokenAtOffset::Single(token) => pick(&token).then_some(token),
            TokenAtOffset::Between(left, right) => {
                if pick(&right) {
                    Some(right)
                } else {
                    pick(&left).then_some(left)
                }
            }
        }
    }

    /// The dotted name under `offset`, if the position is on a name.
    pub fn name_path_at(&self, offset: usize) -> Option<NamePath> {
        let token = self.ident_at(offset)?;
        // A declaration's own name is a path of one segment.
        let parent = token.parent()?;
        if matches!(
            parent.kind(),
            SyntaxKind::PROCEDURE_DEF
                | SyntaxKind::FUNCTION_DEF
                | SyntaxKind::VAR_DEF
                | SyntaxKind::PARAM
        ) {
            return Some(NamePath {
                segments: vec![(token.text().to_string(), byte_range(token.text_range()))],
                cursor: 0,
                is_call: false,
                plain: true,
            });
        }
        let lone = |is_call| NamePath {
            segments: vec![(token.text().to_string(), byte_range(token.text_range()))],
            cursor: 0,
            is_call,
            plain: false,
        };
        let Some(root) = path_root(&token) else {
            return Some(lone(false));
        };
        let Some(segments) = flatten(&root) else {
            let is_call = token
                .parent()
                .and_then(|parent| parent.parent())
                .is_some_and(|grand| grand.kind() == SyntaxKind::CALL_EXPR);
            return Some(lone(is_call));
        };
        let cursor = segments.iter().position(|segment| *segment == token)?;
        let is_call = root.parent().is_some_and(|parent| {
            parent.kind() == SyntaxKind::CALL_EXPR && parent.first_child().as_ref() == Some(&root)
        });
        Some(NamePath {
            segments: segments
                .iter()
                .map(|segment| (segment.text().to_string(), byte_range(segment.text_range())))
                .collect(),
            cursor,
            is_call,
            plain: true,
        })
    }

    /// Every method of the module in text order.
    pub fn local_methods(&self) -> Vec<LocalMethod> {
        module_methods(&self.parsed.syntax_node())
            .filter_map(|node| {
                let name = declared_name(&node)?;
                Some(LocalMethod {
                    name: name.text().to_string(),
                    name_range: byte_range(name.text_range()),
                    range: byte_range(node.text_range()),
                    is_export: node
                        .children_with_tokens()
                        .any(|element| element.kind() == SyntaxKind::KW_EXPORT),
                })
            })
            .collect()
    }

    /// The method whose declaration contains `offset`.
    pub fn method_at(&self, offset: usize) -> Option<LocalMethod> {
        self.local_methods()
            .into_iter()
            .find(|method| method.range.contains(&offset))
    }

    /// Where the module declares `name` as seen from `offset`: a parameter or
    /// a variable of the enclosing method first, then a module variable, then
    /// a method of the module; a name first assigned in the enclosing method
    /// is that method's implicit variable. `is_call` skips the variables.
    pub fn local_definition(
        &self,
        name: &str,
        offset: usize,
        is_call: bool,
    ) -> Option<LocalSymbol> {
        let root = self.parsed.syntax_node();
        let method =
            module_methods(&root).find(|node| byte_range(node.text_range()).contains(&offset));
        let symbol = |token: SyntaxToken, kind, method: Option<&SyntaxNode>| LocalSymbol {
            name: token.text().to_string(),
            kind,
            range: byte_range(token.text_range()),
            method: method.map(|node| byte_range(node.text_range())),
        };
        if !is_call {
            if let Some(method) = &method {
                let params = method
                    .descendants()
                    .filter(|node| node.kind() == SyntaxKind::PARAM)
                    .filter_map(|node| declared_name(&node));
                let vars = method
                    .descendants()
                    .filter(|node| node.kind() == SyntaxKind::VAR_DEF)
                    .flat_map(|node| {
                        node.children_with_tokens()
                            .filter_map(|element| element.into_token())
                            .filter(|token| token.kind() == SyntaxKind::IDENT)
                            .collect::<Vec<_>>()
                    });
                if let Some(token) = params
                    .into_iter()
                    .find(|token| same_name(token.text(), name))
                {
                    return Some(symbol(token, LocalSymbolKind::Parameter, Some(method)));
                }
                if let Some(token) = vars.into_iter().find(|token| same_name(token.text(), name)) {
                    return Some(symbol(token, LocalSymbolKind::Variable, Some(method)));
                }
            }
            if let Some(token) = module_variables(&root).find(|token| same_name(token.text(), name))
            {
                return Some(symbol(token, LocalSymbolKind::ModuleVariable, None));
            }
        }
        if let Some(found) = module_methods(&root)
            .filter_map(|node| declared_name(&node))
            .find(|token| same_name(token.text(), name))
        {
            return Some(symbol(found, LocalSymbolKind::Method, None));
        }
        if is_call {
            return None;
        }
        let method = method?;
        // A loop variable is declared by its `Для` or `Для Каждого`.
        if let Some(token) = method
            .descendants()
            .filter(|node| {
                matches!(
                    node.kind(),
                    SyntaxKind::FOR_STMT | SyntaxKind::FOR_EACH_STMT
                )
            })
            .filter_map(|node| declared_name(&node))
            .find(|token| same_name(token.text(), name))
        {
            return Some(symbol(token, LocalSymbolKind::Variable, Some(&method)));
        }
        let assigned = method
            .descendants()
            .filter(|node| node.kind() == SyntaxKind::ASSIGN_STMT)
            .filter_map(|node| node.first_child())
            .filter(|target| target.kind() == SyntaxKind::IDENT)
            .filter_map(|target| declared_name(&target))
            .find(|token| same_name(token.text(), name))?;
        Some(symbol(assigned, LocalSymbolKind::Variable, Some(&method)))
    }

    /// Every place in the module that names `symbol` as a plain name: its
    /// declaration included. A parameter or variable is looked for inside its
    /// method; a module variable and a method across the module, except in a
    /// method that declares the same name as its own parameter or variable.
    pub fn local_references(&self, symbol: &LocalSymbol) -> Vec<Range<usize>> {
        let root = self.parsed.syntax_node();
        // Methods that shadow a module variable, found once for the module.
        let shadowing: Vec<Range<usize>> = if symbol.kind == LocalSymbolKind::ModuleVariable {
            module_methods(&root)
                .filter(|method| {
                    method
                        .descendants()
                        .filter(|node| {
                            matches!(node.kind(), SyntaxKind::PARAM | SyntaxKind::VAR_DEF)
                        })
                        .flat_map(|node| {
                            node.children_with_tokens()
                                .filter_map(|element| element.into_token())
                                .filter(|token| token.kind() == SyntaxKind::IDENT)
                                .collect::<Vec<_>>()
                        })
                        .any(|token| same_name(token.text(), &symbol.name))
                })
                .map(|method| byte_range(method.text_range()))
                .collect()
        } else {
            Vec::new()
        };
        root.descendants_with_tokens()
            .filter_map(|element| element.into_token())
            .filter(|token| {
                token.kind() == SyntaxKind::IDENT && same_name(token.text(), &symbol.name)
            })
            .map(|token| byte_range(token.text_range()))
            .filter(|range| {
                if *range == symbol.range {
                    return true;
                }
                if let Some(method) = &symbol.method {
                    if !method.contains(&range.start) {
                        return false;
                    }
                }
                // Only the head of a dotted path is a plain name.
                let Some(path) = self.name_path_at(range.start) else {
                    return false;
                };
                if path.cursor != 0 || !path.plain {
                    return false;
                }
                let called = path.is_call && path.segments.len() == 1;
                match symbol.kind {
                    LocalSymbolKind::Method => called,
                    LocalSymbolKind::Parameter | LocalSymbolKind::Variable => !called,
                    LocalSymbolKind::ModuleVariable => {
                        !called && !shadowing.iter().any(|method| method.contains(&range.start))
                    }
                }
            })
            .collect()
    }

    /// Every call written in the module, in text order.
    pub fn calls(&self) -> Vec<LocalCall> {
        self.parsed
            .syntax_node()
            .descendants()
            .filter(|node| node.kind() == SyntaxKind::CALL_EXPR)
            .filter_map(|node| {
                let segments = flatten(&node.first_child()?)?;
                let last = segments.last()?;
                Some(LocalCall {
                    path: segments
                        .iter()
                        .map(|segment| segment.text().to_string())
                        .collect(),
                    range: byte_range(last.text_range()),
                })
            })
            .collect()
    }

    /// The calls written inside `method`, in text order.
    pub fn calls_in(&self, method: &LocalMethod) -> Vec<LocalCall> {
        self.calls()
            .into_iter()
            .filter(|call| method.range.contains(&call.range.start))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::{LineIndex, LocalSymbolKind};
    use crate::parse;

    const MODULE: &str = "Перем МодульнаяПеременная;\n\nПроцедура Вызов(Параметр) Экспорт\n\tПерем Локальная;\n\tРезультат = Ф(Параметр);\n\tСсылка = Справочники.Валюты.ПустаяСсылка();\n\tОбщийМодуль.Сделать(Ссылка.Код, МодульнаяПеременная);\n\tРезультат = Результат + 1;\nКонецПроцедуры\n\nФункция Ф(А)\n\tВозврат А;\nКонецФункции\n";

    fn at(text: &str, needle: &str) -> usize {
        text.find(needle).unwrap_or_else(|| panic!("{needle}"))
    }

    #[test]
    fn a_position_names_the_dotted_path_and_its_segment() {
        let module = parse(MODULE);
        let path = module.name_path_at(at(MODULE, "Валюты") + 2).unwrap();
        let names: Vec<_> = path
            .segments
            .iter()
            .map(|(name, _)| name.as_str())
            .collect();
        assert_eq!(names, ["Справочники", "Валюты", "ПустаяСсылка"]);
        assert_eq!(path.cursor, 1);
        assert!(path.is_call);
        let path = module.name_path_at(at(MODULE, "Ф(Параметр)")).unwrap();
        assert_eq!(path.segments.len(), 1);
        assert!(path.is_call);
        assert!(module.name_path_at(at(MODULE, "КонецПроцедуры")).is_none());
    }

    #[test]
    fn local_names_resolve_to_their_declarations_and_uses() {
        let module = parse(MODULE);
        let parameter = module
            .local_definition("параметр", at(MODULE, "Ф(Параметр)") + 2, false)
            .unwrap();
        assert_eq!(parameter.kind, LocalSymbolKind::Parameter);
        assert_eq!(&MODULE[parameter.range.clone()], "Параметр");
        assert_eq!(module.local_references(&parameter).len(), 2);

        let implicit = module
            .local_definition("Результат", at(MODULE, "Результат + 1"), false)
            .unwrap();
        assert_eq!(implicit.kind, LocalSymbolKind::Variable);
        assert_eq!(implicit.range.start, at(MODULE, "Результат = Ф"));
        assert_eq!(module.local_references(&implicit).len(), 3);

        let method = module.local_definition("Ф", 0, true).unwrap();
        assert_eq!(method.kind, LocalSymbolKind::Method);
        assert_eq!(method.range.start, at(MODULE, "Ф(А)"));
        assert_eq!(module.local_references(&method).len(), 2);

        let module_variable = module
            .local_definition(
                "МодульнаяПеременная",
                at(MODULE, "МодульнаяПеременная)"),
                false,
            )
            .unwrap();
        assert_eq!(module_variable.kind, LocalSymbolKind::ModuleVariable);
        assert!(module.local_definition("Сделать", 0, true).is_none());
    }

    #[test]
    fn a_method_lists_its_calls() {
        let module = parse(MODULE);
        let method = module.method_at(at(MODULE, "Результат = Ф")).unwrap();
        assert_eq!(method.name, "Вызов");
        let calls: Vec<_> = module
            .calls_in(&method)
            .into_iter()
            .map(|call| call.path.join("."))
            .collect();
        assert_eq!(
            calls,
            [
                "Ф",
                "Справочники.Валюты.ПустаяСсылка",
                "ОбщийМодуль.Сделать"
            ]
        );
    }

    #[test]
    fn a_name_inside_a_call_chain_is_not_plain_and_loop_variables_are_declared() {
        let text = "Процедура А(Таблица)\n\tДля Каждого Строка Из Таблица Цикл\n\t\tСтрока.Поле = 1;\n\tКонецЦикла;\n\tЗапрос.Выполнить().Выбрать();\nКонецПроцедуры\n";
        let module = parse(text);
        let chained = module.name_path_at(at(text, "Выбрать")).unwrap();
        assert!(!chained.plain);
        assert!(chained.is_call);
        let member = module.name_path_at(at(text, "Выполнить")).unwrap();
        assert!(member.plain, "a keyword member of a plain path is a name");
        let variable = module
            .local_definition("Строка", at(text, "Строка.Поле"), false)
            .unwrap();
        assert_eq!(variable.kind, LocalSymbolKind::Variable);
        assert_eq!(variable.range.start, at(text, "Строка Из"));
        assert!(!module.local_methods()[0].is_export);
    }

    /// A real module keeps its methods and variables inside regions.
    #[test]
    fn methods_and_variables_inside_regions_are_the_module_s_own() {
        let text = "#Область Переменные\nПерем Кэш;\n#КонецОбласти\n\n#Область ПрограммныйИнтерфейс\nПроцедура А() Экспорт\n\tБ(Кэш);\nКонецПроцедуры\n#КонецОбласти\n\n#Область Служебные\nПроцедура Б(Значение)\nКонецПроцедуры\n#КонецОбласти\n";
        let module = parse(text);
        let names: Vec<_> = module
            .local_methods()
            .into_iter()
            .map(|method| method.name)
            .collect();
        assert_eq!(names, ["А", "Б"]);
        let call = at(text, "Б(Кэш)");
        let method = module.local_definition("Б", call, true).unwrap();
        assert_eq!(method.range.start, at(text, "Б(Значение)"));
        let variable = module
            .local_definition("Кэш", at(text, "Кэш);"), false)
            .unwrap();
        assert_eq!(variable.kind, LocalSymbolKind::ModuleVariable);
        assert_eq!(module.local_references(&variable).len(), 2);
    }

    #[test]
    fn positions_count_utf16_units_and_round_trip() {
        let text = "А😀Б\nВ";
        let index = LineIndex::new(text);
        let b = text.find('Б').unwrap();
        assert_eq!(index.position(b), (0, 3));
        assert_eq!(index.offset(0, 3), b);
        assert_eq!(index.offset(1, 0), text.find('В').unwrap());
        assert_eq!(index.offset(0, 99), text.find('\n').unwrap());
        assert_eq!(index.offset(9, 0), text.len());
    }
}
