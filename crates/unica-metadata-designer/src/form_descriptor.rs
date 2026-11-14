//! `props.set` of a form descriptor: Comment, IncludeHelpInContents,
//! UsePurposes and UseInInterfaceCompatibilityMode, in the places and the
//! spelling the platform export writes them.

use crate::xml_model::MD_CLASSES_NS;
use roxmltree::{Document, Node};
use unica_address::address::NodeKind;
use unica_metadata::descriptor_operations::DescriptorProperty;
use unica_metadata::property_input::DescriptorPropertyValue;

const V8_CORE_NS: &str = "http://v8.1c.ru/8.1/data/core";
const XSI_NS: &str = "http://www.w3.org/2001/XMLSchema-instance";
const APP_NS: &str = "http://v8.1c.ru/8.2/managed-application/core";

/// Writes the given properties of the form descriptor `source` of `kind`
/// (`CommonForm` or `Form`) named `name`. A value equal to the present one
/// keeps every byte; an absent property is written after the property the
/// platform writes before it.
pub(crate) fn set_form_descriptor_properties(
    source: &str,
    kind: NodeKind,
    name: &str,
    changes: &[(DescriptorProperty, DescriptorPropertyValue)],
) -> Result<String, String> {
    crate::descriptor_comment::comment_patch_capability(source, kind, name)?;
    let mut text = source.to_string();
    for (property, value) in changes {
        text = match (property, value) {
            (DescriptorProperty::Comment, DescriptorPropertyValue::Text(comment)) => {
                crate::descriptor_comment::set_comment(&text, kind, name, comment)?
            }
            (DescriptorProperty::IncludeHelpInContents, DescriptorPropertyValue::Boolean(flag)) => {
                set_scalar(
                    &text,
                    kind,
                    "IncludeHelpInContents",
                    if *flag { "true" } else { "false" },
                    &["FormType"],
                )?
            }
            (
                DescriptorProperty::UseInInterfaceCompatibilityMode,
                DescriptorPropertyValue::Text(mode),
            ) => set_scalar(
                &text,
                kind,
                "UseInInterfaceCompatibilityMode",
                mode,
                &["UsePurposes", "IncludeHelpInContents", "FormType"],
            )?,
            (DescriptorProperty::UsePurposes, DescriptorPropertyValue::List(purposes)) => {
                set_purposes(&text, kind, purposes)?
            }
            (property, _) => {
                return Err(format!(
                    "form descriptor property `{}` has no writer",
                    property.names()[0]
                ))
            }
        };
    }
    verify(&text, kind, changes)?;
    Ok(text)
}

fn properties<'a, 'input>(
    document: &'a Document<'input>,
    kind: NodeKind,
) -> Result<Node<'a, 'input>, String> {
    document
        .root_element()
        .children()
        .find(|node| node.is_element())
        .filter(|owner| owner.has_tag_name((MD_CLASSES_NS, kind.as_str())))
        .and_then(|owner| {
            owner
                .children()
                .find(|node| node.has_tag_name((MD_CLASSES_NS, "Properties")))
        })
        .ok_or_else(|| format!("descriptor owner is not {}", kind.as_str()))
}

fn property<'a, 'input>(properties: Node<'a, 'input>, tag: &str) -> Option<Node<'a, 'input>> {
    properties
        .children()
        .find(|node| node.has_tag_name((MD_CLASSES_NS, tag)))
}

/// The qualified name the source spells the element with.
fn spelling<'t>(text: &'t str, node: Node<'_, '_>) -> &'t str {
    let open = &text[node.range().start + 1..];
    &open[..open
        .find(|ch: char| ch.is_whitespace() || ch == '>' || ch == '/')
        .unwrap_or(open.len())]
}

/// The line break and indentation in front of `node`.
fn layout(text: &str, node: Node<'_, '_>) -> (&'static str, String) {
    let before = &text[..node.range().start];
    let line_start = before.rfind('\n').map_or(0, |index| index + 1);
    let indent = before[line_start..]
        .chars()
        .take_while(|ch| *ch == '\t' || *ch == ' ')
        .collect();
    let eol = if before[..line_start].ends_with("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    (eol, indent)
}

/// The spelling of `tag` in the namespace prefix of `like`.
fn sibling_spelling(like: &str, tag: &str) -> String {
    match like.split_once(':') {
        Some((prefix, _)) => format!("{prefix}:{tag}"),
        None => tag.to_string(),
    }
}

fn set_scalar(
    text: &str,
    kind: NodeKind,
    tag: &str,
    value: &str,
    anchors: &[&str],
) -> Result<String, String> {
    let document = Document::parse(text).map_err(|error| error.to_string())?;
    let properties = properties(&document, kind)?;
    let mut updated = text.to_string();
    if let Some(node) = property(properties, tag) {
        let present = node.children().all(|child| !child.is_element())
            && unica_xml_format::read::direct_text(node) == value;
        if present {
            return Ok(updated);
        }
        let name = spelling(text, node).to_string();
        updated.replace_range(node.range(), &format!("<{name}>{value}</{name}>"));
        return Ok(updated);
    }
    let anchor = anchors
        .iter()
        .find_map(|anchor| property(properties, anchor))
        .ok_or_else(|| {
            format!(
                "form descriptor has no `{}` to place `{tag}` after",
                anchors[anchors.len() - 1]
            )
        })?;
    let (eol, indent) = layout(text, anchor);
    let name = sibling_spelling(spelling(text, anchor), tag);
    updated.insert_str(
        anchor.range().end,
        &format!("{eol}{indent}<{name}>{value}</{name}>"),
    );
    Ok(updated)
}

fn observed_purposes(node: Node<'_, '_>) -> Vec<String> {
    node.children()
        .filter(|child| child.has_tag_name((V8_CORE_NS, "Value")))
        .map(|child| unica_xml_format::read::direct_text(child).to_string())
        .collect()
}

fn set_purposes(text: &str, kind: NodeKind, purposes: &[String]) -> Result<String, String> {
    let document = Document::parse(text).map_err(|error| error.to_string())?;
    let properties = properties(&document, kind)?;
    let existing = property(properties, "UsePurposes");
    if let Some(node) = existing {
        let mut present = observed_purposes(node);
        let mut requested = purposes.to_vec();
        present.sort();
        requested.sort();
        if present == requested
            && node.children().filter(|child| child.is_element()).count() == purposes.len()
        {
            return Ok(text.to_string());
        }
    }
    let prefix = |namespace: &str| {
        properties.lookup_prefix(namespace).ok_or_else(|| {
            format!("form descriptor declares no prefix for `{namespace}`; re-export it with the platform before editing UsePurposes")
        })
    };
    // The platform writes the purposes in the order of its enumeration.
    let ordered = DescriptorProperty::UsePurposes
        .allowed_values()
        .iter()
        .filter(|value| purposes.iter().any(|purpose| purpose == *value))
        .collect::<Vec<_>>();
    let place = match existing {
        Some(node) => node,
        None => ["IncludeHelpInContents", "FormType"]
            .iter()
            .find_map(|anchor| property(properties, anchor))
            .ok_or_else(|| {
                "form descriptor has no `FormType` to place `UsePurposes` after".to_string()
            })?,
    };
    let (eol, indent) = layout(text, place);
    let name = match existing {
        Some(node) => spelling(text, node).to_string(),
        None => sibling_spelling(spelling(text, place), "UsePurposes"),
    };
    let block = if ordered.is_empty() {
        format!("<{name}/>")
    } else {
        let (v8, xsi, app) = (prefix(V8_CORE_NS)?, prefix(XSI_NS)?, prefix(APP_NS)?);
        let mut block = format!("<{name}>");
        for purpose in ordered {
            block.push_str(&format!(
                "{eol}{indent}\t<{v8}:Value {xsi}:type=\"{app}:ApplicationUsePurpose\">{purpose}</{v8}:Value>"
            ));
        }
        block.push_str(&format!("{eol}{indent}</{name}>"));
        block
    };
    let mut updated = text.to_string();
    match existing {
        Some(node) => updated.replace_range(node.range(), &block),
        None => updated.insert_str(place.range().end, &format!("{eol}{indent}{block}")),
    }
    Ok(updated)
}

/// The written descriptor reads back the requested values.
fn verify(
    text: &str,
    kind: NodeKind,
    changes: &[(DescriptorProperty, DescriptorPropertyValue)],
) -> Result<(), String> {
    let document = Document::parse(text)
        .map_err(|error| format!("form descriptor postimage is not valid XML: {error}"))?;
    let properties = properties(&document, kind)?;
    for (field, value) in changes {
        let tag = field.names()[0];
        let node = property(properties, tag);
        let matches = match value {
            DescriptorPropertyValue::Text(expected) if *field == DescriptorProperty::Comment => {
                crate::info_projection::observe_collection_item_comment(properties)
                    .ok()
                    .flatten()
                    .as_deref()
                    == Some(expected.as_str())
            }
            DescriptorPropertyValue::Text(expected) => {
                node.is_some_and(|node| unica_xml_format::read::direct_text(node) == *expected)
            }
            DescriptorPropertyValue::Boolean(expected) => node.is_some_and(|node| {
                unica_xml_format::read::direct_text(node)
                    == if *expected { "true" } else { "false" }
            }),
            DescriptorPropertyValue::List(expected) => node.is_some_and(|node| {
                let mut observed = observed_purposes(node);
                let mut expected = expected.clone();
                observed.sort();
                expected.sort();
                observed == expected
            }),
            DescriptorPropertyValue::Parts(_) => false,
        };
        if !matches {
            return Err(format!(
                "form descriptor postimage does not contain the requested `{tag}`"
            ));
        }
    }
    Ok(())
}

/// Whether the owner descriptor `owner` registers the form `name` among its
/// child objects.
pub(crate) fn owner_registers_form(owner: &[u8], name: &str) -> bool {
    let Ok(text) = std::str::from_utf8(owner) else {
        return false;
    };
    let Ok(document) = Document::parse(text.trim_start_matches('\u{feff}')) else {
        return false;
    };
    document
        .root_element()
        .children()
        .find(|node| node.is_element())
        .into_iter()
        .flat_map(|artifact| artifact.children())
        .filter(|node| node.has_tag_name((MD_CLASSES_NS, "ChildObjects")))
        .flat_map(|children| children.children())
        .any(|child| {
            child.has_tag_name((MD_CLASSES_NS, "Form"))
                && unica_xml_format::read::direct_text(child) == name
        })
}

/// The form descriptor `text` with `IncludeHelpInContents` set to `false`
/// after `FormType`, the place the platform writes it; `None` when the
/// descriptor already has it or is not a form descriptor with a type.
pub(crate) fn with_include_help(text: &str) -> Option<String> {
    let document = Document::parse(text).ok()?;
    let properties = properties(&document, NodeKind::Form).ok()?;
    if property(properties, "IncludeHelpInContents").is_some() {
        return None;
    }
    set_scalar(
        text,
        NodeKind::Form,
        "IncludeHelpInContents",
        "false",
        &["FormType"],
    )
    .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIST_FORM: &str = include_str!(
        "../../../tests/fixtures/formats/platform_xml/2.21/all_kinds/src/Catalogs/Валюты/Forms/ФормаСписка.xml"
    );

    fn export() -> &'static str {
        LIST_FORM.trim_start_matches('\u{feff}')
    }

    fn without(text: &str, line: &str) -> String {
        let removed = text.replacen(line, "", 1);
        assert_ne!(removed, text, "{line}");
        removed
    }

    fn set(text: &str, changes: &[(DescriptorProperty, DescriptorPropertyValue)]) -> String {
        set_form_descriptor_properties(text, NodeKind::Form, "ФормаСписка", changes).unwrap()
    }

    #[test]
    fn help_add_and_props_set_insert_the_help_flag_with_the_same_bytes() {
        for eol in ["\r\n", "\n"] {
            let full = export().replace("\r\n", eol);
            let bare = without(
                &full,
                &format!("{eol}\t\t\t<IncludeHelpInContents>false</IncludeHelpInContents>"),
            );
            let by_help = crate::help::form_with_include_help(&bare).unwrap();
            let by_props = set(
                &bare,
                &[(
                    DescriptorProperty::IncludeHelpInContents,
                    DescriptorPropertyValue::Boolean(false),
                )],
            );
            assert_eq!(by_help, by_props);
            // Both restore the platform export byte for byte.
            assert_eq!(by_props, full);
            assert_eq!(crate::help::form_with_include_help(&full), None);
        }
    }

    #[test]
    fn absent_properties_are_written_where_the_platform_writes_them() {
        let full = export();
        let mut bare = full.to_string();
        for line in [
            "\r\n\t\t\t<IncludeHelpInContents>false</IncludeHelpInContents>",
            "\r\n\t\t\t<UseInInterfaceCompatibilityMode>Any</UseInInterfaceCompatibilityMode>",
        ] {
            bare = without(&bare, line);
        }
        let start = bare.find("\r\n\t\t\t<UsePurposes>").unwrap();
        let end = bare.find("</UsePurposes>").unwrap() + "</UsePurposes>".len();
        bare.replace_range(start..end, "");
        let restored = set(
            &bare,
            &[
                (
                    DescriptorProperty::IncludeHelpInContents,
                    DescriptorPropertyValue::Boolean(false),
                ),
                (
                    DescriptorProperty::UsePurposes,
                    DescriptorPropertyValue::List(vec![
                        "MobilePlatformApplication".into(),
                        "PlatformApplication".into(),
                    ]),
                ),
                (
                    DescriptorProperty::UseInInterfaceCompatibilityMode,
                    DescriptorPropertyValue::Text("Any".into()),
                ),
            ],
        );
        assert_eq!(restored, full);
    }

    #[test]
    fn an_equal_value_keeps_every_byte() {
        let full = export();
        let same = set(
            full,
            &[
                (
                    DescriptorProperty::Comment,
                    DescriptorPropertyValue::Text(String::new()),
                ),
                (
                    DescriptorProperty::IncludeHelpInContents,
                    DescriptorPropertyValue::Boolean(false),
                ),
                (
                    DescriptorProperty::UsePurposes,
                    DescriptorPropertyValue::List(vec![
                        "MobilePlatformApplication".into(),
                        "PlatformApplication".into(),
                    ]),
                ),
                (
                    DescriptorProperty::UseInInterfaceCompatibilityMode,
                    DescriptorPropertyValue::Text("Any".into()),
                ),
            ],
        );
        assert_eq!(same, full);
    }

    #[test]
    fn purposes_are_written_in_the_platform_order_and_cleared_to_an_empty_element() {
        let full = export();
        let mobile = set(
            full,
            &[(
                DescriptorProperty::UsePurposes,
                DescriptorPropertyValue::List(vec!["MobilePlatformApplication".into()]),
            )],
        );
        assert_eq!(
            mobile,
            without(full, "\r\n\t\t\t\t<v8:Value xsi:type=\"app:ApplicationUsePurpose\">PlatformApplication</v8:Value>")
        );
        let none = set(
            full,
            &[(
                DescriptorProperty::UsePurposes,
                DescriptorPropertyValue::List(Vec::new()),
            )],
        );
        let start = full.find("<UsePurposes>").unwrap();
        let end = full.find("</UsePurposes>").unwrap() + "</UsePurposes>".len();
        let mut expected = full.to_string();
        expected.replace_range(start..end, "<UsePurposes/>");
        assert_eq!(none, expected);
        let back = set(
            &none,
            &[(
                DescriptorProperty::UsePurposes,
                DescriptorPropertyValue::List(vec![
                    "MobilePlatformApplication".into(),
                    "PlatformApplication".into(),
                ]),
            )],
        );
        assert_eq!(back, full);
    }

    #[test]
    fn purposes_need_the_namespaces_the_platform_writes_them_with() {
        let full = export();
        let start = full.find("<UsePurposes>").unwrap();
        let end = full.find("</UsePurposes>").unwrap() + "</UsePurposes>".len();
        let mut bare = full.to_string();
        bare.replace_range(start..end, "<UsePurposes/>");
        let bare = without(
            &bare,
            " xmlns:app=\"http://v8.1c.ru/8.2/managed-application/core\"",
        );
        let error = set_form_descriptor_properties(
            &bare,
            NodeKind::Form,
            "ФормаСписка",
            &[(
                DescriptorProperty::UsePurposes,
                DescriptorPropertyValue::List(vec!["PlatformApplication".into()]),
            )],
        )
        .unwrap_err();
        assert_eq!(
            error,
            "form descriptor declares no prefix for `http://v8.1c.ru/8.2/managed-application/core`; re-export it with the platform before editing UsePurposes"
        );
    }

    #[test]
    fn a_descriptor_of_another_form_is_refused() {
        let error = set_form_descriptor_properties(
            export(),
            NodeKind::Form,
            "ФормаЭлемента",
            &[(
                DescriptorProperty::IncludeHelpInContents,
                DescriptorPropertyValue::Boolean(true),
            )],
        )
        .unwrap_err();
        assert_eq!(error, "descriptor Name does not match the addressed owner");
    }
}
