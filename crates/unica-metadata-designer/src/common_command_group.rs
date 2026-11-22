use crate::descriptor_comment::{descriptor_properties, direct};
use roxmltree::{Document, Node};

fn named_properties<'a, 'input>(
    document: &'a Document<'input>,
    kind: &str,
    name: &str,
) -> Result<Node<'a, 'input>, String> {
    let properties = descriptor_properties(document, kind)?;
    let owner_name = direct(properties, "Name")?;
    if owner_name.attributes().next().is_some()
        || owner_name.children().any(|node| !node.is_text())
        || unica_xml_format::read::direct_text(owner_name) != name
    {
        return Err(format!("{kind} Name does not match its address"));
    }
    Ok(properties)
}

pub(crate) fn prove_group_descriptor(bytes: &[u8], name: &str) -> Result<(), String> {
    let text = std::str::from_utf8(bytes).map_err(|error| error.to_string())?;
    let document =
        Document::parse(text.trim_start_matches('\u{feff}')).map_err(|error| error.to_string())?;
    named_properties(&document, "CommandGroup", name).map(|_| ())
}

pub(crate) fn patch(source: &str, name: &str, group: &str) -> Result<String, String> {
    let document = Document::parse(source).map_err(|error| error.to_string())?;
    let properties = named_properties(&document, "CommonCommand", name)?;
    let property = direct(properties, "Group")?;
    let mut content = property.children();
    let value = content.next();
    if property.attributes().next().is_some()
        || value.is_some_and(|node| !node.is_text())
        || content.next().is_some()
    {
        return Err("CommonCommand Group must be plain text".into());
    }
    if property.text() == Some(group) {
        return Ok(source.to_string());
    }
    let escaped = unica_xml_format::text::escape_xml_text(group);
    let mut patched = source.to_string();
    if let Some(value) = value {
        patched.replace_range(value.range(), &escaped);
    } else {
        let range = property.range();
        let raw = &source[range.clone()];
        let opening_end = crate::predefined::opening_tag_end(raw)
            .map_err(|_| "CommonCommand Group opening tag is malformed")?;
        let opening = &raw[..opening_end];
        if let Some(opening) = opening.strip_suffix('/') {
            let tag = opening[1..]
                .split_whitespace()
                .next()
                .ok_or("CommonCommand Group has no element name")?;
            patched.replace_range(range, &format!("{opening}>{escaped}</{tag}>"));
        } else {
            patched.insert_str(range.start + opening_end + 1, &escaped);
        }
    }
    Document::parse(&patched)
        .map_err(|error| format!("CommonCommand Group postimage is invalid: {error}"))?;
    Ok(patched)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patch_preserves_prefix_and_bytes_outside_the_group() {
        let source = r#"<md:MetaDataObject xmlns:md="http://v8.1c.ru/8.3/MDClasses"><md:CommonCommand><md:Properties><md:Name>Probe</md:Name><md:Group /><md:Comment>kept</md:Comment></md:Properties></md:CommonCommand></md:MetaDataObject>"#;
        let group = "NavigationPanelOrdinary";
        let expected = source.replace(
            "<md:Group />",
            "<md:Group >NavigationPanelOrdinary</md:Group>",
        );
        assert_eq!(patch(source, "Probe", group).unwrap(), expected);
        assert_eq!(patch(&expected, "Probe", group).unwrap(), expected);
        assert_eq!(
            patch(&expected, "Probe", "CommandGroup.Placement").unwrap(),
            expected.replace(group, "CommandGroup.Placement")
        );
        let malformed = source.replace("<md:Group />", "<md:Group><md:Name/></md:Group>");
        assert!(patch(&malformed, "Probe", group).is_err());
    }

    #[test]
    fn patch_keeps_local_namespace_declarations_on_an_empty_group() {
        let tag = r#"<md:Group xmlns:extra="urn:unica:example"/>"#;
        let source = format!(
            r#"<md:MetaDataObject xmlns:md="http://v8.1c.ru/8.3/MDClasses"><md:CommonCommand><md:Properties><md:Name>Probe</md:Name>{tag}</md:Properties></md:CommonCommand></md:MetaDataObject>"#
        );
        let patched = patch(&source, "Probe", "NavigationPanelOrdinary").unwrap();
        assert_eq!(
            patched,
            source.replace(
                tag,
                r#"<md:Group xmlns:extra="urn:unica:example">NavigationPanelOrdinary</md:Group>"#
            )
        );
        assert!(Document::parse(&patched).is_ok());
    }
}
