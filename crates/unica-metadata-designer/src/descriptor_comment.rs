//! Local edits of the root descriptor's canonical developer comment.

use roxmltree::{Document, Node};
use unica_address::address::NodeKind;
use unica_source_designer::owner::MD_CLASSES_NS;

fn direct<'a, 'input>(parent: Node<'a, 'input>, tag: &str) -> Result<Node<'a, 'input>, String> {
    let nodes = parent
        .children()
        .filter(|node| node.is_element() && node.tag_name().name() == tag)
        .collect::<Vec<_>>();
    match nodes.as_slice() {
        [node] if node.tag_name().namespace() == Some(MD_CLASSES_NS) => Ok(*node),
        _ => Err(format!("descriptor requires one canonical {tag}")),
    }
}

fn descriptor_properties<'a, 'input>(
    document: &'a Document<'input>,
    kind: &str,
) -> Result<Node<'a, 'input>, String> {
    let root = document.root_element();
    if !root.has_tag_name((MD_CLASSES_NS, "MetaDataObject")) {
        return Err("descriptor requires the metadata MetaDataObject root".into());
    }
    let owners = root.children().filter(Node::is_element).collect::<Vec<_>>();
    let [owner] = owners.as_slice() else {
        return Err("descriptor requires exactly one root owner".into());
    };
    if !owner.has_tag_name((MD_CLASSES_NS, kind)) {
        return Err(format!("descriptor owner is not {kind}"));
    }
    direct(*owner, "Properties")
}

/// A comment is an independent read fact; absence does not hide the owner.
/// Reading does not require the writer's exact export format or property profile.
pub fn read_comment(bytes: &[u8], kind: &str) -> Result<Option<String>, String> {
    let text = std::str::from_utf8(bytes).map_err(|error| error.to_string())?;
    let document =
        Document::parse(text.trim_start_matches('\u{feff}')).map_err(|error| error.to_string())?;
    let properties = descriptor_properties(&document, kind)?;
    crate::info_projection::observe_collection_item_comment(properties)
        .map_err(|error| error.message)
}

fn writable_comment<'a, 'input>(
    source: &str,
    document: &'a Document<'input>,
    kind: NodeKind,
    name: &str,
) -> Result<(Node<'a, 'input>, String), String> {
    let root = document.root_element();
    if !root.has_tag_name((MD_CLASSES_NS, "MetaDataObject")) {
        return Err("descriptor requires the metadata MetaDataObject root".into());
    }
    let version = unica_source_designer::roots::root_version_literal(source, root);
    if let Some(refusal) = unica_source_designer::format_profile::writable_profile_refusal(
        format!("{}.{}.xml", kind.as_str(), name),
        version.as_deref(),
    ) {
        return Err(refusal.message);
    }
    let properties = descriptor_properties(document, kind.as_str())?;
    // Legacy inline form/template descriptions are not valid registrations
    // in the writable profile. Refuse instead of letting global legacy repair
    // rewrite unrelated bytes after this comment-only patch.
    if properties
        .parent()
        .into_iter()
        .flat_map(|owner| owner.children())
        .filter(|node| node.is_element() && node.tag_name().name() == "ChildObjects")
        .flat_map(|children| children.children())
        .any(|child| {
            child.is_element()
                && matches!(child.tag_name().name(), "Form" | "Template")
                && child.children().any(|node| node.is_element())
        })
    {
        return Err("descriptor has an inline form or template registration; re-export it with the platform before editing Comment".into());
    }
    let owner_name = direct(properties, "Name")?;
    if owner_name.children().any(|node| node.is_element())
        || unica_xml_format::read::direct_text(owner_name) != name
    {
        return Err("descriptor Name does not match the addressed owner".into());
    }
    let comment = direct(properties, "Comment")?;
    let previous = crate::info_projection::observe_collection_item_comment(properties)
        .map_err(|error| error.message)?
        .ok_or_else(|| "descriptor requires a canonical Comment".to_string())?;
    Ok((comment, previous))
}

/// Prove the Comment writer's preconditions without changing any source bytes.
pub fn comment_patch_capability(source: &str, kind: NodeKind, name: &str) -> Result<(), String> {
    let document = Document::parse(source).map_err(|error| error.to_string())?;
    writable_comment(source, &document, kind, name).map(|_| ())
}

pub(crate) fn set_comment(
    source: &str,
    kind: NodeKind,
    name: &str,
    value: &str,
) -> Result<String, String> {
    let document = Document::parse(source).map_err(|error| error.to_string())?;
    let (comment, previous) = writable_comment(source, &document, kind, name)?;
    if previous == value {
        return Ok(source.to_string());
    }
    let spelling = &source[comment.range().start + 1..];
    let spelling = &spelling[..spelling
        .find(|ch: char| ch.is_whitespace() || ch == '/' || ch == '>')
        .ok_or_else(|| "Comment start tag is malformed".to_string())?];
    // Character references retain CR even across the XML line-ending normalization.
    // Encoding line controls also avoids changing the descriptor's newline profile.
    let escaped = unica_xml_format::text::escape_xml(value)
        .replace('\r', "&#13;")
        .replace('\n', "&#10;")
        .replace('\t', "&#9;");
    let replacement = if value.is_empty() {
        format!("<{spelling}/>")
    } else {
        format!("<{spelling}>{escaped}</{spelling}>")
    };
    let mut updated = source.to_string();
    updated.replace_range(comment.range(), &replacement);
    Document::parse(&updated).map_err(|error| format!("Comment is not valid XML text: {error}"))?;
    Ok(updated)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn descriptor(comment: &str) -> String {
        format!("<m:MetaDataObject xmlns:m=\"{MD_CLASSES_NS}\" version=\"2.20\">\r\n<m:SessionParameter uuid=\"00000000-0000-4000-8000-000000000001\"><m:Properties>\n<m:Name>Probe</m:Name><m:Synonym><keep/></m:Synonym>{comment}<m:Type/></m:Properties>\r\n<m:ChildObjects><m:Comment>child</m:Comment></m:ChildObjects></m:SessionParameter></m:MetaDataObject>")
    }

    #[test]
    fn comment_patch_is_local_and_equal_decoded_text_is_byte_exact() {
        for old in ["<m:Comment />", "<m:Comment>old</m:Comment>"] {
            let source = descriptor(old);
            for value in ["Кириллица & < > \"\n\r\t", "replacement", ""] {
                let updated =
                    set_comment(&source, NodeKind::SessionParameter, "Probe", value).unwrap();
                let escaped = unica_xml_format::text::escape_xml(value)
                    .replace('\r', "&#13;")
                    .replace('\n', "&#10;")
                    .replace('\t', "&#9;");
                let expected = if value.is_empty() {
                    "<m:Comment/>".to_string()
                } else {
                    format!("<m:Comment>{escaped}</m:Comment>")
                };
                if old == "<m:Comment />" && value.is_empty() {
                    assert_eq!(updated, source);
                } else {
                    assert_eq!(updated, source.replacen(old, &expected, 1));
                }
                assert_eq!(
                    set_comment(&updated, NodeKind::SessionParameter, "Probe", value).unwrap(),
                    updated
                );
                let doc = Document::parse(&updated).unwrap();
                let props = doc
                    .descendants()
                    .find(|n| n.has_tag_name((MD_CLASSES_NS, "Properties")))
                    .unwrap();
                assert_eq!(
                    crate::info_projection::observe_collection_item_comment(props)
                        .unwrap()
                        .as_deref(),
                    Some(value)
                );
            }
        }
        let source = descriptor("<m:Comment>&#65;<![CDATA[ &]]><!--keep-->amp;</m:Comment>");
        assert_eq!(
            set_comment(&source, NodeKind::SessionParameter, "Probe", "A &amp;").unwrap(),
            source
        );
    }

    #[test]
    fn comment_patch_refuses_ambiguous_or_damaged_state_and_invalid_xml_text() {
        for raw in [
            "",
            "<m:Comment/><m:Comment/>",
            "<m:Comment><nested/></m:Comment>",
            "<Comment xmlns=\"foreign\">bad</Comment>",
            "<m:Comment strange=\"x\"/>",
        ] {
            assert!(
                set_comment(
                    &descriptor(raw),
                    NodeKind::SessionParameter,
                    "Probe",
                    "text"
                )
                .is_err(),
                "{raw}"
            );
        }
        let source = descriptor("<m:Comment/>");
        for damaged in [
            source.replace("2.20", "2.21"),
            source.replace("2.20", "2.&#50;0"),
            source.replace("Probe", "Other"),
            source.replace("m:SessionParameter", "m:CommonAttribute"),
            source.replace("<m:Properties>", "<m:Properties/><m:Properties>"),
        ] {
            assert!(set_comment(&damaged, NodeKind::SessionParameter, "Probe", "text").is_err());
        }
        for value in ["bad\0text", "bad\u{1}text", "bad\u{fffe}text"] {
            assert!(set_comment(&source, NodeKind::SessionParameter, "Probe", value).is_err());
        }
    }
}
