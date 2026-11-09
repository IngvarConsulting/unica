//! Syntax facts an editor asks for, in byte ranges of the module text:
//! the symbol tree, folding blocks, classified tokens and the call around a
//! cursor. Names compare case-insensitively, as the language does.

use crate::{byte_range, ParsedModule};
use bsl_syntax::{SyntaxKind, SyntaxNode, SyntaxToken};
use std::collections::BTreeSet;
use std::ops::Range;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorSymbolKind {
    Region,
    Procedure,
    Function,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorParameter {
    pub name: String,
    pub range: Range<usize>,
    pub by_value: bool,
    pub default_value: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorSymbol {
    pub name: String,
    pub kind: EditorSymbolKind,
    /// The whole symbol: a method with its annotations, a region from its
    /// start marker to its end marker (or the end of the text when unclosed).
    pub range: Range<usize>,
    /// The name, or the start marker of an unnamed region.
    pub selection_range: Range<usize>,
    pub is_export: bool,
    pub parameters: Vec<EditorParameter>,
    /// The `//` comment lines right above a method, without the slashes.
    pub description: Option<String>,
    pub children: Vec<EditorSymbol>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FoldKind {
    Region,
    /// A method, a statement block or a preprocessor condition, closed by
    /// its own end keyword.
    Block,
    Comment,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoldBlock {
    pub range: Range<usize>,
    pub kind: FoldKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TokenClass {
    Keyword,
    Method,
    Parameter,
    Variable,
    Property,
    Type,
    /// Preprocessor instructions and region markers.
    Directive,
    /// Compilation directives and annotations (`&НаСервере`, `&Перед`).
    Annotation,
    String,
    Number,
    Comment,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassifiedToken {
    pub range: Range<usize>,
    pub class: TokenClass,
    pub declaration: bool,
}

/// The call whose argument list holds a cursor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallSite {
    /// `ОбщМод` of `ОбщМод.Метод(...)`; `None` for a call of a plain name.
    pub qualifier: Option<String>,
    pub name: String,
    pub name_range: Range<usize>,
    /// Zero-based: the commas of the argument list before the cursor.
    pub active_parameter: usize,
}

impl ParsedModule<'_> {
    /// Regions and methods as a tree: a method or region sits in the
    /// innermost region that holds it.
    pub fn editor_symbols(&self) -> Vec<EditorSymbol> {
        let mut flat = self.region_symbols();
        flat.extend(self.method_symbols());
        // Outer before inner: by start, then by the longer range.
        flat.sort_by(|a, b| {
            a.range
                .start
                .cmp(&b.range.start)
                .then(b.range.end.cmp(&a.range.end))
        });
        let mut roots = Vec::<EditorSymbol>::new();
        let mut open = Vec::<EditorSymbol>::new();
        for symbol in flat {
            while let Some(top) = open.last() {
                if symbol.range.start >= top.range.start && symbol.range.end <= top.range.end {
                    break;
                }
                let done = open.pop().expect("a symbol is open");
                attach(&mut open, &mut roots, done);
            }
            if symbol.kind == EditorSymbolKind::Region {
                open.push(symbol);
            } else {
                attach(&mut open, &mut roots, symbol);
            }
        }
        while let Some(done) = open.pop() {
            attach(&mut open, &mut roots, done);
        }
        roots
    }

    fn region_symbols(&self) -> Vec<EditorSymbol> {
        let mut starts = Vec::<(Range<usize>, Option<String>, Option<Range<usize>>)>::new();
        let mut regions = Vec::new();
        for marker in self
            .parsed
            .syntax_node()
            .descendants()
            .filter(|node| node.kind() == SyntaxKind::PRE_REGION_DIR)
        {
            let range = byte_range(marker.text_range());
            let is_end = marker
                .children_with_tokens()
                .filter_map(|element| element.into_token())
                .any(|token| token.kind() == SyntaxKind::PRE_END_REGION);
            if is_end {
                if let Some((start, name, name_range)) = starts.pop() {
                    regions.push(region(start.start..range.end, start, name, name_range));
                }
                continue;
            }
            let name = marker
                .children_with_tokens()
                .filter_map(|element| element.into_token())
                .find(|token| token.kind() == SyntaxKind::IDENT);
            starts.push((
                range,
                name.as_ref().map(|token| token.text().to_owned()),
                name.as_ref().map(|token| byte_range(token.text_range())),
            ));
        }
        // An unclosed region holds the rest of the module.
        let end = self.source.len();
        regions.extend(
            starts
                .into_iter()
                .map(|(start, name, name_range)| region(start.start..end, start, name, name_range)),
        );
        regions
    }

    fn method_symbols(&self) -> Vec<EditorSymbol> {
        self.method_nodes()
            .map(|node| {
                let name = direct_token(&node, SyntaxKind::IDENT);
                let header = direct_token(&node, SyntaxKind::KW_PROCEDURE)
                    .or_else(|| direct_token(&node, SyntaxKind::KW_FUNCTION));
                let range = byte_range(node.text_range());
                EditorSymbol {
                    name: name
                        .as_ref()
                        .map(|token| token.text().to_owned())
                        .unwrap_or_default(),
                    kind: if node.kind() == SyntaxKind::FUNCTION_DEF {
                        EditorSymbolKind::Function
                    } else {
                        EditorSymbolKind::Procedure
                    },
                    selection_range: name
                        .or(header)
                        .map(|token| byte_range(token.text_range()))
                        .unwrap_or(range.start..range.start),
                    is_export: direct_token(&node, SyntaxKind::KW_EXPORT).is_some(),
                    parameters: self.parameters(&node),
                    description: description_above(self.source, range.start),
                    range,
                    children: Vec::new(),
                }
            })
            .collect()
    }

    fn method_nodes(&self) -> impl Iterator<Item = SyntaxNode> {
        self.parsed.syntax_node().descendants().filter(|node| {
            matches!(
                node.kind(),
                SyntaxKind::PROCEDURE_DEF | SyntaxKind::FUNCTION_DEF
            )
        })
    }

    fn parameters(&self, method: &SyntaxNode) -> Vec<EditorParameter> {
        let Some(list) = method
            .children()
            .find(|child| child.kind() == SyntaxKind::PARAM_LIST)
        else {
            return Vec::new();
        };
        list.children()
            .filter(|child| child.kind() == SyntaxKind::PARAM)
            .filter_map(|param| {
                let name = direct_token(&param, SyntaxKind::IDENT)?;
                let default_value = param
                    .children()
                    .find(|child| child.kind() == SyntaxKind::EXPR)
                    .map(|value| {
                        self.source[byte_range(value.text_range())]
                            .trim()
                            .to_owned()
                    });
                Some(EditorParameter {
                    name: name.text().to_owned(),
                    range: byte_range(name.text_range()),
                    by_value: direct_token(&param, SyntaxKind::KW_VAL).is_some(),
                    default_value,
                })
            })
            .collect()
    }

    /// Blocks an editor can fold: regions, methods, statement blocks,
    /// preprocessor conditions and runs of two or more comment lines.
    pub fn fold_blocks(&self) -> Vec<FoldBlock> {
        let mut blocks = self
            .region_symbols()
            .into_iter()
            .map(|region| FoldBlock {
                range: region.range,
                kind: FoldKind::Region,
            })
            .collect::<Vec<_>>();
        for node in self.parsed.syntax_node().descendants() {
            let block = match node.kind() {
                SyntaxKind::PROCEDURE_DEF | SyntaxKind::FUNCTION_DEF => {
                    // From the header, so annotations stay visible.
                    let start = direct_token(&node, SyntaxKind::KW_PROCEDURE)
                        .or_else(|| direct_token(&node, SyntaxKind::KW_FUNCTION))
                        .map(|token| usize::from(token.text_range().start()));
                    start.map(|start| start..usize::from(node.text_range().end()))
                }
                SyntaxKind::IF_STMT
                | SyntaxKind::WHILE_STMT
                | SyntaxKind::FOR_STMT
                | SyntaxKind::FOR_EACH_STMT
                | SyntaxKind::TRY_STMT
                | SyntaxKind::PRE_IF_DIR => Some(byte_range(node.text_range())),
                _ => None,
            };
            if let Some(range) = block {
                blocks.push(FoldBlock {
                    range,
                    kind: FoldKind::Block,
                });
            }
        }
        blocks.extend(comment_runs(self.source, &self.parsed.syntax_node()));
        blocks.sort_by_key(|block| (block.range.start, std::cmp::Reverse(block.range.end)));
        blocks
    }

    /// Every token an editor colours, in text order. Names resolve inside
    /// their method: parameters, then local and module variables. A name
    /// none of them declares (a global context member) is left uncoloured.
    pub fn classified_tokens(&self) -> Vec<ClassifiedToken> {
        let root = self.parsed.syntax_node();
        let module_variables = declared_variables(&root, false);
        let mut scopes = self
            .method_nodes()
            .map(|method| {
                let parameters = self
                    .parameters(&method)
                    .into_iter()
                    .map(|parameter| parameter.name.to_lowercase())
                    .collect::<BTreeSet<_>>();
                let locals = declared_variables(&method, true);
                (byte_range(method.text_range()), parameters, locals)
            })
            .collect::<Vec<_>>();
        scopes.sort_by_key(|(range, _, _)| range.start);

        let mut tokens = Vec::new();
        for token in root
            .descendants_with_tokens()
            .filter_map(|element| element.into_token())
        {
            let kind = token.kind();
            let range = byte_range(token.text_range());
            let class = if kind == SyntaxKind::COMMENT {
                Some((TokenClass::Comment, false))
            } else if kind.is_string_literal() || kind == SyntaxKind::DATE {
                Some((TokenClass::String, false))
            } else if kind.is_number_literal() {
                Some((TokenClass::Number, false))
            } else if kind.is_preprocessor() {
                Some((TokenClass::Directive, false))
            } else if kind.is_annotation() {
                Some((TokenClass::Annotation, false))
            } else if kind.is_keyword() {
                Some((TokenClass::Keyword, false))
            } else if kind == SyntaxKind::IDENT {
                // Methods do not nest: the last one starting before the
                // token is the only one that can hold it.
                let before = scopes.partition_point(|(method, _, _)| method.start <= range.start);
                let scope = before
                    .checked_sub(1)
                    .map(|index| &scopes[index])
                    .filter(|(method, _, _)| range.end <= method.end);
                classify_name(&token, scope, &module_variables)
            } else {
                None
            };
            if let Some((class, declaration)) = class {
                tokens.push(ClassifiedToken {
                    range,
                    class,
                    declaration,
                });
            }
        }
        tokens
    }

    /// The call whose argument list holds `offset`: after its `(` and not
    /// after its `)`. The innermost call wins.
    pub fn call_at(&self, offset: usize) -> Option<CallSite> {
        self.parsed
            .syntax_node()
            .descendants()
            .filter(|node| node.kind() == SyntaxKind::ARG_LIST)
            .filter(|list| {
                let open = direct_token(list, SyntaxKind::L_PAREN)
                    .map(|token| usize::from(token.text_range().end()));
                // An unclosed list being typed reaches up to the next
                // token that is not blank.
                let close = direct_token(list, SyntaxKind::R_PAREN)
                    .map(|token| usize::from(token.text_range().start()))
                    .unwrap_or_else(|| unclosed_end(list));
                open.is_some_and(|open| open <= offset && offset <= close)
            })
            .min_by_key(|list| list.text_range().len())
            .and_then(|list| {
                let call = list.parent()?;
                if call.kind() != SyntaxKind::CALL_EXPR {
                    return None;
                }
                let active_parameter = list
                    .children_with_tokens()
                    .filter_map(|element| element.into_token())
                    .filter(|token| {
                        token.kind() == SyntaxKind::COMMA
                            && usize::from(token.text_range().start()) < offset
                    })
                    .count();
                let (qualifier, name) = callee(&call.first_child()?)?;
                Some(CallSite {
                    qualifier,
                    name: name.text().to_owned(),
                    name_range: byte_range(name.text_range()),
                    active_parameter,
                })
            })
    }
}

/// Where a completion is asked: the word being typed and what it follows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionSite {
    /// The name the cursor stands in, the part after the cursor included:
    /// completion replaces the whole word.
    pub prefix_range: Range<usize>,
    /// The names before the dot of a member access (`Справочники.Валюты.`),
    /// `None` for a plain name. An access whose receiver is not a chain of
    /// names (`Ф().`, `М[0].`) has an empty path.
    pub receiver: Option<Vec<String>>,
}

/// Names the code at a point can use, as declared.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VisibleNames {
    pub parameters: Vec<String>,
    pub locals: Vec<String>,
    pub module_variables: Vec<String>,
}

fn is_name_char(character: char) -> bool {
    character.is_alphanumeric() || character == '_'
}

impl ParsedModule<'_> {
    /// The completion site at `offset`; `None` inside a comment or a
    /// string, where code is not completed.
    pub fn completion_site(&self, offset: usize) -> Option<CompletionSite> {
        let offset = offset.min(self.source.len());
        if !self.source.is_char_boundary(offset) || self.in_text_token(offset) {
            return None;
        }
        let before = &self.source[..offset];
        let start = before
            .char_indices()
            .rev()
            .take_while(|(_, character)| is_name_char(*character))
            .last()
            .map_or(offset, |(at, _)| at);
        let end = offset
            + self.source[offset..]
                .char_indices()
                .find(|(_, character)| !is_name_char(*character))
                .map_or(self.source.len() - offset, |(at, _)| at);
        let receiver = before[..start]
            .strip_suffix('.')
            .map(|chain| receiver_path(chain).unwrap_or_default());
        Some(CompletionSite {
            prefix_range: start..end,
            receiver,
        })
    }

    fn in_text_token(&self, offset: usize) -> bool {
        let root = self.parsed.syntax_node();
        let Ok(at) = u32::try_from(offset) else {
            return false;
        };
        root.token_at_offset(at.into()).any(|token| {
            let range = byte_range(token.text_range());
            let inside = range.start < offset && offset < range.end
                // A line comment and an unterminated string run to the
                // end of their line, cursor included.
                || (offset == range.end
                    && matches!(
                        token.kind(),
                        SyntaxKind::COMMENT | SyntaxKind::STRING_START | SyntaxKind::STRING_PART
                    ));
            inside && (token.kind() == SyntaxKind::COMMENT || token.kind().is_string_literal())
        })
    }

    /// Parameters, local and module variables visible at `offset`, in
    /// declaration order and spelled as declared.
    pub fn visible_names(&self, offset: usize) -> VisibleNames {
        let root = self.parsed.syntax_node();
        let mut names = VisibleNames {
            module_variables: declared_variable_spellings(&root, false),
            ..VisibleNames::default()
        };
        if let Some(method) = self.method_nodes().find(|method| {
            let range = byte_range(method.text_range());
            range.start <= offset && offset <= range.end
        }) {
            names.parameters = self
                .parameters(&method)
                .into_iter()
                .map(|parameter| parameter.name)
                .collect();
            names.locals = declared_variable_spellings(&method, true);
        }
        names
    }

    /// The collection the last `Для Каждого <variable> Из <collection>` loop
    /// before `offset` in its method walks, with the start of the loop;
    /// the path is empty when the collection is not a chain of names.
    pub fn loop_collection(&self, offset: usize, variable: &str) -> Option<(usize, Vec<String>)> {
        let variable = variable.to_lowercase();
        self.enclosing_method(offset)?
            .descendants()
            .filter(|node| node.kind() == SyntaxKind::FOR_EACH_STMT)
            .filter(|node| usize::from(node.text_range().start()) <= offset)
            .filter(|node| {
                direct_token(node, SyntaxKind::IDENT)
                    .is_some_and(|name| name.text().to_lowercase() == variable)
            })
            .last()
            .map(|node| {
                let path = node
                    .children()
                    .find(|child| child.kind() == SyntaxKind::EXPR)
                    .and_then(|collection| {
                        receiver_path(self.source[byte_range(collection.text_range())].trim())
                    })
                    .unwrap_or_default();
                (usize::from(node.text_range().start()), path)
            })
    }

    /// The call a variable was last assigned from before `offset` in its
    /// method, with the start of the assignment: `Объект =
    /// Справочники.Валюты.СоздатьЭлемент();` gives `[Справочники, Валюты,
    /// СоздатьЭлемент]`. Any other value gives an empty path.
    pub fn assigned_call(&self, offset: usize, variable: &str) -> Option<(usize, Vec<String>)> {
        let variable = variable.to_lowercase();
        self.enclosing_method(offset)?
            .descendants()
            .filter(|node| node.kind() == SyntaxKind::ASSIGN_STMT)
            .filter(|node| usize::from(node.text_range().end()) <= offset)
            .filter(|node| {
                node.first_child().is_some_and(|target| {
                    target.kind() == SyntaxKind::IDENT
                        && target.text().to_string().trim().to_lowercase() == variable
                })
            })
            .last()
            .map(|node| {
                // The value is the call itself, not a call inside it.
                let path = node
                    .children()
                    .find(|child| child.kind() == SyntaxKind::EXPR)
                    .and_then(|value| {
                        value
                            .descendants()
                            .find(|child| child.kind() != SyntaxKind::EXPR)
                            .filter(|child| child.kind() == SyntaxKind::CALL_EXPR)
                    })
                    .and_then(|call| call.first_child())
                    .and_then(|callee| {
                        receiver_path(self.source[byte_range(callee.text_range())].trim())
                    })
                    .unwrap_or_default();
                (usize::from(node.text_range().start()), path)
            })
    }

    fn enclosing_method(&self, offset: usize) -> Option<SyntaxNode> {
        self.method_nodes().find(|method| {
            let range = byte_range(method.text_range());
            range.start <= offset && offset <= range.end
        })
    }
}

/// `А.Б.В` as `[А, Б, В]`, read backwards from the end of `chain`; `None`
/// when the last segment is not a name (`Ф()`, `М[0]`, nothing).
fn receiver_path(chain: &str) -> Option<Vec<String>> {
    let mut segments = Vec::new();
    let mut rest = chain;
    loop {
        let start = rest
            .char_indices()
            .rev()
            .take_while(|(_, character)| is_name_char(*character))
            .last()
            .map(|(at, _)| at)?;
        let name = &rest[start..];
        if name
            .chars()
            .next()
            .is_some_and(|first| first.is_ascii_digit())
        {
            return None;
        }
        segments.push(name.to_owned());
        match rest[..start].strip_suffix('.') {
            Some(before) => rest = before,
            None => break,
        }
    }
    segments.reverse();
    Some(segments)
}

/// [`declared_variables`] spelled as declared, first declaration wins.
fn declared_variable_spellings(scope: &SyntaxNode, in_method: bool) -> Vec<String> {
    let wanted = declared_variables(scope, in_method);
    let mut seen = BTreeSet::new();
    let mut names = Vec::new();
    for token in scope
        .descendants_with_tokens()
        .filter_map(|element| element.into_token())
        .filter(|token| token.kind() == SyntaxKind::IDENT)
    {
        let lower = token.text().to_lowercase();
        if wanted.contains(&lower) && seen.insert(lower) {
            names.push(token.text().to_owned());
        }
    }
    names
}

fn unclosed_end(list: &SyntaxNode) -> usize {
    let end = list.text_range().end();
    // The list may end in an empty error node, which has no last token.
    let root = list.ancestors().last().unwrap_or_else(|| list.clone());
    let mut next = root.token_at_offset(end).right_biased();
    while let Some(token) = next
        .as_ref()
        .filter(|token| token.kind().is_trivia() || token.text_range().end() <= end)
    {
        next = token.next_token();
    }
    usize::from(next.map_or(root.text_range().end(), |token| token.text_range().start()))
}

fn region(
    range: Range<usize>,
    start: Range<usize>,
    name: Option<String>,
    name_range: Option<Range<usize>>,
) -> EditorSymbol {
    EditorSymbol {
        name: name.unwrap_or_default(),
        kind: EditorSymbolKind::Region,
        range,
        selection_range: name_range.unwrap_or(start),
        is_export: false,
        parameters: Vec::new(),
        description: None,
        children: Vec::new(),
    }
}

fn attach(open: &mut [EditorSymbol], roots: &mut Vec<EditorSymbol>, symbol: EditorSymbol) {
    match open.last_mut() {
        Some(parent) => parent.children.push(symbol),
        None => roots.push(symbol),
    }
}

fn direct_token(node: &SyntaxNode, kind: SyntaxKind) -> Option<SyntaxToken> {
    node.children_with_tokens()
        .filter_map(|element| element.into_token())
        .find(|token| token.kind() == kind)
}

/// The contiguous `//` lines that end on the line before `start`.
fn description_above(source: &str, start: usize) -> Option<String> {
    let line_start = source[..start].rfind(['\n', '\r']).map_or(0, |at| at + 1);
    let mut lines = Vec::new();
    for line in source[..line_start].lines().rev() {
        let Some(comment) = line.trim().strip_prefix("//") else {
            break;
        };
        lines.push(comment.strip_prefix(' ').unwrap_or(comment).trim_end());
    }
    if lines.is_empty() {
        return None;
    }
    lines.reverse();
    Some(lines.join("\n"))
}

/// Runs of comment lines next to each other, two lines or more. Two
/// comments are next to each other when only blanks and one line break
/// stand between them.
fn comment_runs(source: &str, root: &SyntaxNode) -> Vec<FoldBlock> {
    let mut runs = Vec::new();
    let mut current: Option<(Range<usize>, usize)> = None;
    for token in root
        .descendants_with_tokens()
        .filter_map(|element| element.into_token())
        .filter(|token| token.kind() == SyntaxKind::COMMENT)
    {
        let range = byte_range(token.text_range());
        if let Some((run, lines)) = current.as_mut() {
            let gap = source[run.end..range.start].trim_matches([' ', '\t']);
            if matches!(gap, "\n" | "\r\n" | "\r") {
                run.end = range.end;
                *lines += 1;
                continue;
            }
        }
        if let Some((run, lines)) = current.replace((range, 1)) {
            if lines >= 2 {
                runs.push(FoldBlock {
                    range: run,
                    kind: FoldKind::Comment,
                });
            }
        }
    }
    if let Some((run, lines)) = current {
        if lines >= 2 {
            runs.push(FoldBlock {
                range: run,
                kind: FoldKind::Comment,
            });
        }
    }
    runs
}

/// Lower-cased names `scope` declares: `Перем`, and inside a method also
/// assignment targets and loop variables. Nested methods are not looked at.
fn declared_variables(scope: &SyntaxNode, in_method: bool) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    let mut stack = scope.children().collect::<Vec<_>>();
    while let Some(node) = stack.pop() {
        match node.kind() {
            SyntaxKind::PROCEDURE_DEF | SyntaxKind::FUNCTION_DEF => continue,
            SyntaxKind::VAR_DEF => names.extend(
                node.children_with_tokens()
                    .filter_map(|element| element.into_token())
                    .filter(|token| token.kind() == SyntaxKind::IDENT)
                    .map(|token| token.text().to_lowercase()),
            ),
            SyntaxKind::ASSIGN_STMT if in_method => {
                if let Some(target) = node
                    .first_child()
                    .filter(|target| target.kind() == SyntaxKind::IDENT)
                {
                    names.insert(target.text().to_string().to_lowercase());
                }
            }
            SyntaxKind::FOR_STMT | SyntaxKind::FOR_EACH_STMT if in_method => {
                if let Some(variable) = direct_token(&node, SyntaxKind::IDENT) {
                    names.insert(variable.text().to_lowercase());
                }
                if let Some(target) = node
                    .children()
                    .find(|child| child.kind() == SyntaxKind::IDENT)
                {
                    names.insert(target.text().to_string().to_lowercase());
                }
            }
            _ => {}
        }
        stack.extend(node.children());
    }
    names
}

type Scope = (Range<usize>, BTreeSet<String>, BTreeSet<String>);

fn classify_name(
    token: &SyntaxToken,
    scope: Option<&Scope>,
    module_variables: &BTreeSet<String>,
) -> Option<(TokenClass, bool)> {
    let parent = token.parent()?;
    match parent.kind() {
        SyntaxKind::PROCEDURE_DEF | SyntaxKind::FUNCTION_DEF => Some((TokenClass::Method, true)),
        SyntaxKind::PARAM => Some((TokenClass::Parameter, true)),
        SyntaxKind::VAR_DEF => Some((TokenClass::Variable, true)),
        SyntaxKind::FOR_STMT | SyntaxKind::FOR_EACH_STMT => Some((TokenClass::Variable, false)),
        SyntaxKind::PRE_REGION_DIR => Some((TokenClass::Directive, false)),
        SyntaxKind::NEW_EXPR => Some((TokenClass::Type, false)),
        kind if kind.is_annotation()
            || matches!(
                kind,
                SyntaxKind::ANNOTATION | SyntaxKind::COMPILER_DIRECTIVE
            ) =>
        {
            Some((TokenClass::Annotation, false))
        }
        SyntaxKind::FIELD_EXPR => {
            // The name after the dot: a method when called, else a property.
            let called = parent
                .parent()
                .is_some_and(|call| call.kind() == SyntaxKind::CALL_EXPR)
                && parent
                    .next_sibling()
                    .is_some_and(|next| next.kind() == SyntaxKind::ARG_LIST);
            Some((
                if called {
                    TokenClass::Method
                } else {
                    TokenClass::Property
                },
                false,
            ))
        }
        SyntaxKind::IDENT => {
            let callee = parent.parent().is_some_and(|call| {
                call.kind() == SyntaxKind::CALL_EXPR && call.first_child().as_ref() == Some(&parent)
            });
            if callee {
                return Some((TokenClass::Method, false));
            }
            let name = token.text().to_lowercase();
            match scope {
                Some((_, parameters, _)) if parameters.contains(&name) => {
                    Some((TokenClass::Parameter, false))
                }
                Some((_, _, locals)) if locals.contains(&name) => {
                    Some((TokenClass::Variable, false))
                }
                _ if module_variables.contains(&name) => Some((TokenClass::Variable, false)),
                _ => None,
            }
        }
        kind if kind.is_preprocessor()
            || matches!(
                kind,
                SyntaxKind::PRE_SYMBOL | SyntaxKind::PRE_LOGICAL_OPERAND | SyntaxKind::PRE_EXPR
            ) =>
        {
            Some((TokenClass::Directive, false))
        }
        _ => None,
    }
}

/// `Имя` of a plain call, or `Квалификатор.Имя` when the qualifier is a
/// plain name. Any other callee (`А.Б.В()`, `Ф()()`) has no signature here.
fn callee(node: &SyntaxNode) -> Option<(Option<String>, SyntaxToken)> {
    match node.kind() {
        SyntaxKind::IDENT => Some((None, direct_token(node, SyntaxKind::IDENT)?)),
        SyntaxKind::FIELD_EXPR => {
            let qualifier = node
                .first_child()
                .filter(|q| q.kind() == SyntaxKind::IDENT)?;
            let name = direct_token(node, SyntaxKind::IDENT)?;
            Some((Some(qualifier.text().to_string()), name))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    const MODULE: &str = "Перем МодульнаяПеременная;

#Область ПрограммныйИнтерфейс

// Складывает два числа.
// Второе необязательно.
&НаСервере
Функция Сложить(Знач А, Б = 1) Экспорт
\tПерем Итог;
\tЕсли А > 0 Тогда
\t\tИтог = ОбщМод.Метод(а, Б.Поле(1), );
\tКонецЕсли;
\tДля Каждого Х Из Б Цикл
\t\tСообщить(\"т\" + Х + модульнаяпеременная);
\tКонецЦикла;
\tВозврат Итог;
КонецФункции

#Область Служебные
Процедура Вспомогательная()
\tНовый Структура;
КонецПроцедуры
#КонецОбласти

#КонецОбласти

Процедура ВнеОбластей()
КонецПроцедуры
";

    fn at(text: &str) -> usize {
        MODULE.find(text).unwrap()
    }

    #[test]
    fn symbols_nest_methods_in_their_regions() {
        let parsed = parse(MODULE);
        let symbols = parsed.editor_symbols();
        let names =
            |symbols: &[EditorSymbol]| symbols.iter().map(|s| s.name.clone()).collect::<Vec<_>>();
        assert_eq!(names(&symbols), ["ПрограммныйИнтерфейс", "ВнеОбластей"]);
        let interface = &symbols[0];
        assert_eq!(interface.kind, EditorSymbolKind::Region);
        assert_eq!(names(&interface.children), ["Сложить", "Служебные"]);
        assert_eq!(names(&interface.children[1].children), ["Вспомогательная"]);

        let add = &interface.children[0];
        assert_eq!(add.kind, EditorSymbolKind::Function);
        assert!(add.is_export);
        assert_eq!(&MODULE[add.selection_range.clone()], "Сложить");
        assert_eq!(add.range.start, at("&НаСервере"));
        assert_eq!(
            add.description.as_deref(),
            Some("Складывает два числа.\nВторое необязательно.")
        );
        let parameters = &add.parameters;
        assert_eq!(parameters.len(), 2);
        assert!(parameters[0].by_value && !parameters[1].by_value);
        assert_eq!(parameters[1].default_value.as_deref(), Some("1"));
        assert_eq!(&MODULE[parameters[1].range.clone()], "Б");
        assert_eq!(symbols[1].description, None);
    }

    #[test]
    fn an_unclosed_region_holds_the_rest_of_the_module() {
        let text = "#Область А\nПроцедура П()\nКонецПроцедуры\n";
        let symbols = parse(text).editor_symbols();
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].range, 0..text.len());
        assert_eq!(symbols[0].children[0].name, "П");
    }

    #[test]
    fn folds_cover_regions_methods_blocks_and_comment_runs() {
        let blocks = parse(MODULE).fold_blocks();
        let starts = blocks
            .iter()
            .map(|block| {
                (
                    MODULE[block.range.clone()].lines().next().unwrap(),
                    block.kind,
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            starts,
            [
                ("#Область ПрограммныйИнтерфейс", FoldKind::Region),
                ("// Складывает два числа.", FoldKind::Comment),
                ("Функция Сложить(Знач А, Б = 1) Экспорт", FoldKind::Block),
                ("Если А > 0 Тогда", FoldKind::Block),
                ("Для Каждого Х Из Б Цикл", FoldKind::Block),
                ("#Область Служебные", FoldKind::Region),
                ("Процедура Вспомогательная()", FoldKind::Block),
                ("Процедура ВнеОбластей()", FoldKind::Block),
            ]
        );
        assert!(MODULE[blocks[3].range.clone()].ends_with("КонецЕсли;"));
    }

    #[test]
    fn tokens_resolve_names_in_their_method_case_insensitively() {
        let tokens = parse(MODULE).classified_tokens();
        let class_at = |offset: usize| {
            tokens
                .iter()
                .find(|token| token.range.start == offset)
                .map(|token| (token.class, token.declaration))
        };
        assert_eq!(class_at(at("Перем")), Some((TokenClass::Keyword, false)));
        assert_eq!(
            class_at(at("МодульнаяПеременная")),
            Some((TokenClass::Variable, true))
        );
        assert_eq!(
            class_at(at("#Область")),
            Some((TokenClass::Directive, false))
        );
        assert_eq!(
            class_at(at("// Складывает")),
            Some((TokenClass::Comment, false))
        );
        assert_eq!(
            class_at(at("&НаСервере")),
            Some((TokenClass::Annotation, false))
        );
        assert_eq!(class_at(at("Сложить")), Some((TokenClass::Method, true)));
        assert_eq!(class_at(at("А,")), Some((TokenClass::Parameter, true)));
        assert_eq!(class_at(at("Итог;")), Some((TokenClass::Variable, true)));
        // `а` names the parameter `А`.
        assert_eq!(class_at(at("а, Б")), Some((TokenClass::Parameter, false)));
        assert_eq!(class_at(at("Метод")), Some((TokenClass::Method, false)));
        assert_eq!(class_at(at("Поле")), Some((TokenClass::Method, false)));
        assert_eq!(class_at(at("Х Из")), Some((TokenClass::Variable, false)));
        assert_eq!(class_at(at("Сообщить")), Some((TokenClass::Method, false)));
        assert_eq!(class_at(at("\"т\"")), Some((TokenClass::String, false)));
        assert_eq!(
            class_at(at("модульнаяпеременная)")),
            Some((TokenClass::Variable, false))
        );
        assert_eq!(class_at(at("Структура")), Some((TokenClass::Type, false)));
        assert_eq!(class_at(at("1), )")), Some((TokenClass::Number, false)));
        // A global context name no declaration covers stays uncoloured.
        assert_eq!(class_at(at("ОбщМод")), None);
        assert!(tokens
            .windows(2)
            .all(|pair| pair[0].range.end <= pair[1].range.start));
    }

    #[test]
    fn the_call_around_a_cursor_names_its_callee_and_argument() {
        let parsed = parse(MODULE);
        let first = at("а, Б.Поле");
        let call = parsed.call_at(first).unwrap();
        assert_eq!(call.qualifier.as_deref(), Some("ОбщМод"));
        assert_eq!(call.name, "Метод");
        assert_eq!(call.active_parameter, 0);
        // Inside the nested call the inner one wins.
        let inner = parsed.call_at(at("1), )") + 1).unwrap();
        assert_eq!(
            (inner.qualifier.as_deref(), inner.name.as_str()),
            (Some("Б"), "Поле")
        );
        // After the second comma: the third, empty argument.
        let third = parsed.call_at(at(", );") + 2).unwrap();
        assert_eq!((third.name.as_str(), third.active_parameter), ("Метод", 2));
        assert_eq!(
            parsed
                .call_at(at("Сообщить(") + "Сообщить(".len())
                .map(|call| call.qualifier),
            Some(None)
        );
        // Not inside any argument list.
        assert_eq!(parsed.call_at(at("Перем Итог")), None);
    }

    #[test]
    fn a_call_being_typed_still_has_its_signature_context() {
        let text = "Процедура П()\n\tСложить(1, \nКонецПроцедуры\n";
        let parsed = parse(text);
        let call = parsed.call_at(text.find(", ").unwrap() + 2).unwrap();
        assert_eq!((call.name.as_str(), call.active_parameter), ("Сложить", 1));
    }

    #[test]
    fn a_completion_site_knows_the_typed_prefix_and_the_receiver() {
        let text = "Процедура П(Парам)\n\tПерем Лок;\n\tОбъект = Справочники.Валюты.СоздатьЭлемент();\n\tДля Каждого Стр Из Объект.Товары Цикл\n\t\tСтр.Ко\n\tКонецЦикла;\n\tСправочники.Ва\n\tФ().\n\tСо // Сооб\n\tА = \"Спр\n\tПа\nКонецПроцедуры\n";
        let parsed = parse(text);
        let end_of = |needle: &str| text.find(needle).unwrap() + needle.len();

        let member = parsed.completion_site(end_of("Стр.Ко")).unwrap();
        assert_eq!(&text[member.prefix_range.clone()], "Ко");
        assert_eq!(member.receiver, Some(vec!["Стр".to_owned()]));
        let manager = parsed
            .completion_site(text.rfind("Справочники.Ва").unwrap() + "Справочники.Ва".len())
            .unwrap();
        assert_eq!(&text[manager.prefix_range], "Ва");
        assert_eq!(manager.receiver, Some(vec!["Справочники".to_owned()]));
        assert_eq!(
            parsed.completion_site(end_of("Ф().")).unwrap().receiver,
            Some(Vec::new())
        );
        let name = parsed.completion_site(end_of("\tПа")).unwrap();
        assert_eq!((&text[name.prefix_range], name.receiver), ("Па", None));
        // No completion in a comment or a string.
        assert_eq!(parsed.completion_site(end_of("// Сооб")), None);
        assert_eq!(parsed.completion_site(end_of("\"Спр")), None);
        assert!(parsed.completion_site(end_of("\tСо")).is_some());

        let at = end_of("Стр.Ко");
        let visible = parsed.visible_names(at);
        assert_eq!(visible.parameters, ["Парам"]);
        assert_eq!(visible.locals, ["Лок", "Объект", "Стр", "А"]);
        assert_eq!(
            parsed.loop_collection(at, "стр").map(|(_, path)| path),
            Some(vec!["Объект".to_owned(), "Товары".to_owned()])
        );
        assert_eq!(
            parsed.assigned_call(at, "Объект").map(|(_, path)| path),
            Some(vec![
                "Справочники".to_owned(),
                "Валюты".to_owned(),
                "СоздатьЭлемент".to_owned()
            ])
        );
        assert_eq!(parsed.assigned_call(at, "Лок"), None);
        // The cursor inside a word: the edit covers the whole word.
        let inside = text.find("Справочники.Ва").unwrap() + "Справоч".len();
        let site = parsed.completion_site(inside).unwrap();
        assert_eq!(&text[site.prefix_range], "Справочники");
    }
}
