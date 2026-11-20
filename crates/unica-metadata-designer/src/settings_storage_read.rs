//! Чтение двух основных назначений форм хранилища настроек.

use crate::descriptor_fields::MD;
use crate::info_projection::{
    direct_meta_info_property, direct_text_content, meta_info_property_value_is_valid,
    parse_typed_or_nil_value,
};
use roxmltree::{Document, Node};
use serde_json::{json, Value};
use unica_address::address::{NodeKind, QualifiedAddress};
use unica_address::source_target::MetadataAddress;
use unica_metadata::{MetaInfoPropertyValueKind, MetaObservedPropertyValue};

const XSI: &str = "http://www.w3.org/2001/XMLSchema-instance";
const FORM_PROPERTIES: &[&str] = &["DefaultSaveForm", "DefaultLoadForm"];

fn observe_form(
    properties: Node<'_, '_>,
    name: &str,
    source_set: &str,
) -> Result<Option<String>, String> {
    let Some(node) = direct_meta_info_property(properties, "SettingsStorage", name)
        .map_err(|error| error.message)?
    else {
        return Ok(None);
    };
    if node.attribute((XSI, "nil")).is_some() {
        parse_typed_or_nil_value(node, &format!("properties.{name}"))
            .map_err(|error| error.message)?;
        return Ok(None);
    }
    if !meta_info_property_value_is_valid(node, MetaInfoPropertyValueKind::String) {
        return Err(format!("SettingsStorage contains a malformed {name}"));
    }
    let raw = direct_text_content(node);
    if raw.is_empty() {
        return Ok(None);
    }
    let metadata = MetadataAddress::parse_logical(&raw).map_err(|error| error.message)?;
    let address = QualifiedAddress::parse(&format!("{source_set}:{metadata}"))
        .map_err(|error| error.to_string())?;
    if !matches!(
        address.segments().last().map(|part| part.kind()),
        Some(NodeKind::Form | NodeKind::CommonForm)
    ) {
        return Err(format!("SettingsStorage {name} is not a form address"));
    }
    Ok(Some(address.to_string()))
}

/// Читает ссылки из удержанного дескриптора, не обращаясь к их целям.
pub fn enrich_settings_storage_forms(
    bytes: &[u8],
    source_set: &str,
    payload: &mut Value,
) -> Result<(), String> {
    let text = std::str::from_utf8(bytes).map_err(|error| error.to_string())?;
    let document =
        Document::parse(text.trim_start_matches('\u{feff}')).map_err(|error| error.to_string())?;
    let root = document.root_element();
    if !root.has_tag_name((MD, "MetaDataObject")) {
        return Err("SettingsStorage descriptor has the wrong root".into());
    }
    let owners = root.children().filter(Node::is_element).collect::<Vec<_>>();
    let [owner] = owners.as_slice() else {
        return Err("SettingsStorage descriptor must contain one owner".into());
    };
    if !owner.has_tag_name((MD, "SettingsStorage")) {
        return Err("SettingsStorage owner is absent".into());
    }
    let containers = owner
        .children()
        .filter(|node| node.is_element() && node.tag_name().name() == "Properties")
        .collect::<Vec<_>>();
    let properties = match containers.as_slice() {
        [node] if node.tag_name().namespace() == Some(MD) => *node,
        _ => return Err("SettingsStorage must contain one direct Properties container in the metadata namespace".into()),
    };
    let mut observed = Vec::new();
    let mut unreadable = Vec::new();
    for &name in FORM_PROPERTIES {
        match observe_form(properties, name, source_set) {
            Ok(Some(value)) => observed.push(json!({
                "key": name,
                "value": MetaObservedPropertyValue::Text { value },
            })),
            Ok(None) => {}
            Err(message) => unreadable.push(json!({
                "field": format!("properties.{name}"),
                "message": message,
            })),
        }
    }
    for (key, values) in [("properties", observed), ("unreadable", unreadable)] {
        if values.is_empty() {
            continue;
        }
        if let Some(existing) = payload[key].as_array_mut() {
            existing.extend(values);
        } else {
            payload[key] = json!(values);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn descriptor(properties: &str) -> String {
        format!("<MetaDataObject xmlns=\"{MD}\" xmlns:xsi=\"{XSI}\"><SettingsStorage><Properties><Name>Probe</Name>{properties}</Properties></SettingsStorage></MetaDataObject>")
    }

    #[test]
    fn form_reference_profile_keeps_existing_facts_and_reports_non_form_targets() {
        let mut payload = json!({"properties":[{"key":"Comment","value":"kept"}]});
        let xml = descriptor("<DefaultSaveForm>Catalog.Items</DefaultSaveForm><DefaultLoadForm>CommonForm.Load</DefaultLoadForm>");
        enrich_settings_storage_forms(xml.as_bytes(), "archive", &mut payload).unwrap();
        assert_eq!(payload["properties"][0]["value"], "kept");
        assert_eq!(payload["properties"][1]["key"], "DefaultLoadForm");
        assert_eq!(
            payload["properties"][1]["value"]["value"],
            "archive:CommonForm.Load"
        );
        assert_eq!(
            payload["unreadable"][0]["field"],
            "properties.DefaultSaveForm"
        );
        assert_eq!(payload["unreadable"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn ambiguous_storage_frames_do_not_publish_partial_properties() {
        for xml in [
            descriptor("").replace("SettingsStorage", "Catalog"),
            descriptor("").replace("</SettingsStorage>", "<Properties/></SettingsStorage>"),
            descriptor("").replace(
                "</SettingsStorage>",
                "<Properties xmlns=\"urn:foreign\"/></SettingsStorage>",
            ),
            descriptor("").replace(MD, "urn:foreign"),
        ] {
            let original = json!({"properties":[{"key":"Comment","value":"kept"}]});
            let mut payload = original.clone();
            assert!(enrich_settings_storage_forms(xml.as_bytes(), "main", &mut payload).is_err());
            assert_eq!(payload, original);
        }
    }
}
