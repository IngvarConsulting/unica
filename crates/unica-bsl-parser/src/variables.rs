use super::{byte_range, ParsedModule};
use bsl_syntax::SyntaxKind;
use std::ops::Range;

/// One module-level name, retaining its own export marker and declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VariableSyntax {
    pub name: String,
    pub is_export: bool,
    pub directive: Option<String>,
    pub declaration: Range<usize>,
    pub keyword: Range<usize>,
}

impl ParsedModule<'_> {
    pub fn variables(&self) -> Vec<VariableSyntax> {
        let mut result = Vec::new();
        for node in self.parsed.syntax_node().descendants().filter(|node| {
            node.kind() == SyntaxKind::VAR_DEF
                && !node.ancestors().any(|ancestor| {
                    matches!(
                        ancestor.kind(),
                        SyntaxKind::PROCEDURE_DEF | SyntaxKind::FUNCTION_DEF
                    )
                })
        }) {
            let directive = node
                .children()
                .find(|child| child.kind() == SyntaxKind::COMPILER_DIRECTIVE)
                .map(|child| child.text().to_string().trim().to_string());
            let tokens: Vec<_> = node
                .children_with_tokens()
                .filter_map(|element| element.into_token())
                .collect();
            let Some(keyword) = tokens
                .iter()
                .find(|token| token.kind() == SyntaxKind::KW_VAR)
            else {
                continue;
            };
            for (index, token) in tokens.iter().enumerate() {
                if token.kind() != SyntaxKind::IDENT {
                    continue;
                }
                // The platform applies Export to the preceding name only.
                let is_export = tokens[index + 1..]
                    .iter()
                    .take_while(|token| {
                        !matches!(token.kind(), SyntaxKind::COMMA | SyntaxKind::SEMICOLON)
                    })
                    .any(|token| token.kind() == SyntaxKind::KW_EXPORT);
                result.push(VariableSyntax {
                    name: token.text().to_string(),
                    is_export,
                    directive: directive.clone(),
                    declaration: byte_range(node.text_range()),
                    keyword: byte_range(keyword.text_range()),
                });
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use crate::parse;

    #[test]
    fn module_variables_keep_each_export_and_exclude_method_locals() {
        let source = "&AtClient\r\nVar A Export, B;\r\nVar C,\r\n D Export;\r\nProcedure P()\r\nVar Local;\r\nEndProcedure";
        let parsed = parse(source);
        assert!(parsed.errors().is_empty(), "{:?}", parsed.errors());
        let variables = parsed.variables();
        assert_eq!(
            variables
                .iter()
                .map(|v| (v.name.as_str(), v.is_export))
                .collect::<Vec<_>>(),
            [("A", true), ("B", false), ("C", false), ("D", true)]
        );
        assert_eq!(variables[0].directive.as_deref(), Some("&AtClient"));
        assert_eq!(variables[1].directive, variables[0].directive);
        assert_eq!(variables[2].directive, None);
        assert_eq!(&source[variables[0].keyword.clone()], "Var");
        assert_eq!(variables[2].keyword, variables[3].keyword);
        assert!(source[variables[0].declaration.clone()].starts_with("&AtClient"));
    }
}
