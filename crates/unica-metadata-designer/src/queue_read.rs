//! Чтение очереди (`Queue`, линия 8.5.4 и позже): синоним, свойства
//! обработки и дочерние — измерения и реквизиты. Комментарий читает общий
//! путь дескриптора.
//!
//! По устройству очередь ближе к регистру сведений, чем к справочнику:
//! её сообщение — запись с измерениями и реквизитами, а не ссылка.
use crate::descriptor_fields::{single, synonym, MD};
use roxmltree::{Document, Node};
use serde_json::{json, Value};

/// Свойства очереди, которые читает профиль, с их видом значения.
const PROPERTIES: &[(&str, Scalar)] = &[
    ("Transactioned", Scalar::Boolean),
    ("ProcessingPortionSize", Scalar::UnsignedInteger),
    ("AutoProcessing", Scalar::Boolean),
];

#[derive(Clone, Copy)]
enum Scalar {
    Boolean,
    UnsignedInteger,
}

fn scalar(node: Node<'_, '_>, shape: Scalar) -> Result<Value, String> {
    let text = node.text().unwrap_or_default();
    if node.children().any(|child| child.is_element()) {
        return Err(format!("expected a scalar, found {text:?}"));
    }
    match shape {
        Scalar::Boolean => match text {
            "true" => Ok(Value::Bool(true)),
            "false" => Ok(Value::Bool(false)),
            _ => Err(format!("expected true or false, found {text:?}")),
        },
        Scalar::UnsignedInteger => text
            .parse::<u32>()
            .map(|value| json!(value))
            .map_err(|_| format!("expected a whole number, found {text:?}")),
    }
}

/// Один потомок очереди: имя, синоним и тип значения.
fn member(node: Node<'_, '_>, index: usize, field: &str) -> (Value, Vec<Value>) {
    let mut unreadable = Vec::new();
    let mut item = json!({});
    let at = |suffix: &str| format!("{field}[{index}]{suffix}");
    let properties = match single(node, MD, "Properties") {
        Ok(Some(properties)) => properties,
        Ok(None) => {
            unreadable.push(json!({"field":at(""),"message":"Properties are absent"}));
            return (item, unreadable);
        }
        Err(message) => {
            unreadable.push(json!({"field":at(""),"message":message}));
            return (item, unreadable);
        }
    };
    match single(properties, MD, "Name").map(|node| node.and_then(|node| node.text())) {
        Ok(Some(name)) => item["name"] = Value::String(name.to_string()),
        Ok(None) => unreadable.push(json!({"field":at(".name"),"message":"Name is absent"})),
        Err(message) => unreadable.push(json!({"field":at(".name"),"message":message})),
    }
    match single(properties, MD, "Synonym").and_then(|node| node.map(synonym).transpose()) {
        Ok(Some(synonym)) => item["synonym"] = Value::String(synonym),
        Ok(None) => {}
        Err(message) => unreadable.push(json!({"field":at(".synonym"),"message":message})),
    }
    match crate::info_projection::parse_observed_metadata_type_node(properties) {
        Ok(observed) => match serde_json::to_value(observed) {
            Ok(value) => item["type"] = value,
            Err(error) => unreadable.push(json!({"field":at(".type"),"message":error.to_string()})),
        },
        Err(diagnostic) => {
            unreadable.push(json!({"field":at(".type"),"message":diagnostic.message}))
        }
    }
    (item, unreadable)
}

/// Дополняет ответ чтения очереди её синонимом, свойствами обработки
/// и дочерними. Нечитаемое поле попадает в `unreadable`, остальные читаются.
pub fn enrich_queue(raw: &[u8], payload: &mut Value) -> Result<(), String> {
    let text = std::str::from_utf8(raw).map_err(|error| error.to_string())?;
    let document = Document::parse(text).map_err(|error| error.to_string())?;
    let owner = document
        .root_element()
        .children()
        .find(|node| node.has_tag_name((MD, "Queue")))
        .ok_or("Queue owner is absent")?;
    let mut unreadable = Vec::new();
    let mut properties = Vec::new();
    match single(owner, MD, "Properties") {
        Ok(Some(owned)) => {
            match single(owned, MD, "Synonym").and_then(|node| node.map(synonym).transpose()) {
                Ok(Some(synonym)) => payload["synonym"] = Value::String(synonym),
                Ok(None) => {}
                Err(message) => unreadable.push(json!({"field":"synonym","message":message})),
            }
            for &(key, shape) in PROPERTIES {
                match single(owned, MD, key)
                    .and_then(|node| node.map(|node| scalar(node, shape)).transpose())
                {
                    Ok(Some(value)) => properties.push(json!({"key":key,"value":value})),
                    Ok(None) => {}
                    Err(message) => unreadable
                        .push(json!({"field":format!("properties.{key}"),"message":message})),
                }
            }
        }
        Ok(None) => {}
        Err(message) => unreadable.push(json!({"field":"properties","message":message})),
    }
    payload["properties"] = Value::Array(properties);
    let children = match single(owner, MD, "ChildObjects") {
        Ok(children) => children,
        Err(message) => {
            unreadable.push(json!({"field":"collections","message":message}));
            None
        }
    };
    for (tag, field) in [
        ("Dimension", "collections.dimensions"),
        ("Attribute", "collections.attributes"),
    ] {
        let mut items = Vec::new();
        for (index, node) in children
            .into_iter()
            .flat_map(|children| children.children())
            .filter(|node| node.has_tag_name((MD, tag)))
            .enumerate()
        {
            let (item, findings) = member(node, index, field);
            items.push(item);
            unreadable.extend(findings);
        }
        let key = field.rsplit('.').next().expect("field names a collection");
        payload["collections"][key] = Value::Array(items);
    }
    if !unreadable.is_empty() {
        payload["unreadable"] = Value::Array(unreadable);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXPORT: &str = include_str!(
        "../../../tests/fixtures/formats/platform_xml/2.22/all_kinds/src/Queues/Probe.xml"
    );

    /// Очередь выгрузки 8.5.4 читается синонимом, свойствами обработки,
    /// измерением и реквизитом с их типами.
    #[test]
    fn a_queue_of_the_platform_export_shows_its_properties_and_children() {
        let mut payload = json!({});
        enrich_queue(EXPORT.as_bytes(), &mut payload).unwrap();
        assert_eq!(payload["synonym"], "Проба");
        assert_eq!(
            payload["properties"],
            json!([
                {"key":"Transactioned","value":true},
                {"key":"ProcessingPortionSize","value":1000},
                {"key":"AutoProcessing","value":true},
            ])
        );
        assert!(payload.get("unreadable").is_none(), "{payload}");

        let dimensions = payload["collections"]["dimensions"].as_array().unwrap();
        assert_eq!(dimensions.len(), 1);
        assert_eq!(dimensions[0]["name"], "Измерение");
        assert_eq!(dimensions[0]["synonym"], "Измерение");
        assert_eq!(dimensions[0]["type"]["variants"][0]["kind"], "string");
        let attributes = payload["collections"]["attributes"].as_array().unwrap();
        assert_eq!(attributes.len(), 1);
        assert_eq!(attributes[0]["name"], "Реквизит");
        assert_eq!(attributes[0]["type"]["variants"][0]["kind"], "number");
    }

    /// Испорченное свойство обработки нечитаемо, остальные читаются.
    #[test]
    fn a_damaged_processing_property_is_unreadable_and_the_rest_are_kept() {
        let raw = EXPORT.replace(
            "<ProcessingPortionSize>1000</ProcessingPortionSize>",
            "<ProcessingPortionSize>много</ProcessingPortionSize>",
        );
        let mut payload = json!({});
        enrich_queue(raw.as_bytes(), &mut payload).unwrap();
        assert_eq!(
            payload["properties"],
            json!([
                {"key":"Transactioned","value":true},
                {"key":"AutoProcessing","value":true},
            ])
        );
        assert_eq!(
            payload["unreadable"][0]["field"],
            "properties.ProcessingPortionSize"
        );
        assert_eq!(
            payload["collections"]["dimensions"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }

    /// Очередь без дочерних читается пустыми коллекциями, а не отказом:
    /// «пусто» отличается от «вид не поддержан».
    #[test]
    fn a_queue_without_children_reads_empty_collections() {
        let start = EXPORT.find("<ChildObjects>").unwrap();
        let end = EXPORT.find("</ChildObjects>").unwrap() + "</ChildObjects>".len();
        let raw = format!("{}{}", &EXPORT[..start], &EXPORT[end..]);
        let mut payload = json!({});
        enrich_queue(raw.as_bytes(), &mut payload).unwrap();
        assert_eq!(payload["collections"]["dimensions"], json!([]));
        assert_eq!(payload["collections"]["attributes"], json!([]));
        assert!(payload.get("unreadable").is_none(), "{payload}");
    }
}
