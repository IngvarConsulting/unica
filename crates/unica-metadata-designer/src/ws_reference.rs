//! One descriptor fact and its existing-field patch; WSDL is never an input.
use roxmltree::{Document, Node};
const MD: &str = "http://v8.1c.ru/8.3/MDClasses";

fn location<'a, 'input>(doc: &'a Document<'input>) -> Result<Option<Node<'a, 'input>>, String> {
    let root = doc.root_element();
    if !root.has_tag_name((MD, "MetaDataObject")) {
        return Err("WSReference descriptor has the wrong root".into());
    }
    let owners: Vec<_> = root.children().filter(Node::is_element).collect();
    let [owner] = owners.as_slice() else {
        return Err("WSReference requires one direct owner".into());
    };
    if !owner.has_tag_name((MD, "WSReference")) {
        return Err("WSReference owner is absent".into());
    }
    let containers: Vec<_> = owner
        .children()
        .filter(|n| n.is_element() && n.tag_name().name() == "Properties")
        .collect();
    let [properties] = containers.as_slice() else {
        return Err("WSReference requires one direct Properties container".into());
    };
    if !properties.has_tag_name((MD, "Properties")) {
        return Err("WSReference Properties has the wrong namespace".into());
    }
    let fields: Vec<_> = properties
        .children()
        .filter(|n| n.is_element() && n.tag_name().name() == "LocationURL")
        .collect();
    match fields.as_slice() {
        [] => Ok(None),
        [field]
            if field.has_tag_name((MD, "LocationURL"))
                && field.attributes().len() == 0
                && !field.children().any(|n| n.is_element() || n.is_pi()) =>
        {
            Ok(Some(*field))
        }
        _ => Err("LocationURL requires one plain-text property in the metadata namespace".into()),
    }
}

fn text(node: Node<'_, '_>) -> String {
    node.children()
        .filter(Node::is_text)
        .filter_map(|n| n.text())
        .collect()
}

/// Missing and explicitly empty are different observations. A damaged field
/// leaves the separately proved owner available to the caller.
pub fn read_location_url(bytes: &[u8]) -> Result<Option<String>, String> {
    let source = std::str::from_utf8(bytes).map_err(|e| e.to_string())?;
    let doc = Document::parse(source.trim_start_matches('\u{feff}')).map_err(|e| e.to_string())?;
    location(&doc).map(|node| node.map(text))
}

/// The format boundary rejects caller text before reading descriptor state.
pub fn validate_location_url(value: &str) -> Result<(), String> {
    if value.chars().any(|c| !matches!(c, '\u{9}' | '\u{a}' | '\u{d}' | '\u{20}'..='\u{d7ff}' | '\u{e000}'..='\u{fffd}' | '\u{10000}'..='\u{10ffff}')) {
        return Err("LocationURL contains a character forbidden by XML 1.0".into());
    }
    Ok(())
}

/// Patch only an existing property. The caller proves owner identity and
/// registration from retained images before accepting this postimage.
pub fn patch_location_url(
    source: &str,
    expected_name: &str,
    value: &str,
) -> Result<String, String> {
    validate_location_url(value)?;
    let doc = Document::parse(source).map_err(|e| e.to_string())?;
    let field = location(&doc)?.ok_or("WSReference has no LocationURL property")?;
    let names: Vec<_> = field
        .parent()
        .expect("LocationURL has a proved Properties parent")
        .children()
        .filter(|node| node.is_element() && node.tag_name().name() == "Name")
        .collect();
    let [name] = names.as_slice() else {
        return Err("WSReference requires one plain Name matching its target".into());
    };
    if !name.has_tag_name((MD, "Name"))
        || name.attributes().len() != 0
        || name
            .children()
            .any(|node| node.is_element() || node.is_pi())
        || text(*name) != expected_name
    {
        return Err("WSReference requires one plain Name matching its target".into());
    }
    if text(field) == value {
        return Ok(source.to_string());
    }
    let range = field.range();
    let opening = &source[range.start + 1..];
    let end = opening
        .find(|c: char| c.is_whitespace() || c == '/' || c == '>')
        .ok_or("invalid LocationURL opening tag")?;
    let name = &opening[..end];
    let escaped = unica_xml_format::text::escape_xml_text(value).replace('\r', "&#13;");
    let fragment = &source[range.clone()];
    let opening_end = crate::predefined::opening_tag_end(fragment)?;
    let opening = &fragment[..opening_end];
    let opening = opening.strip_suffix('/').unwrap_or(opening);
    let replacement = if value.is_empty() {
        format!("{opening}/>")
    } else {
        format!("{opening}>{escaped}</{name}>")
    };
    let mut updated = source.to_string();
    updated.replace_range(range, &replacement);
    Ok(updated)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn descriptor(property: &str) -> String {
        format!("<MetaDataObject xmlns=\"{MD}\" version=\"2.20\"><WSReference uuid=\"16620000-0000-4000-8000-000000000001\"><Properties><Name>Probe</Name><Comment>Keep &amp; verify</Comment>{property}</Properties></WSReference></MetaDataObject>")
    }
    #[test]
    fn location_observation_distinguishes_missing_empty_text_and_damage() {
        for (raw, expected) in [
            ("", None),
            ("<LocationURL/>", Some("")),
            (
                "<LocationURL>  &amp; <!--x-->ю<![CDATA[<]]>\n</LocationURL>",
                Some("  & ю<\n"),
            ),
        ] {
            assert_eq!(
                read_location_url(descriptor(raw).as_bytes())
                    .unwrap()
                    .as_deref(),
                expected
            );
        }
        for raw in [
            "<LocationURL/><LocationURL/>",
            "<LocationURL xmlns=\"urn:foreign\"/>",
            "<LocationURL nil=\"true\"/>",
            "<LocationURL><nested/></LocationURL>",
        ] {
            let source = descriptor(raw);
            assert!(read_location_url(source.as_bytes()).is_err());
            assert!(patch_location_url(&source, "Probe", "new").is_err());
        }
    }
    #[test]
    fn patch_changes_only_the_existing_field_and_preserves_exact_text() {
        let source = descriptor("<LocationURL>old</LocationURL>");
        for value in [
            "",
            "https://example.invalid/?a=1&b=<ю>&quote=\"x\"",
            " \t\r\n value ",
        ] {
            let updated = patch_location_url(&source, "Probe", value).unwrap();
            assert_eq!(
                read_location_url(updated.as_bytes()).unwrap().as_deref(),
                Some(value)
            );
            let doc = Document::parse(&updated).unwrap();
            let field = location(&doc).unwrap().unwrap();
            let mut outside = updated.clone();
            outside.replace_range(field.range(), "<LocationURL>old</LocationURL>");
            assert_eq!(outside, source);
        }
        assert!(patch_location_url(&descriptor(""), "Probe", "new").is_err());
        for value in ["bad\0value", "bad\u{b}value", "bad\u{fffe}value"] {
            assert!(patch_location_url(&source, "Probe", value).is_err());
        }
    }
    #[test]
    fn noop_keeps_original_entity_spelling_comments_and_prefix() {
        let source=descriptor("<m:LocationURL xmlns:m=\"http://v8.1c.ru/8.3/MDClasses\">a&#38;<!--keep-->b</m:LocationURL>");
        assert_eq!(patch_location_url(&source, "Probe", "a&b").unwrap(), source);
        let patched = patch_location_url(&source, "Probe", "changed").unwrap();
        assert!(patched.contains("changed</m:LocationURL>"));
        assert!(Document::parse(&patched).is_ok());
    }
}
