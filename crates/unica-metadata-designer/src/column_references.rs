//! Declared journal-column sources and strict inline target identity.
use crate::info_projection::{
    direct_text_content, node_has_only_whitespace_text, qname_resolves_to,
    strict_text_leaf_is_valid,
};
use roxmltree::{Document, Node};
use unica_address::source_target::{MetadataAddress, PLATFORM_XML_8_3_27_FORMAT_2_20};
use unica_metadata::{ColumnAttributeReference, ColumnReferenceFacts};

const MD: &str = "http://v8.1c.ru/8.3/MDClasses";
const XR: &str = "http://v8.1c.ru/8.3/xcf/readable";
const XSI: &str = "http://www.w3.org/2001/XMLSchema-instance";

fn one_direct<'a, 'input>(
    node: Node<'a, 'input>,
    name: &str,
) -> Result<Option<Node<'a, 'input>>, String> {
    let nodes = node
        .children()
        .filter(|child| child.is_element() && child.tag_name().name() == name)
        .collect::<Vec<_>>();
    match nodes.as_slice() {
        [] => Ok(None),
        [node] if node.tag_name().namespace() == Some(MD) => Ok(Some(*node)),
        _ => Err(format!("{name} is duplicated or uses a foreign namespace")),
    }
}
fn properties<'a, 'input>(node: Node<'a, 'input>) -> Result<Node<'a, 'input>, String> {
    one_direct(node, "Properties")?.ok_or_else(|| "Properties is absent".to_string())
}
fn identity(properties: Node<'_, '_>) -> Result<String, String> {
    let node = one_direct(properties, "Name")?.ok_or_else(|| "Name is absent".to_string())?;
    let name = direct_text_content(node);
    if !strict_text_leaf_is_valid(node) || !unica_metadata::identifier::is_1c_identifier(&name) {
        return Err("Name is not one direct identifier".to_string());
    }
    Ok(name)
}

pub fn read(node: Node<'_, '_>) -> ColumnReferenceFacts {
    match read_column(node) {
        Ok(facts) => facts,
        Err(reason) => ColumnReferenceFacts {
            targets: None,
            limits: vec![format!("References: {reason}")],
        },
    }
}
fn read_column(node: Node<'_, '_>) -> Result<ColumnReferenceFacts, String> {
    if !node.has_tag_name((MD, "Column")) {
        return Err("column identity is outside the read profile".to_string());
    }
    let properties = properties(node)?;
    identity(properties)?;
    let Some(container) = one_direct(properties, "References")? else {
        return Ok(ColumnReferenceFacts {
            targets: None,
            limits: vec!["References is absent; completeness is unknown".to_string()],
        });
    };
    let mut limits = Vec::new();
    if container.attributes().len() != 0 || !node_has_only_whitespace_text(container) {
        limits.push(
            "References has unsupported attributes or mixed text; completeness is unknown"
                .to_string(),
        );
    }
    let mut targets = Vec::new();
    for (index, item) in container.children().filter(Node::is_element).enumerate() {
        let valid = item.has_tag_name((XR, "Item"))
            && item.attributes().len() == 1
            && !item.children().any(|child| child.is_element())
            && item
                .attribute((XSI, "type"))
                .is_some_and(|value| qname_resolves_to(item, value, XR, "MDObjectRef"));
        if !valid {
            limits.push(format!(
                "References[{index}] is not a scalar xr:MDObjectRef item"
            ));
            continue;
        }
        let raw = direct_text_content(item);
        let parts = raw.trim().split('.').collect::<Vec<_>>();
        let target = match parts.as_slice() {
            ["Document", name, "Attribute", attribute]
                if unica_metadata::identifier::is_1c_identifier(name)
                    && unica_metadata::identifier::is_1c_identifier(attribute) =>
            {
                MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, &format!("Document.{name}"))
                    .ok()
                    .map(|owner| ColumnAttributeReference {
                        owner,
                        attribute: (*attribute).to_string(),
                    })
            }
            _ => None,
        };
        match target {
            Some(target) => targets.push(target),
            None => limits.push(format!(
                "References[{index}] is outside the Document.Attribute read profile"
            )),
        }
    }
    Ok(ColumnReferenceFacts {
        targets: Some(targets),
        limits,
    })
}

/// Proves only the directly declared Attribute, without reading its Type or companions.
pub fn prove_attribute_target(
    bytes: &[u8],
    target: &ColumnAttributeReference,
) -> Result<(), String> {
    let parts = target.owner.segments().collect::<Vec<_>>();
    let ["Document", owner_name] = parts.as_slice() else {
        return Err("reference owner is outside the Document read profile".to_string());
    };
    let attribute_name = &target.attribute;
    let text = std::str::from_utf8(bytes).map_err(|error| error.to_string())?;
    let document =
        Document::parse(text.trim_start_matches('\u{feff}')).map_err(|error| error.to_string())?;
    let root = document.root_element();
    if !root.has_tag_name((MD, "MetaDataObject")) {
        return Err("document descriptor has the wrong root".to_string());
    }
    let owners = root.children().filter(Node::is_element).collect::<Vec<_>>();
    let [owner] = owners.as_slice() else {
        return Err("document descriptor has no unique owner".to_string());
    };
    if !owner.has_tag_name((MD, "Document")) || identity(properties(*owner)?)? != *owner_name {
        return Err("document descriptor identity does not match the reference".to_string());
    }
    let children = one_direct(*owner, "ChildObjects")?
        .ok_or_else(|| "document child declarations are unavailable".to_string())?;
    let mut matches = 0;
    for attribute in children
        .children()
        .filter(|node| node.is_element() && node.tag_name().name() == "Attribute")
    {
        if !attribute.has_tag_name((MD, "Attribute")) {
            return Err("attribute identity uses a foreign namespace".to_string());
        }
        if identity(properties(attribute)?)? == *attribute_name {
            matches += 1;
        }
    }
    if matches != 1 {
        return Err("reference has no unique directly declared Attribute target".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_column_references_resolve_qnames_and_preserve_partial_presence() {
        let xml = format!(
            r#"<Column xmlns="{MD}" xmlns:r="{XR}" xmlns:i="{XSI}"><Properties><Name>Status</Name><References><r:Item i:type="r:MDObjectRef">Document.Sale.Attribute.Status</r:Item><r:Item i:type="r:Unknown">Document.Return.Attribute.Status</r:Item></References></Properties></Column>"#
        );
        let doc = Document::parse(&xml).unwrap();
        let facts = read(doc.root_element());
        assert_eq!(
            facts
                .targets
                .unwrap()
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            ["Document.Sale.Attribute.Status"]
        );
        assert_eq!(facts.limits.len(), 1);
        for (value, known) in [
            ("<References/>", true),
            ("", false),
            ("<References/><References/>", false),
        ] {
            let xml=format!("<Column xmlns=\"{MD}\"><Properties><Name>Status</Name>{value}</Properties></Column>");
            let doc = Document::parse(&xml).unwrap();
            let facts = read(doc.root_element());
            assert_eq!(facts.targets.is_some(), known);
            assert_eq!(facts.limits.is_empty(), known);
        }
    }
}
