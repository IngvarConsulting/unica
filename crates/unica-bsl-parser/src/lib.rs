//! Owned syntax facts for the existing BSL readers and planners.

mod outline;

use bsl_syntax::ast::{Annotation, AstNode, FunctionDef, PreIfDir, PreRegionDir, ProcedureDef};
use bsl_syntax::{SyntaxKind, SyntaxNode};
use serde::Serialize;
use std::ops::Range;

pub use outline::{
    bsl_method_parameters, exact_bsl_method_facts, exported_bsl_procedure_matches,
    first_exported_bsl_procedure, misplaced_variable_declarations, outline_regions,
    parse_module as outline_module, BslMethodFacts, BslMethodParameters, BslProcedureRequirement,
    OutlineRegion, MISPLACED_VARIABLE_MESSAGE,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeOutlineMethod {
    pub name: String,
    pub kind: CodeOutlineMethodKind,
    pub parameters: Vec<CodeOutlineParameter>,
    #[serde(rename = "export")]
    pub is_export: bool,
    pub line: usize,
    pub end_line: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CodeOutlineMethodKind {
    Procedure,
    Function,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeOutlineParameter {
    pub name: String,
    pub by_value: bool,
    pub default_value: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiagnosticRange {
    pub start_line: usize,
    pub start_column: usize,
    pub end_line: usize,
    pub end_column: usize,
}

impl DiagnosticRange {
    pub fn is_non_empty(self) -> bool {
        (self.start_line, self.start_column) < (self.end_line, self.end_column)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseDiagnostic {
    pub range: Range<usize>,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtensionAnnotationKind {
    Before,
    After,
    Instead,
    ChangeAndValidate,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnnotationFact {
    pub range: Range<usize>,
    pub text: String,
    pub is_compiler_directive: bool,
    pub is_annotation: bool,
    pub extension_kind: Option<ExtensionAnnotationKind>,
    pub target: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MethodSyntax {
    pub name: Option<String>,
    pub kind: CodeOutlineMethodKind,
    pub is_export: bool,
    pub range: Range<usize>,
    pub header_keyword: Option<Range<usize>>,
    pub parameter_list: Option<Range<usize>>,
    pub declaration_end: Option<usize>,
    pub closing_keyword: Option<Range<usize>>,
    pub async_start: Option<usize>,
    pub parameters: Result<Vec<CodeOutlineParameter>, String>,
    pub annotations: Vec<AnnotationFact>,
    /// Patch planning uses direct declaration children, rather than descendants.
    pub direct_declaration_end: Option<usize>,
    pub direct_closing_start: Option<usize>,
}

/// Existing AST-helper observations retained independently of token pairing.
/// Outline deliberately pairs tokens instead: the upstream helpers have
/// different behavior for directive case and whitespace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegionMarker {
    pub range: Range<usize>,
    pub is_start: bool,
    pub is_end: bool,
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConditionalBranch {
    pub range: Range<usize>,
    pub guard: String,
}

pub struct ParsedModule<'a> {
    source: &'a str,
    parsed: bsl_syntax::Parse<SyntaxNode>,
    diagnostics: Vec<ParseDiagnostic>,
}

pub fn parse(text: &str) -> ParsedModule<'_> {
    let parsed = bsl_parser::parse(text);
    let diagnostics = parsed
        .errors()
        .iter()
        .map(|error| ParseDiagnostic {
            range: byte_range(error.range()),
            message: error.message().to_string(),
        })
        .collect();
    ParsedModule {
        source: text,
        parsed,
        diagnostics,
    }
}

impl ParsedModule<'_> {
    pub fn errors(&self) -> &[ParseDiagnostic] {
        &self.diagnostics
    }

    pub fn methods(&self) -> Vec<MethodSyntax> {
        let source = self.source;
        self.parsed
            .syntax_node()
            .descendants()
            .filter_map(|node| {
                let (name, kind, is_export) = if let Some(method) = ProcedureDef::cast(node.clone())
                {
                    (
                        method
                            .name_or_keyword()
                            .map(|token| token.text().to_string()),
                        CodeOutlineMethodKind::Procedure,
                        method.export_keyword().is_some(),
                    )
                } else {
                    let method = FunctionDef::cast(node.clone())?;
                    (
                        method
                            .name_or_keyword()
                            .map(|token| token.text().to_string()),
                        CodeOutlineMethodKind::Function,
                        method.export_keyword().is_some(),
                    )
                };
                let header_keyword = first_range(&node, |kind| {
                    matches!(kind, SyntaxKind::KW_PROCEDURE | SyntaxKind::KW_FUNCTION)
                });
                let parameter_list = node
                    .children()
                    .find(|child| child.kind() == SyntaxKind::PARAM_LIST)
                    .map(|child| byte_range(child.text_range()));
                let export = first_range(&node, |kind| kind == SyntaxKind::KW_EXPORT);
                let declaration_end = export
                    .map(|range| range.end)
                    .or_else(|| parameter_list.as_ref().map(|range| range.end));
                let closing_keyword = first_range(&node, |kind| {
                    matches!(
                        kind,
                        SyntaxKind::KW_END_PROCEDURE | SyntaxKind::KW_END_FUNCTION
                    )
                });
                let async_start = first_range(&node, |kind| kind == SyntaxKind::KW_ASYNC)
                    .map(|range| range.start);
                let direct_declaration_end = node
                    .children_with_tokens()
                    .filter_map(|element| element.into_token())
                    .find(|token| token.kind() == SyntaxKind::KW_EXPORT)
                    .map(|token| usize::from(token.text_range().end()))
                    .or_else(|| parameter_list.as_ref().map(|range| range.end));
                let direct_closing_start = node
                    .children_with_tokens()
                    .filter_map(|element| element.into_token())
                    .find(|token| {
                        matches!(
                            token.kind(),
                            SyntaxKind::KW_END_PROCEDURE | SyntaxKind::KW_END_FUNCTION
                        )
                    })
                    .map(|token| usize::from(token.text_range().start()));
                let annotations = node
                    .children()
                    .filter_map(Annotation::cast)
                    .map(|annotation| {
                        let range = byte_range(annotation.syntax().text_range());
                        let extension_kind = annotation.kind_token().and_then(|token| match token
                            .kind()
                        {
                            SyntaxKind::ANN_BEFORE => Some(ExtensionAnnotationKind::Before),
                            SyntaxKind::ANN_AFTER => Some(ExtensionAnnotationKind::After),
                            SyntaxKind::ANN_AROUND => Some(ExtensionAnnotationKind::Instead),
                            SyntaxKind::ANN_CHANGE_AND_VALIDATE => {
                                Some(ExtensionAnnotationKind::ChangeAndValidate)
                            }
                            _ => None,
                        });
                        let target = annotation
                            .syntax()
                            .descendants_with_tokens()
                            .filter_map(|element| element.into_token())
                            .find(|token| token.kind() == SyntaxKind::STRING)
                            .map(|token| token.text().trim_matches('"').to_string());
                        AnnotationFact {
                            text: source[range.clone()].trim().to_string(),
                            range,
                            is_compiler_directive: annotation.syntax().kind()
                                == SyntaxKind::COMPILER_DIRECTIVE,
                            is_annotation: annotation.syntax().kind() == SyntaxKind::ANNOTATION,
                            extension_kind,
                            target,
                        }
                    })
                    .collect();
                Some(MethodSyntax {
                    name,
                    kind,
                    is_export,
                    range: byte_range(node.text_range()),
                    header_keyword,
                    parameter_list,
                    declaration_end,
                    closing_keyword,
                    async_start,
                    parameters: outline::parameters(&node),
                    annotations,
                    direct_declaration_end,
                    direct_closing_start,
                })
            })
            .collect()
    }

    pub fn region_markers(&self) -> Vec<RegionMarker> {
        let mut markers = self
            .parsed
            .syntax_node()
            .descendants()
            .filter_map(PreRegionDir::cast)
            .map(|marker| RegionMarker {
                range: byte_range(marker.syntax().text_range()),
                is_start: marker.is_start(),
                is_end: marker.is_end(),
                name: marker.name(),
            })
            .collect::<Vec<_>>();
        markers.sort_by_key(|marker| marker.range.start);
        markers
    }

    pub fn conditional_branches(&self) -> Vec<ConditionalBranch> {
        let mut nodes = self
            .parsed
            .syntax_node()
            .descendants()
            .filter_map(PreIfDir::cast)
            .collect::<Vec<_>>();
        nodes.sort_by_key(|node| node.syntax().text_range().start());
        let mut branches = Vec::new();
        for node in nodes {
            let mut prior = Vec::<String>::new();
            if let Some(condition) = node.condition() {
                let guard = normalize_guard(condition.syntax());
                if let Some(range) = node_range(node.then_body_nodes()) {
                    branches.push(ConditionalBranch {
                        range,
                        guard: guard.clone(),
                    });
                }
                prior.push(guard);
            }
            for clause in node.elsif_clauses() {
                if let Some(condition) = clause.condition() {
                    let condition = normalize_guard(condition.syntax());
                    let guard = format!("Not ({}) And {condition}", prior.join(" Or "));
                    if let Some(range) = node_range(clause.body_nodes()) {
                        branches.push(ConditionalBranch { range, guard });
                    }
                    prior.push(condition);
                }
            }
            if let Some(clause) = node.else_clause() {
                let guard = format!("Not ({})", prior.join(" Or "));
                if let Some(range) = node_range(clause.body_nodes()) {
                    branches.push(ConditionalBranch { range, guard });
                }
            }
        }
        branches
    }
}

fn byte_range(range: bsl_syntax::TextRange) -> Range<usize> {
    usize::from(range.start())..usize::from(range.end())
}

fn first_range(node: &SyntaxNode, matches: impl Fn(SyntaxKind) -> bool) -> Option<Range<usize>> {
    node.descendants_with_tokens()
        .filter_map(|element| element.into_token())
        .find(|token| matches(token.kind()))
        .map(|token| byte_range(token.text_range()))
}

fn node_range(nodes: impl Iterator<Item = SyntaxNode>) -> Option<Range<usize>> {
    let nodes = nodes.collect::<Vec<_>>();
    Some(
        usize::from(nodes.first()?.text_range().start())
            ..usize::from(nodes.last()?.text_range().end()),
    )
}

fn normalize_guard(node: &SyntaxNode) -> String {
    let mut parts = Vec::new();
    for token in node
        .descendants_with_tokens()
        .filter_map(|element| element.into_token())
    {
        if token.kind().is_trivia() {
            continue;
        }
        let raw = token.text();
        let normalized = match raw.to_lowercase().as_str() {
            "клиент" | "client" => "Client",
            "сервер" | "server" => "Server",
            "вебклиент" | "webclient" => "WebClient",
            "тонкийклиент" | "thinclient" => "ThinClient",
            "толстыйклиент" | "thickclient" => "ThickClient",
            "внешнеесоединение" | "externalconnection" => "ExternalConnection",
            "мобильныйклиент" | "mobileclient" => "MobileClient",
            "мобильноеприложениеклиент" | "mobileappclient" => {
                "MobileAppClient"
            }
            "мобильноеприложениесервер" | "mobileappserver" => {
                "MobileAppServer"
            }
            "и" | "and" => "And",
            "или" | "or" => "Or",
            "не" | "not" => "Not",
            _ => raw,
        };
        parts.push(normalized.to_string());
    }
    parts.join(" ").replace("( ", "(").replace(" )", ")")
}

#[cfg(test)]
mod tests {
    use super::{parse, CodeOutlineMethodKind, ExtensionAnnotationKind};

    #[test]
    fn method_facts_preserve_annotations_parameters_and_exact_patch_body_bounds() {
        let source = "&Before(\"Original\")\n&AtServer\nProcedure Обработать(Val Value = -1) Export\n    Return;\nEndProcedure\n";
        let parsed = parse(source);
        assert!(parsed.errors().is_empty(), "{:?}", parsed.errors());
        let methods = parsed.methods();
        assert_eq!(methods.len(), 1);
        let method = &methods[0];
        assert_eq!(method.name.as_deref(), Some("Обработать"));
        assert_eq!(method.kind, CodeOutlineMethodKind::Procedure);
        assert!(method.is_export);
        assert_eq!(
            method.direct_declaration_end,
            Some(source.find("Export").unwrap() + 6)
        );
        assert_eq!(
            method.direct_closing_start,
            Some(source.find("EndProcedure").unwrap())
        );
        let parameters = method.parameters.as_ref().unwrap();
        assert_eq!(parameters[0].name, "Value");
        assert!(parameters[0].by_value);
        assert_eq!(parameters[0].default_value.as_deref(), Some("-1"));
        assert_eq!(
            method.annotations[0].extension_kind,
            Some(ExtensionAnnotationKind::Before)
        );
        assert_eq!(method.annotations[0].target.as_deref(), Some("Original"));
        assert_eq!(method.annotations[1].text, "&AtServer");
        assert!(method.annotations[1].is_compiler_directive);
    }

    #[test]
    fn conditional_branches_preserve_exclusive_guards_and_source_ranges() {
        let source = "#Если Сервер Тогда\nСообщить(1);\n#ИначеЕсли Клиент Тогда\nСообщить(2);\n#Иначе\nСообщить(3);\n#КонецЕсли\n";
        let parsed = parse(source);
        assert!(parsed.errors().is_empty(), "{:?}", parsed.errors());
        let branches = parsed.conditional_branches();
        assert_eq!(branches.len(), 3);
        assert_eq!(branches[0].guard, "Server");
        assert_eq!(branches[1].guard, "Not (Server) And Client");
        assert_eq!(branches[2].guard, "Not (Server Or Client)");
        for (branch, statement) in
            branches
                .iter()
                .zip(["Сообщить(1)", "Сообщить(2)", "Сообщить(3)"])
        {
            assert_eq!(source[branch.range.clone()].trim(), statement);
        }
    }
}
