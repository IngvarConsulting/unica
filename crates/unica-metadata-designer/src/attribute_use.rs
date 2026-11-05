//! Existing header AttributeUse: replace only the observed literal, never insert it.
use roxmltree::{Document, Node};
use std::ops::Range;
use unica_metadata::{MetaDiagnostic, MetaDiagnosticCode, ATTRIBUTE_USE_VALUES};

const MD: &str = "http://v8.1c.ru/8.3/MDClasses";

fn bad(message: &str) -> MetaDiagnostic {
    MetaDiagnostic::error(MetaDiagnosticCode::InvalidArguments, message).with_field("use")
}

fn child<'a, 'i>(node: Node<'a, 'i>, name: &str) -> Result<Node<'a, 'i>, MetaDiagnostic> {
    let mut found = node
        .children()
        .filter(|n| n.is_element() && n.tag_name().name() == name);
    let value = found
        .next()
        .ok_or_else(|| bad(&format!("{name} is missing; no default is assumed")))?;
    if found.next().is_some() || !value.has_tag_name((MD, name)) || value.attributes().len() != 0 {
        return Err(bad(&format!("{name} must be one metadata property")));
    }
    Ok(value)
}

fn observed<'a>(
    xml: &'a str,
    attribute: Node<'_, '_>,
) -> Result<(&'a str, Range<usize>), MetaDiagnostic> {
    let property = child(child(attribute, "Properties")?, "Use")?;
    let mut children = property.children();
    let text = children
        .next()
        .filter(Node::is_text)
        .ok_or_else(|| bad("Use requires a literal AttributeUse value"))?;
    if children.next().is_some() {
        return Err(bad("Use requires a single literal AttributeUse value"));
    }
    let raw = &xml[text.range()];
    let value = raw.trim_matches([' ', '\t', '\r', '\n']);
    if !ATTRIBUTE_USE_VALUES.contains(&value) {
        return Err(bad(
            "Use must contain ForItem, ForFolder or ForFolderAndItem",
        ));
    }
    let start =
        text.range().start + raw.len() - raw.trim_start_matches([' ', '\t', '\r', '\n']).len();
    Ok((value, start..start + value.len()))
}

pub(crate) fn edit(
    xml: &str,
    selected: Range<usize>,
    value: &str,
) -> Result<Option<(Range<usize>, String)>, MetaDiagnostic> {
    if !ATTRIBUTE_USE_VALUES.contains(&value) {
        return Err(bad("use must be ForItem, ForFolder or ForFolderAndItem"));
    }
    let doc = Document::parse(xml).map_err(|_| bad("invalid metadata XML"))?;
    let root = doc.root_element();
    if !root.has_tag_name((MD, "MetaDataObject"))
        || unica_source_designer::roots::root_version_literal(xml, root).as_deref() != Some("2.20")
    {
        return Err(bad("use requires metadata XML format 2.20"));
    }
    let owners: Vec<_> = root.children().filter(Node::is_element).collect();
    let [owner] = owners.as_slice() else {
        return Err(bad("expected one metadata owner"));
    };
    if !(owner.has_tag_name((MD, "Catalog"))
        || owner.has_tag_name((MD, "ChartOfCharacteristicTypes")))
    {
        return Err(bad("use requires Catalog or ChartOfCharacteristicTypes"));
    }
    let attribute = child(*owner, "ChildObjects")?
        .children()
        .find(|node| node.range() == selected && node.has_tag_name((MD, "Attribute")))
        .ok_or_else(|| bad("use requires a header attribute"))?;
    let (previous, range) = observed(xml, attribute)?;
    Ok((previous != value).then(|| (range, value.to_owned())))
}

pub(crate) fn effect(
    xml: &str,
    object: Node<'_, '_>,
    mut value: serde_json::Value,
    selected: &[&str],
) -> Result<serde_json::Value, MetaDiagnostic> {
    let parent = child(object, "ChildObjects")?;
    for item in value
        .as_array_mut()
        .ok_or_else(|| bad("attribute effect must be an array"))?
    {
        let name = item["name"]
            .as_str()
            .ok_or_else(|| bad("attribute effect has no name"))?;
        if !selected
            .iter()
            .any(|selected| unica_metadata::metadata_name_eq(name, selected))
        {
            continue;
        }
        let attribute = parent
            .children()
            .filter(|n| n.has_tag_name((MD, "Attribute")))
            .find(|n| {
                child(*n, "Properties")
                    .and_then(|p| child(p, "Name"))
                    .is_ok_and(|n| {
                        n.text()
                            .is_some_and(|v| unica_metadata::metadata_name_eq(v, name))
                    })
            })
            .ok_or_else(|| bad("attribute effect target is unavailable"))?;
        item["Use"] = serde_json::Value::String(observed(xml, attribute)?.0.into());
    }
    Ok(value)
}

#[cfg(test)]
mod tests;
