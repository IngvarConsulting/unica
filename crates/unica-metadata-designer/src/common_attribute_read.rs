//! Наблюдение флагов индексирования, истории и ввода общего реквизита.

use crate::info_projection::{
    direct_meta_info_property, direct_text_content, meta_info_property_value_is_valid,
};
use roxmltree::{Document, Node};
use serde_json::{json, Value};
use unica_metadata::{MetaInfoPropertyValueKind, MetaObservedPropertyValue};

const MD: &str = "http://v8.1c.ru/8.3/MDClasses";

#[derive(Clone, Copy)]
enum ValueKind {
    Enumeration(&'static [&'static str]),
    Boolean,
}

const PROFILE: &[(&str, ValueKind)] = &[
    (
        "Indexing",
        ValueKind::Enumeration(&["DontIndex", "Index", "IndexWithAdditionalOrder"]),
    ),
    (
        "FullTextSearch",
        ValueKind::Enumeration(&["Use", "DontUse"]),
    ),
    ("DataHistory", ValueKind::Enumeration(&["Use", "DontUse"])),
    ("FillFromFillingValue", ValueKind::Boolean),
    (
        "FillChecking",
        ValueKind::Enumeration(&["DontCheck", "ShowError"]),
    ),
    (
        "QuickChoice",
        ValueKind::Enumeration(&["Auto", "Use", "DontUse"]),
    ),
    (
        "CreateOnInput",
        ValueKind::Enumeration(&["Auto", "Use", "DontUse"]),
    ),
    (
        "ChoiceHistoryOnInput",
        ValueKind::Enumeration(&["Auto", "DontUse"]),
    ),
];

fn observe(
    properties: Node<'_, '_>,
    name: &str,
    kind: ValueKind,
) -> Result<Option<MetaObservedPropertyValue>, String> {
    let Some(node) = direct_meta_info_property(properties, "CommonAttribute", name)
        .map_err(|error| error.message)?
    else {
        return Ok(None);
    };
    let shape = match kind {
        ValueKind::Enumeration(_) => MetaInfoPropertyValueKind::String,
        ValueKind::Boolean => MetaInfoPropertyValueKind::Boolean,
    };
    let value = direct_text_content(node);
    if !meta_info_property_value_is_valid(node, shape)
        || matches!(kind, ValueKind::Enumeration(values) if !values.contains(&value.as_str()))
    {
        return Err(format!("CommonAttribute contains a malformed {name}"));
    }
    Ok(Some(match kind {
        ValueKind::Enumeration(_) => MetaObservedPropertyValue::Text { value },
        ValueKind::Boolean => MetaObservedPropertyValue::Boolean {
            value: value == "true",
        },
    }))
}

/// Добавляет только наблюдаемые свойства; порча одного поля не скрывает остальные.
pub fn enrich_common_attribute(bytes: &[u8], payload: &mut Value) -> Result<(), String> {
    let text = std::str::from_utf8(bytes).map_err(|error| error.to_string())?;
    let document =
        Document::parse(text.trim_start_matches('\u{feff}')).map_err(|error| error.to_string())?;
    let root = document.root_element();
    if !root.has_tag_name((MD, "MetaDataObject")) {
        return Err("CommonAttribute descriptor has the wrong root".into());
    }
    let owners = root.children().filter(Node::is_element).collect::<Vec<_>>();
    let [owner] = owners.as_slice() else {
        return Err("CommonAttribute descriptor must contain one owner".into());
    };
    if !owner.has_tag_name((MD, "CommonAttribute")) {
        return Err("CommonAttribute owner is absent".into());
    }
    let containers = owner
        .children()
        .filter(|node| node.is_element() && node.tag_name().name() == "Properties")
        .collect::<Vec<_>>();
    let properties = match containers.as_slice() {
        [node] if node.tag_name().namespace() == Some(MD) => *node,
        _ => return Err("CommonAttribute must contain one direct Properties container in the metadata namespace".into()),
    };
    let mut observed = Vec::new();
    let mut unreadable = Vec::new();
    for &(name, kind) in PROFILE {
        match observe(properties, name, kind) {
            Ok(Some(value)) => observed.push(json!({"key": name, "value": value})),
            Ok(None) => {}
            Err(message) => unreadable.push(json!({
                "field": format!("properties.{name}"), "message": message,
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
        format!("<MetaDataObject xmlns=\"{MD}\"><CommonAttribute><Properties><Name>Probe</Name>{properties}</Properties></CommonAttribute></MetaDataObject>")
    }

    #[test]
    fn common_attribute_profile_validates_each_property_without_hiding_independent_facts() {
        for &(name, kind) in PROFILE {
            let valid = match kind {
                ValueKind::Enumeration(values) => values[0],
                ValueKind::Boolean => "false",
            };
            for raw in [
                format!("<{name}/>"),
                format!("<{name}>Unknown</{name}>"),
                format!("<{name}><Nested>{valid}</Nested></{name}>"),
                format!("<{name} extra=\"bad\">{valid}</{name}>"),
                format!("<{name} xmlns=\"urn:foreign\">{valid}</{name}>"),
                format!("<{name}>{valid}</{name}><{name}>{valid}</{name}>"),
                format!("<{name}> {valid} </{name}>"),
            ] {
                let mut payload = json!({"properties":[{"key":"Comment", "value":"kept"}]});
                enrich_common_attribute(descriptor(&raw).as_bytes(), &mut payload).unwrap();
                assert_eq!(payload["properties"].as_array().unwrap().len(), 1, "{raw}");
                assert_eq!(payload["properties"][0]["value"], "kept");
                assert_eq!(payload["unreadable"].as_array().unwrap().len(), 1, "{raw}");
                assert_eq!(
                    payload["unreadable"][0]["field"],
                    format!("properties.{name}")
                );
            }
            let values = match kind {
                ValueKind::Enumeration(values) => values,
                ValueKind::Boolean => &["false", "true"],
            };
            for value in values {
                let mut payload = json!({});
                enrich_common_attribute(
                    descriptor(&format!("<{name}>{value}</{name}>")).as_bytes(),
                    &mut payload,
                )
                .unwrap();
                let expected = match kind {
                    ValueKind::Enumeration(_) => json!({"kind":"text", "value": value}),
                    ValueKind::Boolean => json!({"kind":"boolean", "value": *value == "true"}),
                };
                assert_eq!(payload["properties"][0]["value"], expected);
                assert!(payload.get("unreadable").is_none());
            }
        }
        let mut payload = json!({});
        enrich_common_attribute(
            descriptor("<FutureProperty>ignored</FutureProperty>").as_bytes(),
            &mut payload,
        )
        .unwrap();
        assert_eq!(payload, json!({}));
    }

    #[test]
    fn common_attribute_profile_rejects_ambiguous_properties_containers() {
        for extra in ["<Properties/>", "<Properties xmlns=\"urn:foreign\"/>"] {
            let source =
                descriptor("").replace("</CommonAttribute>", &format!("{extra}</CommonAttribute>"));
            assert!(enrich_common_attribute(source.as_bytes(), &mut json!({})).is_err());
        }
    }
}
