//! The existing integration service's scalar address, independent of channel edits.
use crate::info_projection::{
    direct_meta_info_property, direct_text_content, meta_info_property_value_is_valid,
};
use roxmltree::{Document, Node};
use std::path::Path;
use unica_metadata::MetaInfoPropertyValueKind;

pub const PROPERTY: &str = "ExternalIntegrationServiceAddress";
const MD: &str = "http://v8.1c.ru/8.3/MDClasses";

fn properties<'a, 'input>(document: &'a Document<'input>) -> Result<Node<'a, 'input>, String> {
    let root = document.root_element();
    if !root.has_tag_name((MD, "MetaDataObject")) {
        return Err("IntegrationService descriptor has the wrong root".into());
    }
    let owners = root.children().filter(Node::is_element).collect::<Vec<_>>();
    let [owner] = owners.as_slice() else {
        return Err("IntegrationService descriptor must contain one owner".into());
    };
    if !owner.has_tag_name((MD, "IntegrationService")) {
        return Err("IntegrationService descriptor has the wrong owner".into());
    }
    let containers = owner
        .children()
        .filter(|node| node.is_element() && node.tag_name().name() == "Properties")
        .collect::<Vec<_>>();
    match containers.as_slice() {
        [node] if node.tag_name().namespace() == Some(MD) => Ok(*node),
        _ => Err("IntegrationService must contain one direct Properties container in the metadata namespace".into()),
    }
}

fn address<'a, 'input>(properties: Node<'a, 'input>) -> Result<Option<Node<'a, 'input>>, String> {
    let Some(node) = direct_meta_info_property(properties, "IntegrationService", PROPERTY)
        .map_err(|error| error.message)?
    else {
        return Ok(None);
    };
    if !meta_info_property_value_is_valid(node, MetaInfoPropertyValueKind::String) {
        return Err(
            "IntegrationService contains a malformed ExternalIntegrationServiceAddress".into(),
        );
    }
    Ok(Some(node))
}

/// An absent field is unknown; an explicit empty field is a known empty string.
/// A bad address limits only this fact in the caller's identity payload.
pub fn read(bytes: &[u8]) -> Result<Option<String>, String> {
    let text = std::str::from_utf8(bytes).map_err(|error| error.to_string())?;
    let document =
        Document::parse(text.trim_start_matches('\u{feff}')).map_err(|error| error.to_string())?;
    Ok(address(properties(&document)?)?.map(direct_text_content))
}

/// A range edit preserves all unmentioned descriptor bytes, including channels.
/// The writer requires the existing explicit field and the exact writable owner.
pub fn patch(text: &str, name: &str, value: &str) -> Result<String, String> {
    let path = Path::new("IntegrationServices").join(format!("{name}.xml"));
    let proof =
        unica_source_designer::owner::prove_already_read_metadata_owner(&path, text.as_bytes())
            .map_err(|error| error.message)?;
    if proof.artifact_kind() != "IntegrationService" || proof.artifact_name() != Some(name) {
        return Err(
            "IntegrationService descriptor identity does not match its logical target".into(),
        );
    }
    if let Some(refusal) = unica_source_designer::format_profile::writable_profile_refusal(
        path.display(),
        proof.version(),
    ) {
        return Err(refusal.message);
    }
    let document = Document::parse(text).map_err(|error| error.to_string())?;
    let properties = properties(&document)?;
    let actual_name = direct_meta_info_property(properties, "IntegrationService", "Name")
        .map_err(|error| error.message)?
        .filter(|node| meta_info_property_value_is_valid(*node, MetaInfoPropertyValueKind::String))
        .map(direct_text_content);
    if actual_name.as_deref() != Some(name) {
        return Err(
            "IntegrationService must contain one exact scalar Name matching its target".into(),
        );
    }
    let property = address(properties)?.ok_or_else(|| "IntegrationService has no explicit ExternalIntegrationServiceAddress; restore a platform export before changing it".to_string())?;
    if direct_text_content(property) == value {
        return Ok(text.to_string());
    }
    let opening = &text[property.range().start + 1..];
    let tag = &opening[..opening
        .find(|ch: char| ch.is_whitespace() || ch == '>' || ch == '/')
        .unwrap_or(opening.len())];
    // XML normalizes literal CR. A character reference preserves the supplied string.
    let escaped = value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('\r', "&#13;");
    // Keep the original opening tag, including property-local namespace bindings.
    // A quoted namespace URI may itself contain `>`.
    let mut quote = None;
    let mut opening_end = None;
    for (offset, ch) in text[property.range().start..].char_indices() {
        if let Some(delimiter) = quote {
            if ch == delimiter {
                quote = None;
            }
        } else if matches!(ch, '\'' | '"') {
            quote = Some(ch);
        } else if ch == '>' {
            opening_end = Some(property.range().start + offset);
            break;
        }
    }
    let opening_end = opening_end
        .ok_or_else(|| "IntegrationService address has no opening tag boundary".to_string())?;
    let head = &text[property.range().start..opening_end];
    let head = head.strip_suffix('/').unwrap_or(head);
    let replacement = if value.is_empty() {
        format!("{head}/>")
    } else {
        format!("{head}>{escaped}</{tag}>")
    };
    let mut result = text.to_string();
    result.replace_range(property.range(), &replacement);
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn descriptor(field: &str) -> String {
        format!("<MetaDataObject xmlns=\"{MD}\" version=\"2.20\"><IntegrationService uuid=\"16780000-0000-4000-8000-000000000001\"><InternalInfo/><Properties><Name>Bus</Name><Comment>keep</Comment>{field}</Properties><ChildObjects><IntegrationServiceChannel uuid=\"16780000-0000-4000-8000-000000000002\"><Properties><Name>Out</Name><ExternalIntegrationServiceChannelName>keep-channel</ExternalIntegrationServiceChannelName></Properties></IntegrationServiceChannel></ChildObjects></IntegrationService></MetaDataObject>")
    }
    #[test]
    fn integration_address_observation_distinguishes_absence_empty_and_damage() {
        assert_eq!(read(descriptor("").as_bytes()).unwrap(), None);
        assert_eq!(
            read(descriptor("<ExternalIntegrationServiceAddress/>").as_bytes()).unwrap(),
            Some(String::new())
        );
        assert_eq!(read(descriptor("<ExternalIntegrationServiceAddress>a&amp;<!--keep-->b<![CDATA[c]]></ExternalIntegrationServiceAddress>").as_bytes()).unwrap().as_deref(), Some("a&bc"));
        for field in [
            "<ExternalIntegrationServiceAddress/><ExternalIntegrationServiceAddress/>",
            "<ExternalIntegrationServiceAddress xmlns=\"urn:foreign\"/>",
            "<ExternalIntegrationServiceAddress><Nested/></ExternalIntegrationServiceAddress>",
            "<ExternalIntegrationServiceAddress unknown=\"keep\"/>",
        ] {
            assert!(read(descriptor(field).as_bytes()).is_err(), "{field}");
        }
    }
    #[test]
    fn integration_address_range_edit_roundtrips_exact_strings_and_keeps_other_bytes() {
        let source = descriptor("<ExternalIntegrationServiceAddress/>");
        for value in [
            "https://example.invalid/сервис?a=1&b=\"quoted\"",
            "  <привет> & мир  ",
            "one\rtwo\r\nthree\nfour\tend",
        ] {
            let result = patch(&source, "Bus", value).unwrap();
            assert_eq!(read(result.as_bytes()).unwrap().as_deref(), Some(value));
            let doc = Document::parse(&result).unwrap();
            let node = address(properties(&doc).unwrap()).unwrap().unwrap();
            let mut remainder = result.clone();
            remainder.replace_range(node.range(), "<ExternalIntegrationServiceAddress/>");
            assert_eq!(remainder, source, "all unmentioned bytes survive");
            assert_eq!(
                patch(&result, "Bus", value).unwrap(),
                result,
                "equivalent setting is byte-exact no-op"
            );
            assert_eq!(patch(&result, "Bus", "").unwrap(), source);
        }
    }
    #[test]
    fn integration_address_writer_requires_exact_owner_and_existing_scalar() {
        let valid = descriptor("<ExternalIntegrationServiceAddress/>");
        assert!(patch(&valid, "Other", "https://example.invalid/").is_err());
        for damaged in [
            valid.replace("<Name>Bus</Name>", "<Name>Bus</Name><Name>Other</Name>"),
            descriptor(""),
            valid.replace("version=\"2.20\"", "version=\"2.&#50;0\""),
            valid.replace("<Properties>", "<Properties xmlns=\"urn:foreign\">"),
            valid.replace(
                "<ExternalIntegrationServiceAddress/>",
                "<ExternalIntegrationServiceAddress/><ExternalIntegrationServiceAddress/>",
            ),
            valid.replace(
                "</IntegrationService>",
                "<Properties/></IntegrationService>",
            ),
            valid
                .replace("IntegrationService ", "Catalog ")
                .replace("</IntegrationService>", "</Catalog>"),
        ] {
            assert!(patch(&damaged, "Bus", "https://example.invalid/").is_err());
        }
    }
}
