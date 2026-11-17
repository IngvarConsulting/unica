//! Сохранённая видимость разделов корневого командного интерфейса.
// module: private

use crate::interface::{INTERFACE_CI_NS as CI_NS, INTERFACE_XR_NS as XR_NS};
use roxmltree::{Document, Node};
use std::{collections::HashSet, ops::Range};
use unica_metadata::configuration::{CfSubsystemVisibility, CfSubsystemVisibilityRole};
use unica_source_designer::registration::{cf_edit_line_ending, cf_edit_line_start};
use unica_xml_format::text::escape_xml;

const SECTION: &str = "SubsystemsVisibility";

#[derive(Debug, Clone)]
pub struct VisibilityUpdate {
    pub subsystem: String,
    pub common: Option<bool>,
    pub roles: Vec<(String, Option<bool>)>,
}

fn ci(node: Node<'_, '_>, tag: &str) -> bool {
    node.has_tag_name(tag) && node.tag_name().namespace().is_none_or(|ns| ns == CI_NS)
}

fn visibility<'a, 'input>(node: Node<'a, 'input>) -> Result<Node<'a, 'input>, String> {
    let mut values = node.children().filter(|child| ci(*child, "Visibility"));
    match (values.next(), values.next()) {
        (Some(visibility), None) => Ok(visibility),
        _ => Err(format!(
            "expected one Visibility for {}",
            name(node, "Subsystem")?
        )),
    }
}

fn flag(node: Node<'_, '_>) -> Result<bool, String> {
    let invalid = || format!("invalid boolean in {}", node.tag_name().name());
    if node.children().any(|child| child.is_element()) {
        return Err(invalid());
    }
    let value: String = node
        .children()
        .filter(|child| child.is_text())
        .filter_map(|child| child.text())
        .collect();
    match value.trim() {
        "true" | "1" => Ok(true),
        "false" | "0" => Ok(false),
        _ => Err(invalid()),
    }
}

fn name<'a>(node: Node<'a, '_>, kind: &str) -> Result<&'a str, String> {
    node.attribute("name")
        .filter(|name| {
            name.split_once('.').is_some_and(|(prefix, rest)| {
                prefix == kind
                    && !rest.contains('.')
                    && unica_xml_format::names::xml_ncname_is_valid(rest)
            })
        })
        .ok_or_else(|| format!("invalid {kind} reference in {SECTION}"))
}

pub(crate) fn read(root: Node<'_, '_>) -> Result<Vec<CfSubsystemVisibility>, String> {
    let mut sections = root.children().filter(|node| ci(*node, SECTION));
    let Some(section) = sections.next() else {
        return Ok(Vec::new());
    };
    if sections.next().is_some() {
        return Err(format!("duplicate {SECTION}"));
    }
    let mut subsystems = HashSet::new();
    section
        .children()
        .filter(|node| ci(*node, "Subsystem"))
        .map(|node| {
            let subsystem = name(node, "Subsystem")?;
            if !subsystems.insert(subsystem) {
                return Err(format!("duplicate visibility for {subsystem}"));
            }
            let values = visibility(node)?;
            let mut common = values
                .children()
                .filter(|node| node.has_tag_name((XR_NS, "Common")));
            let common_flag = common.next().map(flag).transpose()?.unwrap_or(false);
            if common.next().is_some() {
                return Err(format!("duplicate Common for {subsystem}"));
            }
            let mut role_names = HashSet::new();
            let roles = values
                .children()
                .filter(|node| node.has_tag_name((XR_NS, "Value")))
                .map(|node| {
                    let role = name(node, "Role")?;
                    if !role_names.insert(role) {
                        return Err(format!("duplicate visibility for {subsystem} and {role}"));
                    }
                    Ok(CfSubsystemVisibilityRole {
                        role: role.to_string(),
                        visible: flag(node)?,
                    })
                })
                .collect::<Result<_, String>>()?;
            Ok(CfSubsystemVisibility {
                subsystem: subsystem.to_string(),
                common: common_flag,
                roles,
            })
        })
        .collect()
}

fn qualified_ci(root: Node<'_, '_>, tag: &str) -> String {
    if root.lookup_namespace_uri(None) == Some(CI_NS) || root.tag_name().namespace().is_none() {
        return tag.to_string();
    }
    root.lookup_prefix(CI_NS)
        .map_or_else(|| tag.to_string(), |prefix| format!("{prefix}:{tag}"))
}

fn xr_flag(parent: Node<'_, '_>, tag: &str, role: Option<&str>, visible: bool) -> String {
    let namespace = if parent.lookup_namespace_uri(Some("xr")) == Some(XR_NS) {
        String::new()
    } else {
        format!(" xmlns:xr=\"{XR_NS}\"")
    };
    let name = role.map_or_else(String::new, |role| {
        format!(" name=\"{}\"", escape_xml(role))
    });
    format!("<xr:{tag}{namespace}{name}>{visible}</xr:{tag}>")
}

/// Вставляет дочерний фрагмент в закрывающую строку, сохраняя весь старый текст.
fn append(text: &mut String, parent: Range<usize>, fragment: &str, depth: usize) {
    let eol = cf_edit_line_ending(text);
    let close_indent = "\t".repeat(depth);
    let child_indent = "\t".repeat(depth + 1);
    let input = &text[parent.clone()];
    if input.ends_with("/>") {
        let end = input
            .find(|character: char| character.is_whitespace() || character == '/')
            .unwrap();
        let tag = input[1..end].to_string();
        text.replace_range(
            parent.end - 2..parent.end,
            &format!(">{eol}{child_indent}{fragment}{eol}{close_indent}</{tag}>"),
        );
    } else {
        let close = parent.start + input.rfind("</").expect("parsed element has a closing tag");
        let line_start = cf_edit_line_start(text, close);
        if line_start > parent.start && text[line_start..close].trim().is_empty() {
            text.insert_str(line_start, &format!("{child_indent}{fragment}{eol}"));
        } else {
            text.insert_str(
                close,
                &format!("{eol}{child_indent}{fragment}{eol}{close_indent}"),
            );
        }
    }
}

fn delete_range(text: &str, range: Range<usize>) -> Range<usize> {
    let start = cf_edit_line_start(text, range.start);
    let end = text[range.end..]
        .find('\n')
        .map(|relative| range.end + relative);
    if let Some(end) = end {
        if text[start..range.start].trim().is_empty() && text[range.end..end].trim().is_empty() {
            return start..end + 1;
        }
    }
    range
}

pub(crate) fn write(text: &mut String, update: &VisibilityUpdate) -> Result<(), String> {
    let document = Document::parse(text).map_err(|error| error.to_string())?;
    let root = document.root_element();
    if !ci(root, "CommandInterface") {
        return Err("not a CommandInterface document".to_string());
    }
    let section = root.children().find(|node| ci(*node, SECTION));
    let row = section.and_then(|section| {
        section.children().find(|node| {
            ci(*node, "Subsystem") && node.attribute("name") == Some(update.subsystem.as_str())
        })
    });

    if let Some(row) = row {
        let values = visibility(row)?;
        let mut edits = Vec::new();
        let mut additions = Vec::new();
        let common = values
            .children()
            .find(|node| node.has_tag_name((XR_NS, "Common")));
        if let Some(visible) = update.common {
            let fragment = xr_flag(values, "Common", None, visible);
            match common {
                Some(node) if flag(node)? != visible => edits.push((node.range(), fragment)),
                None if visible => {
                    // XR Common precedes per-role values. Missing Common in a
                    // saved row is serialized as false by the platform.
                    if let Some(first) = values
                        .children()
                        .find(|node| node.has_tag_name((XR_NS, "Value")))
                    {
                        let point = delete_range(text, first.range()).start;
                        let eol = cf_edit_line_ending(text);
                        let insertion = if point < first.range().start {
                            format!("\t\t\t\t{fragment}{eol}")
                        } else {
                            fragment
                        };
                        edits.push((point..point, insertion));
                    } else {
                        additions.push(fragment);
                    }
                }
                _ => {}
            }
        }
        for (role, visible) in &update.roles {
            let existing = values.children().find(|node| {
                node.has_tag_name((XR_NS, "Value")) && node.attribute("name") == Some(role.as_str())
            });
            match (existing, visible) {
                (Some(node), Some(visible)) if flag(node)? != *visible => {
                    edits.push((node.range(), xr_flag(values, "Value", Some(role), *visible)));
                }
                (None, Some(visible)) => {
                    additions.push(xr_flag(values, "Value", Some(role), *visible));
                }
                (Some(node), None) => edits.push((delete_range(text, node.range()), String::new())),
                _ => {}
            }
        }
        if !additions.is_empty() {
            // Append first so all existing element offsets remain valid.
            let values = values.range();
            let separator = format!("{}\t\t\t\t", cf_edit_line_ending(text));
            append(text, values, &additions.join(&separator), 3);
        }
        edits.sort_by_key(|(range, _)| (range.start, range.end));
        for (range, replacement) in edits.into_iter().rev() {
            text.replace_range(range, &replacement);
        }
    } else if let Some(visible) = update.common {
        let eol = cf_edit_line_ending(text);
        let context = section.unwrap_or(root);
        let subsystem = qualified_ci(context, "Subsystem");
        let visibility = qualified_ci(context, "Visibility");
        let section_name = qualified_ci(root, SECTION);
        let mut row = format!(
            "<{subsystem} name=\"{}\">{eol}\t\t\t<{visibility}>{eol}\t\t\t\t{}",
            escape_xml(&update.subsystem),
            xr_flag(context, "Common", None, visible)
        );
        for (role, value) in &update.roles {
            if let Some(value) = value {
                row.push_str(&format!(
                    "{eol}\t\t\t\t{}",
                    xr_flag(context, "Value", Some(role), *value)
                ));
            }
        }
        row.push_str(&format!(
            "{eol}\t\t\t</{visibility}>{eol}\t\t</{subsystem}>"
        ));
        if let Some(section) = section {
            append(text, section.range(), &row, 1);
        } else {
            let rendered = format!("<{section_name}>{eol}\t\t{row}{eol}\t</{section_name}>");
            let successor = root.children().find(|node| {
                [
                    "CommandsPlacement",
                    "CommandsOrder",
                    "SubsystemsOrder",
                    "GroupsOrder",
                ]
                .iter()
                .any(|tag| ci(*node, tag))
            });
            if let Some(successor) = successor {
                text.insert_str(successor.range().start, &format!("{rendered}{eol}\t"));
            } else {
                append(text, root.range(), &rendered, 0);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setting_common_and_removing_first_override_uses_one_safe_boundary() {
        let mut source = format!(
            "\u{feff}<CommandInterface xmlns=\"{CI_NS}\" xmlns:r=\"{XR_NS}\">\r\n\
\t<SubsystemsVisibility>\r\n\
\t\t<Subsystem name=\"Subsystem.Sales\">\r\n\
\t\t\t<Visibility>\r\n\
\t\t\t\t<r:Value name=\"Role.Hide\">false</r:Value>\r\n\
\t\t\t\t<!-- keep -->\r\n\
\t\t\t\t<r:Value name=\"Role.Keep\">false</r:Value>\r\n\
\t\t\t</Visibility>\r\n\
\t\t</Subsystem>\r\n\
\t</SubsystemsVisibility>\r\n\
\t<SubsystemsOrder><Subsystem>Subsystem.Sales</Subsystem></SubsystemsOrder>\r\n\
</CommandInterface>"
        );
        write(
            &mut source,
            &VisibilityUpdate {
                subsystem: "Subsystem.Sales".into(),
                common: Some(true),
                roles: vec![("Role.Hide".into(), None)],
            },
        )
        .unwrap();
        let doc = Document::parse(&source).unwrap();
        let rows = read(doc.root_element()).unwrap();
        assert!(rows[0].common);
        assert_eq!(rows[0].roles.len(), 1);
        assert_eq!(rows[0].roles[0].role, "Role.Keep");
        assert!(!rows[0].roles[0].visible);
        assert!(
            source.contains("<!-- keep -->\r\n\t\t\t\t<r:Value name=\"Role.Keep\">false</r:Value>")
        );
        assert!(source
            .contains("<SubsystemsOrder><Subsystem>Subsystem.Sales</Subsystem></SubsystemsOrder>"));
        assert!(source.starts_with('\u{feff}'));
    }

    #[test]
    fn damaged_visibility_does_not_discard_root_order() {
        for malformed in [
            "<Visibility><xr:Common/></Visibility>",
            "<Visibility><xr:Common>false<x:Other/></xr:Common></Visibility>",
            "<Visibility><xr:Common>false<!-- split -->junk</xr:Common></Visibility>",
            "<Visibility><xr:Common>false</xr:Common></Visibility><Visibility><xr:Common>true</xr:Common></Visibility>",
        ] {
            let text = format!(
                r#"<CommandInterface xmlns="{CI_NS}" xmlns:xr="{XR_NS}" xmlns:x="urn:test">
                    <SubsystemsVisibility><Subsystem name="Subsystem.Sales">
                        {malformed}
                    </Subsystem></SubsystemsVisibility>
                    <SubsystemsOrder><Subsystem>Subsystem.Sales</Subsystem></SubsystemsOrder>
                </CommandInterface>"#
            );
            let interface = crate::configuration::parse_cf_command_interface_xml(&text).unwrap();
            assert_eq!(interface.subsystem_order, ["Subsystem.Sales"]);
            assert!(interface.subsystems_visibility.is_none(), "{malformed}");
            assert!(interface.subsystems_visibility_error.is_some());
        }
    }

    #[test]
    fn new_row_uses_the_section_namespace_after_prefix_rebinding() {
        let mut text = format!(
            r#"<ci:CommandInterface xmlns:ci="{CI_NS}" xmlns:xr="{XR_NS}">
                <alt:SubsystemsVisibility xmlns:ci="urn:other" xmlns:alt="{CI_NS}"/>
                <ci:SubsystemsOrder><ci:Subsystem>Subsystem.Sales</ci:Subsystem></ci:SubsystemsOrder>
            </ci:CommandInterface>"#
        );
        write(
            &mut text,
            &VisibilityUpdate {
                subsystem: "Subsystem.Sales".into(),
                common: Some(false),
                roles: vec![("Role.Reader".into(), Some(true))],
            },
        )
        .unwrap();
        let parsed = crate::configuration::parse_cf_command_interface_xml(&text).unwrap();
        assert_eq!(parsed.subsystem_order, ["Subsystem.Sales"]);
        let rows = parsed.subsystems_visibility.unwrap();
        assert_eq!(rows[0].subsystem, "Subsystem.Sales");
        assert!(!rows[0].common);
        assert!(rows[0].roles[0].visible);
        assert!(text.contains("<alt:Subsystem name=\"Subsystem.Sales\">"));
    }
}
