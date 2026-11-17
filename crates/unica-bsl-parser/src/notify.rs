//! Синтаксические факты о местах создания описания оповещения.

use crate::{byte_range, ParsedModule};
use bsl_syntax::ast::{AstNode, NewExpr};
use bsl_syntax::ast_utils::new_expr_type_name_token;
use bsl_syntax::{SyntaxKind, SyntaxNode, SyntaxToken};
use std::collections::HashSet;
use std::ops::Range;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotifyDescriptionSite {
    pub range: Range<usize>,
    pub handler: Option<String>,
    pub receiver: Option<String>,
}

impl ParsedModule<'_> {
    pub fn notification_descriptions(&self) -> Vec<NotifyDescriptionSite> {
        let root = self.parsed.syntax_node();
        let implicit_module_names = implicit_module_names(&root);
        root.descendants()
            .filter_map(NewExpr::cast)
            .filter_map(|constructor| {
                let node = constructor.syntax();
                let name = new_expr_type_name_token(node)?;
                if !matches!(
                    name.text().to_lowercase().as_str(),
                    "описаниеоповещения" | "notifydescription"
                ) {
                    return None;
                }
                let range = byte_range(node.text_range());
                let mut site = NotifyDescriptionSite {
                    range,
                    handler: None,
                    receiver: None,
                };
                let arguments = node
                    .children()
                    .find(|child| child.kind() == SyntaxKind::ARG_LIST);
                if let Some(arguments) = arguments.filter(|arguments| {
                    arguments
                        .children_with_tokens()
                        .any(|element| element.kind() == SyntaxKind::R_PAREN)
                        && !self.errors().iter().any(|error| {
                            error.range.start < site.range.end
                                && error.range.end >= site.range.start
                        })
                }) {
                    let [handler, receiver] = first_two_slots(&arguments);
                    site.handler = handler
                        .and_then(|node| {
                            single_token(node, SyntaxKind::LITERAL, SyntaxKind::STRING)
                        })
                        .and_then(|literal| {
                            literal
                                .text()
                                .strip_prefix('"')?
                                .strip_suffix('"')
                                .map(|text| text.replace("\"\"", "\""))
                        });
                    site.receiver = receiver
                        .and_then(|node| single_token(node, SyntaxKind::IDENT, SyntaxKind::IDENT))
                        .filter(|name| {
                            self.local_definition(name.text(), site.range.start, false)
                                .is_none()
                                && !implicit_module_names
                                    .contains(&crate::symbols::name_key(name.text()))
                        })
                        .map(|name| name.text().to_string());
                }
                Some(site)
            })
            .collect()
    }
}

fn implicit_module_names(root: &SyntaxNode) -> HashSet<String> {
    root.descendants()
        .filter(|node| {
            matches!(
                node.kind(),
                SyntaxKind::ASSIGN_STMT | SyntaxKind::FOR_STMT | SyntaxKind::FOR_EACH_STMT
            )
        })
        .filter(|node| {
            !node.ancestors().any(|ancestor| {
                matches!(
                    ancestor.kind(),
                    SyntaxKind::PROCEDURE_DEF | SyntaxKind::FUNCTION_DEF
                )
            })
        })
        .filter_map(|node| {
            let target = if node.kind() == SyntaxKind::ASSIGN_STMT {
                node.first_child()
                    .filter(|child| child.kind() == SyntaxKind::IDENT)?
            } else {
                node
            };
            target
                .children_with_tokens()
                .filter_map(|element| element.into_token())
                .find(|token| token.kind() == SyntaxKind::IDENT)
                .map(|token| crate::symbols::name_key(token.text()))
        })
        .collect()
}

fn first_two_slots(arguments: &SyntaxNode) -> [Option<SyntaxNode>; 2] {
    let mut slots = [None, None];
    let mut slot = 0;
    for element in arguments.children_with_tokens() {
        match element.kind() {
            SyntaxKind::COMMA => {
                slot += 1;
                if slot == slots.len() {
                    break;
                }
            }
            SyntaxKind::EXPR => slots[slot] = element.into_node(),
            _ => {}
        }
    }
    slots
}

fn single_token(
    mut expression: SyntaxNode,
    node_kind: SyntaxKind,
    token_kind: SyntaxKind,
) -> Option<SyntaxToken> {
    while matches!(expression.kind(), SyntaxKind::EXPR | SyntaxKind::PAREN_EXPR) {
        let mut children = expression.children();
        let child = children.next()?;
        if children.next().is_some() {
            return None;
        }
        expression = child;
    }
    if expression.kind() == node_kind {
        let mut tokens = expression
            .descendants_with_tokens()
            .filter_map(|element| element.into_token())
            .filter(|token| !token.kind().is_trivia());
        let token = tokens.next()?;
        if token.kind() == token_kind && tokens.next().is_none() {
            return Some(token);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    #[test]
    fn notification_sites_keep_literal_slots_and_distinct_source_positions() {
        let source = r#"Procedure Start(Name)
// New NotifyDescription("Comment", ThisObject)
    Text = "New NotifyDescription(""Text"", ThisObject)";
    First = Новый ОписаниеОповещения("Done", (ЭтотОбъект), );
    Send(New NotifyDescription("Done", ThisObject), New NotifyDescription("Done", ModuleName));
    Second = New NotifyDescription(, ThisObject);
    Third = New NotifyDescription(Name, Receiver());
    Fourth = New NotifyDescription("Do" + "ne",);
EndProcedure
"#;
        let parsed = crate::parse(source);
        let sites = parsed.notification_descriptions();
        assert_eq!(sites.len(), 6, "{sites:?}");
        assert_eq!(
            sites
                .iter()
                .map(|site| (site.handler.as_deref(), site.receiver.as_deref()))
                .collect::<Vec<_>>(),
            vec![
                (Some("Done"), Some("ЭтотОбъект")),
                (Some("Done"), Some("ThisObject")),
                (Some("Done"), Some("ModuleName")),
                (None, Some("ThisObject")),
                (None, None),
                (None, None),
            ]
        );
        let lines = crate::symbols::LineIndex::new(source);
        let first = lines.position(sites[1].range.start);
        let second = lines.position(sites[2].range.start);
        assert_eq!(first.0, second.0);
        assert!(first.1 < second.1);
    }

    #[test]
    fn notification_receivers_respect_the_modules_and_methods_own_names() {
        let source = r#"Var ModuleVariable;
Procedure Start(Parameter)
    Var Explicit;
    If True Then
        Implicit = Receiver();
    EndIf;
    For Each Item In Items Do
        Value = New NotifyDescription("Done", Item);
    EndDo;
    A = New NotifyDescription("Done", Parameter);
    B = New NotifyDescription("Done", Explicit);
    C = New NotifyDescription("Done", Implicit);
    D = New NotifyDescription("Done", ModuleVariable);
    E = New NotifyDescription("Done", ModuleAssignment);
    F = New NotifyDescription("Done", RegisteredModule);
EndProcedure
ModuleAssignment = Receiver();
"#;
        let sites = crate::parse(source).notification_descriptions();
        assert_eq!(sites.len(), 7, "{sites:?}");
        assert!(
            sites[..6].iter().all(|site| site.receiver.is_none()),
            "{sites:?}"
        );
        assert_eq!(sites[6].receiver.as_deref(), Some("RegisteredModule"));
    }
}
