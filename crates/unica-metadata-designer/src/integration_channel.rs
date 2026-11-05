//! Existing integration channels: namespace-aware observations and local scalar patches.
use roxmltree::{Document, Node};
use serde_json::{json, Value};
use unica_metadata::property_input::IntegrationServiceChannelChanges;

const MD: &str = "http://v8.1c.ru/8.3/MDClasses";
const FIELDS: [&str; 4] = [
    "ExternalIntegrationServiceChannelName",
    "MessageDirection",
    "ReceiveMessageProcessing",
    "Transactioned",
];

fn one<'a, 'i>(parent: Node<'a, 'i>, tag: &str) -> Result<Node<'a, 'i>, String> {
    let nodes: Vec<_> = parent
        .children()
        .filter(|n| n.is_element() && n.tag_name().name() == tag)
        .collect();
    match nodes.as_slice() {
        [node] if node.has_tag_name((MD, tag)) && node.attributes().len() == 0 => Ok(*node),
        _ => Err(format!("{tag} requires one direct metadata element")),
    }
}

fn scalar(node: Node<'_, '_>) -> Result<String, String> {
    if node.children().any(|n| !n.is_text()) {
        return Err(format!("{} must contain only text", node.tag_name().name()));
    }
    Ok(node.children().filter_map(|n| n.text()).collect())
}

fn owner<'a, 'i>(doc: &'a Document<'i>, expected: &str) -> Result<Node<'a, 'i>, String> {
    let root = doc.root_element();
    if !root.has_tag_name((MD, "MetaDataObject")) {
        return Err("integration service has the wrong XML root".into());
    }
    let children: Vec<_> = root.children().filter(Node::is_element).collect();
    let [service] = children.as_slice() else {
        return Err("integration service requires one owner".into());
    };
    if !service.has_tag_name((MD, "IntegrationService"))
        || scalar(one(one(*service, "Properties")?, "Name")?)? != expected
    {
        return Err("integration service identity does not match its target".into());
    }
    Ok(*service)
}

fn channels<'a, 'i>(service: Node<'a, 'i>) -> Result<Vec<(String, Node<'a, 'i>)>, String> {
    let container = one(service, "ChildObjects")?;
    let mut result = Vec::new();
    for child in container
        .children()
        .filter(|n| n.is_element() && n.tag_name().name() == "IntegrationServiceChannel")
    {
        if !child.has_tag_name((MD, "IntegrationServiceChannel")) {
            return Err("integration channel has a foreign namespace".into());
        }
        let name = scalar(one(one(child, "Properties")?, "Name")?)?;
        if name.is_empty() || result.iter().any(|(seen, _)| seen == &name) {
            return Err("integration channel names must be nonempty and unique".into());
        }
        result.push((name, child));
    }
    Ok(result)
}

fn flag(value: &str) -> Result<bool, String> {
    match value {
        "true" | "1" => Ok(true),
        "false" | "0" => Ok(false),
        _ => Err("Transactioned must be an XML boolean".into()),
    }
}

/// A damaged optional scalar does not erase the channel or its readable peers.
pub fn read(bytes: &[u8], expected: &str) -> Result<(Vec<Value>, Vec<Value>), String> {
    let source = std::str::from_utf8(bytes).map_err(|e| e.to_string())?;
    let doc = Document::parse(source.trim_start_matches('\u{feff}')).map_err(|e| e.to_string())?;
    let mut rows = Vec::new();
    let mut errors = Vec::new();
    for (name, node) in channels(owner(&doc, expected)?)? {
        let properties = one(node, "Properties")?;
        let mut row = json!({"name":name});
        for field in FIELDS {
            let value = one(properties, field).and_then(scalar).and_then(|value| {
                if field == "Transactioned" {
                    flag(&value).map(Value::Bool)
                } else if field == "MessageDirection"
                    && !matches!(value.as_str(), "Receive" | "Send")
                {
                    Err("MessageDirection must be Receive or Send".into())
                } else {
                    Ok(Value::String(value))
                }
            });
            match value {
                Ok(value) => row[field] = value,
                Err(message) => errors.push(
                    json!({"field":format!("details.channels[{name}].{field}"),"message":message}),
                ),
            }
        }
        rows.push(row);
    }
    Ok((rows, errors))
}

pub struct ChannelPatch {
    pub text: String,
    /// Only a changed binding (or activation of a receive channel) needs BSL facts.
    pub handler_to_validate: Option<String>,
}

pub fn patch(
    source: &str,
    service: &str,
    channel: &str,
    changes: &IntegrationServiceChannelChanges,
) -> Result<ChannelPatch, String> {
    let doc = Document::parse(source).map_err(|e| e.to_string())?;
    let members = channels(owner(&doc, service)?)?;
    let (_, node) = members
        .iter()
        .find(|(name, _)| name == channel)
        .ok_or("integration channel was not found")?;
    let properties = one(*node, "Properties")?;
    let old_direction = scalar(one(properties, "MessageDirection")?)?;
    let direction = changes.direction.as_deref().unwrap_or(&old_direction);
    if !matches!(direction, "Receive" | "Send") {
        return Err("MessageDirection must be Receive or Send".into());
    }
    let old_handler = scalar(one(properties, "ReceiveMessageProcessing")?)?;
    let handler = changes.handler.as_deref().unwrap_or(&old_handler);
    let handler_to_validate = (!handler.is_empty()
        && (changes.handler.is_some() || (old_direction != direction && direction == "Receive")))
        .then(|| handler.to_string());
    let mut replacements = Vec::new();
    let transactioned = changes.transactioned.map(|v| v.to_string());
    for (field, value) in FIELDS.into_iter().zip([
        changes.external_name.as_deref(),
        changes.direction.as_deref(),
        changes.handler.as_deref(),
        transactioned.as_deref(),
    ]) {
        let Some(value) = value else {
            continue;
        };
        let field_node = one(properties, field)?;
        let old = scalar(field_node)?;
        if old == value || (field == "Transactioned" && flag(&old)? == flag(value)?) {
            continue;
        }
        let range = field_node.range();
        let start = &source[range.start + 1..];
        let end = start
            .find(|c: char| c.is_whitespace() || c == '/' || c == '>')
            .ok_or("invalid scalar tag")?;
        let qname = &start[..end];
        let escaped = unica_xml_format::text::escape_xml_text(value).replace('\r', "&#13;");
        let fragment = &source[range.clone()];
        let opening_end = crate::predefined::opening_tag_end(fragment)?;
        let opening = &fragment[..opening_end];
        let opening = opening.strip_suffix('/').unwrap_or(opening);
        replacements.push((range, format!("{opening}>{escaped}</{qname}>")));
    }
    replacements.sort_by_key(|(range, _)| range.start);
    let mut text = source.to_string();
    for (range, replacement) in replacements.into_iter().rev() {
        text.replace_range(range, &replacement);
    }
    Document::parse(&text).map_err(|e| e.to_string())?;
    Ok(ChannelPatch {
        text,
        handler_to_validate,
    })
}

pub fn validate_handler(source: &str, name: &str) -> Result<(), String> {
    let parsed = unica_bsl_parser::parse(source);
    if !parsed.errors().is_empty() {
        return Err("integration service module contains syntax errors".into());
    }
    let matches: Vec<_> = parsed
        .methods()
        .into_iter()
        .filter(|m| {
            m.name
                .as_ref()
                .is_some_and(|n| n.to_lowercase() == name.to_lowercase())
        })
        .collect();
    match matches.as_slice() {
        [method] if method.kind == unica_bsl_parser::CodeOutlineMethodKind::Procedure
            && method.async_start.is_none()
            && method.parameters.as_ref().is_ok_and(|params| params.len() == 2) => Ok(()),
        _ => Err("ReceiveMessageProcessing must name a unique two-parameter procedure in the integration service module".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image() -> String {
        "<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.20\"><IntegrationService uuid=\"owner\"><Properties><Name>Bus</Name></Properties><ChildObjects><IntegrationServiceChannel uuid=\"in\"><InternalInfo/><Properties><Name>In</Name><Transactioned>1</Transactioned><MessageDirection>Receive</MessageDirection><ReceiveMessageProcessing>Handle</ReceiveMessageProcessing><ExternalIntegrationServiceChannelName>A&amp;B</ExternalIntegrationServiceChannelName></Properties></IntegrationServiceChannel><IntegrationServiceChannel uuid=\"out\"><Properties><Name>Out</Name><ExternalIntegrationServiceChannelName>out</ExternalIntegrationServiceChannelName><MessageDirection>Send</MessageDirection><ReceiveMessageProcessing/><Transactioned>true</Transactioned></Properties></IntegrationServiceChannel></ChildObjects></IntegrationService></MetaDataObject>".into()
    }
    fn changes() -> IntegrationServiceChannelChanges {
        IntegrationServiceChannelChanges {
            external_name: None,
            direction: None,
            handler: None,
            transactioned: None,
        }
    }
    #[test]
    fn integration_channel_patches_only_addressed_scalars_and_keeps_equivalent_bytes() {
        let source = image();
        let mut values = changes();
        values.external_name = Some("A&B".into());
        values.transactioned = Some(true);
        assert_eq!(patch(&source, "Bus", "In", &values).unwrap().text, source);
        values.external_name = Some("<new>\r&".into());
        values.transactioned = Some(false);
        values.direction = Some("Send".into());
        let updated = patch(&source, "Bus", "In", &values).unwrap();
        assert!(updated.handler_to_validate.is_none());
        assert_eq!(
            updated.text,
            source
                .replace(
                    "<Transactioned>1</Transactioned>",
                    "<Transactioned>false</Transactioned>"
                )
                .replace(
                    "<MessageDirection>Receive</MessageDirection>",
                    "<MessageDirection>Send</MessageDirection>"
                )
                .replace("A&amp;B", "&lt;new&gt;&#13;&amp;")
        );
        let (rows, errors) = read(updated.text.as_bytes(), "Bus").unwrap();
        assert!(errors.is_empty());
        assert_eq!(rows[0]["ExternalIntegrationServiceChannelName"], "<new>\r&");
        assert_eq!(rows[1]["Transactioned"], true);
        assert_eq!(rows[1]["ReceiveMessageProcessing"], "");
        values.handler = Some("".into());
        assert!(patch(&source, "Bus", "In", &values)
            .unwrap()
            .handler_to_validate
            .is_none());
    }
    #[test]
    fn integration_channel_preserves_local_namespaces_and_unchanged_direction() {
        let source = image().replace(
            "<Transactioned>1</Transactioned>",
            "<p:Transactioned xmlns:p=\"http://v8.1c.ru/8.3/MDClasses\">1</p:Transactioned>",
        );
        let mut values = changes();
        values.transactioned = Some(false);
        let updated = patch(&source, "Bus", "In", &values).unwrap();
        assert_eq!(
            updated.text,
            source.replace(">1</p:Transactioned>", ">false</p:Transactioned>")
        );
        values = changes();
        values.direction = Some("Receive".into());
        let unchanged = patch(&source, "Bus", "In", &values).unwrap();
        assert_eq!(unchanged.text, source);
        assert!(unchanged.handler_to_validate.is_none());
    }
    #[test]
    fn integration_channel_rejects_ambiguous_targets_and_retains_partial_scalar_facts() {
        let source = image();
        for bad in [
            source.replace("<Name>In</Name>", "<Name>In</Name><Name>In</Name>"),
            source.replace("<Name>Out</Name>", "<Name>In</Name>"),
            source.replace("<ChildObjects>", "<ChildObjects xmlns=\"foreign\">"),
        ] {
            assert!(patch(&bad, "Bus", "In", &changes()).is_err());
        }
        let bad = source.replace(
            "<Transactioned>1</Transactioned>",
            "<Transactioned><Nested/></Transactioned>",
        );
        let (rows, errors) = read(bad.as_bytes(), "Bus").unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(errors.len(), 1);
        assert_eq!(rows[0]["MessageDirection"], "Receive");
        assert!(rows[0].get("Transactioned").is_none());
        let mut values = changes();
        values.transactioned = Some(true);
        assert!(patch(&bad, "Bus", "In", &values).is_err());
        assert!(patch(&source, "Wrong", "In", &values).is_err());
        assert!(patch(&source, "Bus", "Missing", &values).is_err());
        values.external_name = Some("bad\u{b}".into());
        assert!(patch(&source, "Bus", "In", &values).is_err());
    }
    #[test]
    fn integration_channel_handler_is_a_unique_two_parameter_procedure_without_export_requirement()
    {
        assert!(validate_handler(
            "Процедура Обработать(Сообщение, Отказ)\nКонецПроцедуры",
            "ОБРАБОТАТЬ"
        )
        .is_ok());
        for source in [
            "Процедура Other(A, B)\nКонецПроцедуры",
            "Процедура Handle(A)\nКонецПроцедуры",
            "Функция Handle(A, B)\nКонецФункции",
            "Процедура Handle(A, B)\nКонецПроцедуры\nПроцедура HANDLE(A, B)\nКонецПроцедуры",
            "Процедура Handle(A, B)\nЕсли\nКонецПроцедуры",
        ] {
            assert!(validate_handler(source, "Handle").is_err(), "{source}");
        }
    }
}
