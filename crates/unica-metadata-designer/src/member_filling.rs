//! Strict existing FillFromFillingValue mutation and its semantic evidence.
use roxmltree::{Document, Node};
use unica_metadata::{MetaDiagnostic, MetaDiagnosticCode};
const MD: &str = "http://v8.1c.ru/8.3/MDClasses";
fn bad(message: &str) -> MetaDiagnostic {
    MetaDiagnostic::error(MetaDiagnosticCode::InvalidArguments, message)
        .with_field("fillFromFillingValue")
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

fn member_tag(collection: unica_metadata::MetaCollection) -> Result<&'static str, MetaDiagnostic> {
    match collection {
        unica_metadata::MetaCollection::Attributes => Ok("Attribute"),
        unica_metadata::MetaCollection::Dimensions => Ok("Dimension"),
        unica_metadata::MetaCollection::Resources => Ok("Resource"),
        _ => Err(bad(
            "fillFromFillingValue requires an attribute, dimension or resource",
        )),
    }
}
fn member_parent<'a, 'i>(
    owner: Node<'a, 'i>,
    scope: Option<&str>,
) -> Result<Node<'a, 'i>, MetaDiagnostic> {
    let parent = child(owner, "ChildObjects")?;
    let Some(name) = scope else {
        return Ok(parent);
    };
    let mut sections = parent
        .children()
        .filter(|node| node.has_tag_name((MD, "TabularSection")))
        .filter(|node| {
            child(*node, "Properties")
                .and_then(|properties| child(properties, "Name"))
                .is_ok_and(|node| {
                    node.text()
                        .is_some_and(|value| unica_metadata::metadata_name_eq(value, name))
                })
        });
    let section = sections
        .next()
        .ok_or_else(|| bad("tabular section is unavailable"))?;
    if sections.next().is_some() {
        return Err(bad("tabular section is duplicated"));
    }
    child(section, "ChildObjects")
}
fn observed(
    xml: &str,
    dimension: Node<'_, '_>,
) -> Result<(bool, std::ops::Range<usize>), MetaDiagnostic> {
    let property = child(child(dimension, "Properties")?, "FillFromFillingValue")?;
    let (value, range) = literal(xml, property)?;
    match value {
        "true" => Ok((true, range)),
        "false" => Ok((false, range)),
        _ => Err(bad(
            "FillFromFillingValue must contain literal true or false",
        )),
    }
}
pub(crate) fn edit(
    xml: &str,
    selected: std::ops::Range<usize>,
    value: bool,
    collection: unica_metadata::MetaCollection,
    scope: Option<&str>,
) -> Result<Option<(std::ops::Range<usize>, String)>, MetaDiagnostic> {
    let doc = Document::parse(xml).map_err(|_| bad("invalid metadata XML"))?;
    let root = doc.root_element();
    if !root.has_tag_name((MD, "MetaDataObject")) {
        return Err(bad("expected metadata descriptor"));
    }
    if unica_source_designer::roots::root_version_literal(xml, root).as_deref() != Some("2.20") {
        return Err(bad(
            "fillFromFillingValue requires metadata XML format 2.20",
        ));
    }
    let owners: Vec<_> = root.children().filter(Node::is_element).collect();
    let [owner] = owners.as_slice() else {
        return Err(bad("expected one metadata owner"));
    };
    let kind = unica_metadata::MetadataKind::parse(owner.tag_name().name())
        .map_err(|_| bad("unknown metadata owner"))?;
    if owner.tag_name().namespace() != Some(MD)
        || !unica_metadata::metadata_filling_data_is_allowed(kind, collection, scope.is_some())
    {
        return Err(bad(
            "fillFromFillingValue is unavailable for this owner and member context",
        ));
    }
    let parent = member_parent(*owner, scope)?;
    let tag = member_tag(collection)?;
    let dimension = parent
        .children()
        .find(|node| node.range() == selected && node.has_tag_name((MD, tag)))
        .ok_or_else(|| bad("selected member is not in the requested collection"))?;
    let (previous, range) = observed(xml, dimension)?;
    Ok((previous != value).then(|| (range, value.to_string())))
}
/// The writer owns effect evidence independently of public source read profiles.
pub(crate) fn effect(
    xml: &str,
    object: Node<'_, '_>,
    collection: unica_metadata::MetaCollection,
    scope: Option<&str>,
    mut value: serde_json::Value,
    selected: &[&str],
) -> Result<serde_json::Value, MetaDiagnostic> {
    let parent = member_parent(object, scope)?;
    let tag = member_tag(collection)?;
    for item in value
        .as_array_mut()
        .ok_or_else(|| bad("member effect must be an array"))?
    {
        let name = item["name"]
            .as_str()
            .ok_or_else(|| bad("member effect has no name"))?;
        if !selected
            .iter()
            .any(|selected| unica_metadata::metadata_name_eq(name, selected))
        {
            continue;
        }
        let dimension = parent
            .children()
            .filter(|n| n.has_tag_name((MD, tag)))
            .find(|n| {
                child(*n, "Properties")
                    .and_then(|p| child(p, "Name"))
                    .is_ok_and(|n| {
                        n.text()
                            .is_some_and(|v| unica_metadata::metadata_name_eq(v, name))
                    })
            })
            .ok_or_else(|| bad("member effect target is unavailable"))?;
        item["FillFromFillingValue"] = serde_json::Value::Bool(observed(xml, dimension)?.0);
    }
    Ok(value)
}
#[cfg(test)]
mod tests;
