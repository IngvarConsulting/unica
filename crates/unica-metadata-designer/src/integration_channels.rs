//! Bounded XML profile of the inline IntegrationServiceChannel declarations.
use crate::info_projection::{
    direct_text_content, node_has_only_whitespace_text, strict_localized_meta_info_property_value,
    strict_text_leaf_is_valid,
};
use roxmltree::{Document, Node};
use std::collections::BTreeSet;
use unica_metadata::integration_channels::{
    IntegrationChannel, IntegrationChannelLimit, IntegrationChannels,
    INTEGRATION_CHANNEL_PROPERTIES,
};
use unica_metadata::{MetaObservedProperty, MetaObservedPropertyValue};
const MD: &str = "http://v8.1c.ru/8.3/MDClasses";
const PREFIX: &str = "collections.channels";

fn one<'a, 'input>(node: Node<'a, 'input>, name: &str) -> Result<Option<Node<'a, 'input>>, String> {
    let nodes = node
        .children()
        .filter(|n| n.is_element() && n.tag_name().name() == name)
        .collect::<Vec<_>>();
    match nodes.as_slice() {
        [] => Ok(None),
        [node] if node.tag_name().namespace() == Some(MD) => Ok(Some(*node)),
        _ => Err(format!("{name} is duplicated or uses a foreign namespace")),
    }
}
fn properties<'a, 'input>(node: Node<'a, 'input>) -> Result<Node<'a, 'input>, String> {
    let properties = one(node, "Properties")?.ok_or_else(|| "Properties is absent".to_string())?;
    if properties.attributes().len() != 0 || !node_has_only_whitespace_text(properties) {
        return Err("Properties has unsupported attributes or mixed text".into());
    }
    Ok(properties)
}
fn name(node: Node<'_, '_>) -> Result<String, String> {
    let node = one(properties(node)?, "Name")?.ok_or("Name is absent")?;
    let value = direct_text_content(node);
    if !strict_text_leaf_is_valid(node) || !unica_metadata::identifier::is_1c_identifier(&value) {
        return Err("Name is not one direct platform identifier".into());
    }
    Ok(value)
}
fn candidate_names(node: Node<'_, '_>) -> Vec<String> {
    node.children()
        .filter(|n| n.is_element() && n.tag_name().name() == "Properties")
        .flat_map(|p| {
            p.children()
                .filter(|n| n.is_element() && n.tag_name().name() == "Name")
        })
        .filter(|n| strict_text_leaf_is_valid(*n))
        .map(direct_text_content)
        .filter(|s| unica_metadata::identifier::is_1c_identifier(s))
        .collect()
}
fn observe(
    properties: Node<'_, '_>,
    key: &str,
) -> Result<Option<MetaObservedPropertyValue>, String> {
    let Some(node) = one(properties, key)? else {
        return Ok(None);
    };
    if key == "Synonym" {
        return strict_localized_meta_info_property_value(node)
            .map(Some)
            .ok_or_else(|| "Synonym has malformed localized content".into());
    }
    if !strict_text_leaf_is_valid(node) {
        return Err(format!("{key} must contain text only"));
    }
    let value = direct_text_content(node);
    match key {
        "Transactioned" => match value.as_str() {
            "true" => Ok(Some(MetaObservedPropertyValue::Boolean { value: true })),
            "false" => Ok(Some(MetaObservedPropertyValue::Boolean { value: false })),
            _ => Err("Transactioned is not true or false".into()),
        },
        "MessageDirection" if !["Receive", "Send"].contains(&value.as_str()) => {
            Err("MessageDirection is outside Receive/Send".into())
        }
        _ => Ok(Some(MetaObservedPropertyValue::Text { value })),
    }
}
fn limit(facts: &mut IntegrationChannels, field: impl Into<String>, message: impl Into<String>) {
    facts.limits.push(IntegrationChannelLimit {
        field: field.into(),
        message: message.into(),
    });
}

pub fn read(bytes: &[u8], expected_name: &str) -> Result<IntegrationChannels, String> {
    let text = std::str::from_utf8(bytes).map_err(|e| e.to_string())?;
    let doc = Document::parse(text.trim_start_matches('\u{feff}')).map_err(|e| e.to_string())?;
    let root = doc.root_element();
    if !root.has_tag_name((MD, "MetaDataObject")) {
        return Err("IntegrationService descriptor has the wrong root".into());
    }
    let owners = root.children().filter(Node::is_element).collect::<Vec<_>>();
    let [owner] = owners.as_slice() else {
        return Err("IntegrationService descriptor has no unique owner".into());
    };
    if !owner.has_tag_name((MD, "IntegrationService")) || name(*owner)? != expected_name {
        return Err("IntegrationService identity does not match the selected owner".into());
    }
    let mut facts = IntegrationChannels::default();
    let children = match one(*owner, "ChildObjects") {
        Ok(Some(node)) if node.attributes().len() == 0 && node_has_only_whitespace_text(node) => {
            node
        }
        Ok(None) => {
            limit(
                &mut facts,
                PREFIX,
                "ChildObjects is absent; channel completeness is unknown",
            );
            return Ok(facts);
        }
        _ => {
            limit(&mut facts,PREFIX,"ChildObjects is duplicated, foreign, or malformed; channel completeness is unknown");
            return Ok(facts);
        }
    };
    let mut items = Vec::new();
    let mut unavailable = BTreeSet::new();
    let mut seen = BTreeSet::new();
    for (index, channel) in children.children().filter(Node::is_element).enumerate() {
        let field = format!("{PREFIX}[{index}]");
        if !channel.has_tag_name((MD, "IntegrationServiceChannel")) {
            facts.identity_incomplete = true;
            limit(
                &mut facts,
                &field,
                "unsupported direct child; channel completeness is unknown",
            );
            continue;
        }
        let identity = name(channel).and_then(|name| {
            if channel.attributes().any(|a| {
                a.namespace().is_some()
                    || a.name() != "uuid"
                    || uuid::Uuid::parse_str(a.value()).is_err()
            }) {
                return Err("channel has unsupported identity attributes".into());
            }
            Ok(name)
        });
        let name = match identity {
            Ok(value) => value,
            Err(message) => {
                facts.identity_incomplete = true;
                unavailable.extend(candidate_names(channel));
                limit(&mut facts, format!("{field}.Name"), message);
                continue;
            }
        };
        if !seen.insert(name.clone()) {
            unavailable.insert(name.clone());
            limit(
                &mut facts,
                format!("{field}.Name"),
                format!("channel `{name}` has an ambiguous identity"),
            );
        }
        let properties = properties(channel)?;
        let mut item = IntegrationChannel {
            name,
            properties: Vec::new(),
            property_limits: Vec::new(),
        };
        for &key in INTEGRATION_CHANNEL_PROPERTIES {
            match observe(properties, key) {
                Ok(Some(value)) => item.properties.push(MetaObservedProperty {
                    name: key.into(),
                    value,
                }),
                Ok(None) => {}
                Err(message) => {
                    let field = format!("{field}.properties.{key}");
                    item.property_limits.push(format!("{field}: {message}"));
                    limit(&mut facts, field, message);
                }
            }
        }
        for child in channel.children().filter(Node::is_element) {
            if child.tag_name().namespace() != Some(MD)
                || !["Properties", "InternalInfo"].contains(&child.tag_name().name())
            {
                let message = "channel has unsupported direct content";
                item.property_limits.push(message.into());
                limit(&mut facts, &field, message);
            }
        }
        items.push(item);
    }
    items.retain(|item| !unavailable.contains(&item.name));
    facts.unavailable_names = unavailable.into_iter().collect();
    if !items.is_empty() || facts.limits.is_empty() {
        facts.items = Some(items);
    }
    Ok(facts)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn document(children: &str) -> String {
        format!("<MetaDataObject xmlns=\"{MD}\" xmlns:v8=\"http://v8.1c.ru/8.1/data/core\"><IntegrationService><Properties><Name>Bus</Name></Properties>{children}</IntegrationService></MetaDataObject>")
    }
    fn channel(name: &str, properties: &str) -> String {
        format!("<IntegrationServiceChannel><Properties><Name>{name}</Name>{properties}</Properties></IntegrationServiceChannel>")
    }

    #[test]
    fn native_integration_channels_preserve_localized_empty_false_and_full_text() {
        let long = "я".repeat(2100);
        let xml=document(&format!("<ChildObjects>{}</ChildObjects>",channel("Input",&format!("<Synonym><v8:item><v8:lang>ru</v8:lang><v8:content>Вход</v8:content></v8:item><v8:item><v8:lang>en</v8:lang><v8:content/></v8:item></Synonym><Comment/><ExternalIntegrationServiceChannelName>{long}</ExternalIntegrationServiceChannelName><MessageDirection>Receive</MessageDirection><ReceiveMessageProcessing/><Transactioned>false</Transactioned>"))));
        let facts = read(xml.as_bytes(), "Bus").unwrap();
        assert!(facts.limits.is_empty());
        let item = &facts.items.unwrap()[0];
        assert_eq!(item.name, "Input");
        assert_eq!(item.properties.len(), 6);
        assert_eq!(
            item.properties[0].value,
            MetaObservedPropertyValue::LocalizedString {
                values: vec![
                    unica_metadata::MetaLocalizedText {
                        language: "ru".into(),
                        content: "Вход".into()
                    },
                    unica_metadata::MetaLocalizedText {
                        language: "en".into(),
                        content: String::new()
                    }
                ]
            }
        );
        assert_eq!(
            item.properties[1].value,
            MetaObservedPropertyValue::Text {
                value: String::new()
            }
        );
        assert_eq!(
            item.properties[2].value,
            MetaObservedPropertyValue::Text { value: long }
        );
        assert_eq!(
            item.properties[4].value,
            MetaObservedPropertyValue::Text {
                value: String::new()
            }
        );
        assert_eq!(
            item.properties[5].value,
            MetaObservedPropertyValue::Boolean { value: false }
        );
    }

    #[test]
    fn native_integration_channels_distinguish_empty_unknown_and_unavailable_identity() {
        let empty = read(document("<ChildObjects/>").as_bytes(), "Bus").unwrap();
        assert_eq!(empty.items, Some(vec![]));
        assert!(empty.limits.is_empty());
        let unknown = read(document("").as_bytes(), "Bus").unwrap();
        assert!(unknown.items.is_none());
        assert!(!unknown.limits.is_empty());
        let xml = document(&format!(
            "<ChildObjects>{}{}{}</ChildObjects>",
            channel("A", ""),
            channel("A", ""),
            channel("B", "<Transactioned>true</Transactioned>")
        ));
        let facts = read(xml.as_bytes(), "Bus").unwrap();
        assert_eq!(
            facts
                .items
                .unwrap()
                .iter()
                .map(|i| i.name.as_str())
                .collect::<Vec<_>>(),
            ["B"]
        );
        assert_eq!(facts.unavailable_names, ["A"]);
        assert!(!facts.limits.is_empty());
    }
}
