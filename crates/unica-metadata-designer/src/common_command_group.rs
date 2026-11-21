use crate::descriptor_comment::{descriptor_properties, direct};
use roxmltree::Document;

pub(crate) fn patch(source: &str, name: &str, group: &str) -> Result<String, String> {
    let document = Document::parse(source).map_err(|error| error.to_string())?;
    let properties = descriptor_properties(&document, "CommonCommand")?;
    let owner_name = direct(properties, "Name")?;
    if owner_name.children().any(|node| !node.is_text())
        || unica_xml_format::read::direct_text(owner_name) != name
    {
        return Err("CommonCommand Name does not match its address".into());
    }
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
        let opening_end = raw
            .find('>')
            .ok_or("CommonCommand Group opening tag is malformed")?;
        let opening = &raw[..opening_end];
        if let Some(opening) = opening.strip_suffix('/') {
            let tag = opening[1..].trim_end();
            patched.replace_range(range, &format!("{opening}>{escaped}</{tag}>"));
        } else {
            patched.insert_str(range.start + opening_end + 1, &escaped);
        }
    }
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
}
