//! Strict existing TypeReductionMode mutation and its semantic evidence.
use roxmltree::{Document, Node};
use unica_metadata::{MetaDiagnostic, MetaDiagnosticCode};
const MD: &str = "http://v8.1c.ru/8.3/MDClasses";
fn bad(message: &str) -> MetaDiagnostic {
    MetaDiagnostic::error(MetaDiagnosticCode::InvalidArguments, message)
        .with_field("typeReductionMode")
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
fn literal<'a>(
    xml: &'a str,
    node: Node<'_, '_>,
) -> Result<(&'a str, std::ops::Range<usize>), MetaDiagnostic> {
    let mut children = node.children();
    let text = children
        .next()
        .filter(|n| n.is_text())
        .ok_or_else(|| bad("expected a literal property value"))?;
    if children.next().is_some() {
        return Err(bad("expected a single literal property value"));
    }
    let range = text.range();
    let raw = &xml[range.clone()];
    let value = raw.trim();
    let start = range.start + raw.len() - raw.trim_start().len();
    Ok((value, start..start + value.len()))
}

fn observed<'a>(
    xml: &'a str,
    dimension: Node<'_, '_>,
) -> Result<(&'a str, std::ops::Range<usize>), MetaDiagnostic> {
    let property = child(child(dimension, "Properties")?, "TypeReductionMode")?;
    let (value, range) = literal(xml, property)?;
    if !matches!(value, "Deny" | "TransformValues" | "DeleteData") {
        return Err(bad(
            "TypeReductionMode must contain Deny, TransformValues or DeleteData",
        ));
    }
    Ok((value, range))
}
pub(crate) fn edit(
    xml: &str,
    selected: std::ops::Range<usize>,
    value: &str,
) -> Result<Option<(std::ops::Range<usize>, String)>, MetaDiagnostic> {
    if !matches!(value, "Deny" | "TransformValues") {
        return Err(bad("typeReductionMode must be Deny or TransformValues"));
    }
    let doc = Document::parse(xml).map_err(|_| bad("invalid metadata XML"))?;
    let root = doc.root_element();
    if !root.has_tag_name((MD, "MetaDataObject")) {
        return Err(bad("expected metadata descriptor"));
    }
    if unica_source_designer::roots::root_version_literal(xml, root).as_deref() != Some("2.20") {
        return Err(bad("typeReductionMode requires metadata XML format 2.20"));
    }
    let owners: Vec<_> = root.children().filter(Node::is_element).collect();
    let [owner] = owners.as_slice() else {
        return Err(bad("expected one information register"));
    };
    if !owner.has_tag_name((MD, "InformationRegister")) {
        return Err(bad("typeReductionMode requires an information register"));
    }
    let dimension = child(*owner, "ChildObjects")?
        .children()
        .find(|node| node.range() == selected && node.has_tag_name((MD, "Dimension")))
        .ok_or_else(|| bad("selected dimension is not in the register"))?;
    let (previous, range) = observed(xml, dimension)?;
    Ok((previous != value).then(|| (range, value.to_owned())))
}
/// The writer owns effect evidence independently of public source read profiles.
pub(crate) fn effect(
    xml: &str,
    object: Node<'_, '_>,
    mut value: serde_json::Value,
    selected: &[&str],
) -> Result<serde_json::Value, MetaDiagnostic> {
    let parent = child(object, "ChildObjects")?;
    for item in value
        .as_array_mut()
        .ok_or_else(|| bad("dimension effect must be an array"))?
    {
        let name = item["name"]
            .as_str()
            .ok_or_else(|| bad("dimension effect has no name"))?;
        if !selected
            .iter()
            .any(|selected| unica_metadata::metadata_name_eq(name, selected))
        {
            continue;
        }
        let dimension = parent
            .children()
            .filter(|n| n.has_tag_name((MD, "Dimension")))
            .find(|n| {
                child(*n, "Properties")
                    .and_then(|p| child(p, "Name"))
                    .is_ok_and(|n| {
                        n.text()
                            .is_some_and(|v| unica_metadata::metadata_name_eq(v, name))
                    })
            })
            .ok_or_else(|| bad("dimension effect target is unavailable"))?;
        item["TypeReductionMode"] = serde_json::Value::String(observed(xml, dimension)?.0.into());
    }
    Ok(value)
}
#[cfg(test)]
mod tests;
