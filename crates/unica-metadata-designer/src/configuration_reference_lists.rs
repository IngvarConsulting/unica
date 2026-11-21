// module: private

use roxmltree::{Document, Node};
use std::collections::HashSet;
use unica_address::address::QualifiedAddress;
use unica_metadata::descriptor_operations::DescriptorProperty;
use unica_source_designer::owner::MD_CLASSES_NS;
use unica_xml_format::read::{
    child_element_in_namespace, child_elements_in_namespace, direct_text,
};

const XR_NS: &str = "http://v8.1c.ru/8.3/xcf/readable";
const XSI_NS: &str = "http://www.w3.org/2001/XMLSchema-instance";

/// Проверяет вид и форму ссылки. Регистрацию и дескриптор проверяет план правки
/// по текущему состоянию набора исходников.
pub(crate) fn target(
    property: DescriptorProperty,
    raw: &str,
    source_set: &str,
) -> Result<QualifiedAddress, String> {
    let address =
        QualifiedAddress::resolve_input(raw, &[source_set]).map_err(|error| error.to_string())?;
    if address.source_set() == source_set
        && address.line_range().is_none()
        && matches!(address.segments(), [owner] if owner.name().is_some()
            && property.reference_kinds().contains(&owner.kind()))
    {
        Ok(address)
    } else {
        let kinds = property
            .reference_kinds()
            .iter()
            .map(|kind| kind.as_str())
            .collect::<Vec<_>>();
        Err(format!(
            "{} expects an object in source set `{source_set}` of kind {}",
            property.names()[0],
            kinds.join(" or ")
        ))
    }
}

fn list_node<'a, 'input>(
    properties: Node<'a, 'input>,
    property: DescriptorProperty,
) -> Result<Option<Node<'a, 'input>>, String> {
    let name = property.names()[0];
    let matches = child_elements_in_namespace(properties, name, MD_CLASSES_NS);
    match matches.as_slice() {
        [] => Ok(None),
        [node] => Ok(Some(*node)),
        _ => Err(format!("duplicate Configuration property {name}")),
    }
}

fn non_nilled(node: Node<'_, '_>) -> bool {
    node.attribute((XSI_NS, "nil"))
        .is_none_or(|value| matches!(value.trim(), "false" | "0"))
}

pub(crate) fn read(
    properties: Node<'_, '_>,
    property: DescriptorProperty,
) -> Result<Option<Vec<String>>, String> {
    let Some(node) = list_node(properties, property)? else {
        return Ok(None);
    };
    let invalid = || {
        format!(
            "{} contains an invalid metadata reference list",
            property.names()[0]
        )
    };
    if !non_nilled(node) || !direct_text(node).trim().is_empty() {
        return Err(invalid());
    }
    let mut refs = Vec::new();
    let mut seen = HashSet::new();
    for item in node.children().filter(Node::is_element) {
        let valid_type = match item.attribute((XSI_NS, "type")) {
            Some("MDObjectRef") => item.lookup_namespace_uri(None) == Some(XR_NS),
            Some(qualified) => qualified.split_once(':').is_some_and(|(prefix, local)| {
                !prefix.is_empty()
                    && local == "MDObjectRef"
                    && item.lookup_namespace_uri(Some(prefix)) == Some(XR_NS)
            }),
            None => false,
        };
        if item.tag_name().name() != "Item"
            || item.tag_name().namespace() != Some(XR_NS)
            || !valid_type
            || !non_nilled(item)
            || item.children().any(|child| child.is_element())
        {
            return Err(invalid());
        }
        let text = direct_text(item);
        let raw = text.trim();
        let address = target(property, raw, "source").map_err(|_| invalid())?;
        if address.logical_path() != raw || !seen.insert(raw.to_string()) {
            return Err(invalid());
        }
        refs.push(raw.to_string());
    }
    Ok(Some(refs))
}

pub(crate) fn properties<'a, 'input: 'a>(
    doc: &'a Document<'input>,
) -> Result<Node<'a, 'input>, String> {
    let root = doc.root_element();
    let configuration = child_element_in_namespace(root, "Configuration", MD_CLASSES_NS)
        .ok_or("the source set has no Configuration descriptor")?;
    let properties = child_elements_in_namespace(configuration, "Properties", MD_CLASSES_NS);
    match properties.as_slice() {
        [node] => Ok(*node),
        _ => Err("the configuration root must have one Properties element".into()),
    }
}

pub(crate) fn replace(
    source: &str,
    property: DescriptorProperty,
    references: &[String],
) -> Result<String, String> {
    let doc = Document::parse(source).map_err(|error| error.to_string())?;
    let properties = properties(&doc)?;
    let existing = list_node(properties, property)?;
    if read(properties, property)?.as_deref() == Some(references) {
        return Ok(source.to_string());
    }

    let name = property.names()[0];
    let (range, place) = match existing {
        Some(node) => (node.range(), node),
        None => {
            let before = match property {
                DescriptorProperty::AdditionalFullTextSearchDictionaries => "CommonSettingsStorage",
                DescriptorProperty::StandaloneConfigurationRestrictionRoles => {
                    "MobileApplicationURLs"
                }
                _ => return Err(format!("unknown reference list {name}")),
            };
            let place = child_element_in_namespace(properties, before, MD_CLASSES_NS)
                .ok_or_else(|| format!("no {before} property to place {name} before"))?;
            let position = place.range().start;
            (position..position, place)
        }
    };

    let spelling = source[place.range().start + 1..place.range().end]
        .split(|ch: char| ch.is_whitespace() || matches!(ch, '>' | '/'))
        .next()
        .expect("parsed XML element");
    let prefix = spelling.split_once(':').map(|(prefix, _)| prefix);
    let tag = prefix.map_or_else(|| name.to_string(), |prefix| format!("{prefix}:{name}"));
    let mut namespaces = String::new();
    if properties.lookup_namespace_uri(prefix) != Some(MD_CLASSES_NS) {
        match prefix {
            Some(prefix) => namespaces.push_str(&format!(" xmlns:{prefix}=\"{MD_CLASSES_NS}\"")),
            None => namespaces.push_str(&format!(" xmlns=\"{MD_CLASSES_NS}\"")),
        }
    }
    let line_start = source[..range.start]
        .rfind('\n')
        .map_or(0, |index| index + 1);
    let indent = &source[line_start..range.start];
    let indent = if indent.chars().all(|ch| matches!(ch, '\t' | ' ')) {
        indent
    } else {
        ""
    };
    let eol = if source.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let reference_prefix = if prefix == Some("xr") { "ref" } else { "xr" };
    let type_prefix = if prefix == Some("xsi") {
        "schema"
    } else {
        "xsi"
    };
    let mut xml = if references.is_empty() {
        format!("<{tag}{namespaces}/>")
    } else {
        for (prefix, uri) in [(reference_prefix, XR_NS), (type_prefix, XSI_NS)] {
            if properties.lookup_namespace_uri(Some(prefix)) != Some(uri) {
                namespaces.push_str(&format!(" xmlns:{prefix}=\"{uri}\""));
            }
        }
        let mut xml = format!("<{tag}{namespaces}>");
        for item in references {
            let item = unica_xml_format::text::escape_xml_text(item);
            xml.push_str(&format!(
                "{eol}{indent}\t<{reference_prefix}:Item {type_prefix}:type=\"{reference_prefix}:MDObjectRef\">{item}</{reference_prefix}:Item>"
            ));
        }
        xml.push_str(&format!("{eol}{indent}</{tag}>"));
        xml
    };
    if existing.is_none() {
        xml.push_str(eol);
        xml.push_str(indent);
    }
    let mut updated = source.to_string();
    updated.replace_range(range, &xml);
    Ok(updated)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_reference_lists_distinguish_absence_empty_and_damage() {
        let field = DescriptorProperty::StandaloneConfigurationRestrictionRoles;
        let read_source = |body: &str| {
            let source = format!(
                r#"<Properties xmlns="{MD_CLASSES_NS}" xmlns:r="{XR_NS}" xmlns:s="{XSI_NS}">{body}</Properties>"#
            );
            let doc = Document::parse(&source).unwrap();
            read(doc.root_element(), field)
        };
        assert_eq!(read_source("").unwrap(), None);
        assert_eq!(
            read_source("<StandaloneConfigurationRestrictionRoles/>").unwrap(),
            Some(vec![])
        );
        assert!(read_source(r#"<StandaloneConfigurationRestrictionRoles s:nil="true"/>"#).is_err());
        for malformed in [
            r#"<r:Item s:type="r:MDObjectRef">Constant.Probe</r:Item>"#,
            r#"<r:Item s:type="Wrong:MDObjectRef">Role.Probe</r:Item>"#,
            r#"<Item xmlns="http://v8.1c.ru/8.3/xcf/readable" s:type=":MDObjectRef">Role.Probe</Item>"#,
            r#"<r:Item s:type="r:MDObjectRef">Role.Probe<Noise/></r:Item>"#,
        ] {
            assert!(read_source(&format!(
                "<StandaloneConfigurationRestrictionRoles>{malformed}</StandaloneConfigurationRestrictionRoles>"
            )).is_err());
        }
    }
}
