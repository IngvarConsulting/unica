//! Functional option storage properties, retaining all unrelated source bytes.

use roxmltree::{Document, Node};
use unica_metadata::property_input::FunctionalOptionPropertyChanges;
use unica_source_contract::{proposal::ApplyProposal, SourceSetKind};

const MD: &str = "http://v8.1c.ru/8.3/MDClasses";

fn only_child<'a>(parent: Node<'a, 'a>, tag: &str) -> Result<Node<'a, 'a>, String> {
    let mut nodes = parent
        .children()
        .filter(|node| node.is_element() && node.tag_name().name() == tag);
    match (nodes.next(), nodes.next()) {
        (Some(node), None) if node.has_tag_name((MD, tag)) => Ok(node),
        _ => Err(format!(
            "expected one direct {tag} element in the metadata namespace"
        )),
    }
}

fn owner<'a>(document: &'a Document<'a>, kind: &str) -> Result<Node<'a, 'a>, String> {
    let root = document.root_element();
    if !root.has_tag_name((MD, "MetaDataObject")) {
        return Err("expected MetaDataObject in the metadata namespace".into());
    }
    let mut children = root.children().filter(Node::is_element);
    match (children.next(), children.next()) {
        (Some(node), None) if node.has_tag_name((MD, kind)) => Ok(node),
        _ => Err(format!("expected one {kind} owner")),
    }
}

fn scalar(node: Node<'_, '_>) -> Result<String, String> {
    if node.attributes().next().is_some() || node.children().any(|node| node.is_element()) {
        return Err(format!("{} must be a plain scalar", node.tag_name().name()));
    }
    Ok(node
        .children()
        .filter(Node::is_text)
        .filter_map(|node| node.text())
        .collect())
}

fn replacement(source: &str, node: Node<'_, '_>, value: &str) -> String {
    let original = &source[node.range()];
    let name = original
        .trim_start_matches('<')
        .split([' ', '\t', '\r', '\n', '/', '>'])
        .next()
        .expect("element name");
    let opening = &original[..original.find('>').expect("element opening")];
    let opening = opening.strip_suffix('/').unwrap_or(opening);
    format!("{opening}>{value}</{name}>")
}

fn properties<'a>(document: &'a Document<'a>, name: &str) -> Result<Node<'a, 'a>, String> {
    let properties = only_child(owner(document, "FunctionalOption")?, "Properties")?;
    if scalar(only_child(properties, "Name")?)? != name {
        return Err("FunctionalOption Name does not match its address".into());
    }
    Ok(properties)
}

pub fn read(source: &str, name: &str) -> Result<(String, bool), String> {
    let document = Document::parse(source.trim_start_matches('\u{feff}'))
        .map_err(|error| error.to_string())?;
    let properties = properties(&document, name)?;
    let location = scalar(only_child(properties, "Location")?)?;
    let flag = scalar(only_child(properties, "PrivilegedGetMode")?)?;
    let flag = match flag.as_str() {
        "true" | "1" => true,
        "false" | "0" => false,
        _ => return Err("PrivilegedGetMode must be an XML boolean".into()),
    };
    Ok((location, flag))
}

pub fn patch(
    source: &str,
    name: &str,
    changes: &FunctionalOptionPropertyChanges,
) -> Result<String, String> {
    let observed = read(source, name)?;
    let document = Document::parse(source).map_err(|error| error.to_string())?;
    let properties = properties(&document, name)?;
    let mut replacements = Vec::new();
    if let Some(location) = &changes.location {
        if *location != observed.0 {
            let escaped = location
                .replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;");
            let node = only_child(properties, "Location")?;
            replacements.push((node.range(), replacement(source, node, &escaped)));
        }
    }
    if let Some(flag) = changes.privileged_get_mode {
        if flag != observed.1 {
            let node = only_child(properties, "PrivilegedGetMode")?;
            replacements.push((node.range(), replacement(source, node, &flag.to_string())));
        }
    }
    replacements.sort_by_key(|(range, _)| std::cmp::Reverse(range.start));
    let mut updated = source.to_string();
    for (range, replacement) in replacements {
        updated.replace_range(range, &replacement);
    }
    Ok(updated)
}

/// Retain the storage descriptor as a proposal dependency, including no-op edits.
pub fn validate_location(
    staged: &mut ApplyProposal<'_>,
    source_kind: SourceSetKind,
    location: &str,
) -> Result<(), String> {
    if location.is_empty() {
        return Err(
            "Location must not be empty: the platform requires functional option storage".into(),
        );
    }
    let parts: Vec<_> = location.split('.').collect();
    let [kind, name, members @ ..] = parts.as_slice() else {
        return Err("Location must name an existing constant, attribute or resource".into());
    };
    let metadata_kind = unica_metadata::MetadataKind::parse(kind).map_err(|error| error.message)?;
    let collections = unica_metadata::metadata_kind_collections(metadata_kind);
    use unica_metadata::MetaCollection;
    let allowed = match members {
        [] => *kind == "Constant",
        ["Attribute", _] => collections.contains(&MetaCollection::Attributes),
        ["Resource", _] => collections.contains(&MetaCollection::Resources),
        ["TabularSection", _, "Attribute", _] => {
            collections.contains(&MetaCollection::TabularSections)
        }
        _ => false,
    };
    if !allowed {
        return Err("Location must name an existing constant, attribute or resource".into());
    }
    // The address parser also rejects separators and traversal in identifiers.
    unica_address::address::QualifiedAddress::parse(&format!("storage:{location}"))
        .map_err(|error| error.to_string())?;
    let address =
        unica_address::source_target::MetadataAddress::parse_logical(&format!("{kind}.{name}"))
            .map_err(|error| error.to_string())?;
    let relative =
        unica_source_designer::layout::metadata_descriptor_relative(&address, source_kind)?;
    let bytes = staged
        .read(&relative)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("Location target {location} does not exist"))?;
    let source = std::str::from_utf8(&bytes).map_err(|error| error.to_string())?;
    let document = Document::parse(source.trim_start_matches('\u{feff}'))
        .map_err(|error| error.to_string())?;
    let object = owner(&document, kind)?;
    if scalar(only_child(only_child(object, "Properties")?, "Name")?)? != *name {
        return Err("Location owner Name does not match its address".into());
    }
    let mut selected = object;
    for [member_kind, member_name] in members.as_chunks::<2>().0 {
        let children = only_child(selected, "ChildObjects")?;
        let mut found = None;
        for node in children
            .children()
            .filter(|node| node.is_element() && node.tag_name().name() == *member_kind)
        {
            if !node.has_tag_name((MD, *member_kind)) {
                return Err("Location member has a foreign namespace".into());
            }
            if scalar(only_child(only_child(node, "Properties")?, "Name")?)? == *member_name
                && found.replace(node).is_some()
            {
                return Err(format!("Location target {location} is ambiguous"));
            }
        }
        selected = found.ok_or_else(|| format!("Location target {location} does not exist"))?;
    }
    only_child(only_child(selected, "Properties")?, "Type")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(extra: &str) -> String {
        format!("<MetaDataObject xmlns=\"{MD}\" version=\"2.20\"><FunctionalOption uuid=\"kept\"><Properties><Name>Option</Name><Location>Constant.Before</Location><PrivilegedGetMode>false</PrivilegedGetMode><Content>Catalog.Kept</Content>{extra}</Properties></FunctionalOption></MetaDataObject>")
    }

    #[test]
    fn patch_rejects_ambiguous_and_non_scalar_properties() {
        let changes = FunctionalOptionPropertyChanges {
            comment: None,
            location: None,
            privileged_get_mode: Some(true),
        };
        for bad in [
            "<Location>Constant.Other</Location>",
            "<PrivilegedGetMode>true</PrivilegedGetMode>",
            "<Location xmlns=\"foreign\"/>",
        ] {
            assert!(patch(&source(bad), "Option", &changes).is_err());
        }
        for (from, to) in [
            ("false", "invalid"),
            ("<Location>", "<Location unknown=\"1\">"),
            ("Constant.Before", "<Nested/>"),
            ("<Name>Option</Name>", "<Name>Wrong</Name>"),
        ] {
            assert!(patch(&source("").replace(from, to), "Option", &changes).is_err());
        }
        assert_eq!(
            patch(&source(""), "Option", &changes).unwrap(),
            source("").replace("false", "true")
        );
        let unchanged = FunctionalOptionPropertyChanges {
            comment: None,
            location: Some("Constant.Before".into()),
            privileged_get_mode: Some(false),
        };
        assert_eq!(
            patch(&source(""), "Option", &unchanged).unwrap(),
            source("")
        );
    }

    #[test]
    fn prefixed_properties_keep_their_local_namespace_binding() {
        let source = source("").replace(
            "<PrivilegedGetMode>false</PrivilegedGetMode>",
            &format!("<m:PrivilegedGetMode xmlns:m=\"{MD}\">false</m:PrivilegedGetMode>"),
        );
        let changes = FunctionalOptionPropertyChanges {
            comment: None,
            location: None,
            privileged_get_mode: Some(true),
        };
        let updated = patch(&source, "Option", &changes).unwrap();
        assert!(read(&updated, "Option").unwrap().1);
        assert_eq!(updated, source.replace(">false<", ">true<"));
    }
}
