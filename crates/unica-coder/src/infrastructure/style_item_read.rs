//! Explicit StyleItem observations, independent of the metadata writer algebra.
use roxmltree::{Document, Node};
use serde_json::{json, Number, Value};

const MD: &str = "http://v8.1c.ru/8.3/MDClasses";
const UI: &str = "http://v8.1c.ru/8.1/data/ui";
const CORE: &str = "http://v8.1c.ru/8.1/data/core";
const XSI: &str = "http://www.w3.org/2001/XMLSchema-instance";

#[derive(Default)]
struct FontValue {
    reference: Option<String>,
    kind: Option<String>,
    face_name: Option<String>,
    height: Option<Number>,
    scale: Option<Number>,
    bold: Option<bool>,
    italic: Option<bool>,
    underline: Option<bool>,
    strikeout: Option<bool>,
}

enum StyleValue {
    Font(FontValue),
    Border {
        width: Option<Number>,
        style: Option<String>,
    },
    Color(String),
}

#[derive(Default)]
struct StyleItem {
    synonym: Option<String>,
    item_type: Option<String>,
    value: Option<StyleValue>,
    unreadable: Vec<Value>,
}

impl StyleItem {
    fn damaged(&mut self, field: &str, message: impl Into<String>) {
        self.unreadable
            .push(json!({"field":field,"message":message.into()}));
    }

    fn child<'a, 'input>(
        &mut self,
        node: Node<'a, 'input>,
        ns: &str,
        name: &str,
        field: &str,
    ) -> Option<Node<'a, 'input>> {
        let mut children = node
            .children()
            .filter(|child| child.has_tag_name((ns, name)));
        let first = children.next();
        if children.next().is_some() {
            self.damaged(field, format!("duplicate {name}"));
            None
        } else {
            first
        }
    }

    fn text(&mut self, node: Node<'_, '_>, field: &str) -> Option<String> {
        if node.children().any(|child| child.is_element())
            || node
                .attribute((XSI, "nil"))
                .is_some_and(|nil| !matches!(nil.trim(), "false" | "0"))
        {
            self.damaged(field, "expected explicit text value");
            return None;
        }
        Some(node.text().unwrap_or_default().to_owned())
    }

    fn typed(&mut self, node: Node<'_, '_>, ns: &str, local: &str, field: &str) -> bool {
        if let Some(nil) = node.attribute((XSI, "nil")) {
            if !matches!(nil.trim(), "false" | "0") {
                self.damaged(
                    field,
                    format!("xsi:nil={nil:?}; no explicit {local} fields can be read"),
                );
                return false;
            }
        }
        let matches = node.attribute((XSI, "type")).is_some_and(|raw| {
            let (prefix, name) = raw
                .split_once(':')
                .map_or((None, raw), |(prefix, name)| (Some(prefix), name));
            name == local && node.lookup_namespace_uri(prefix) == Some(ns)
        });
        if !matches {
            self.damaged(field, format!("expected {{{ns}}}{local} xsi:type"));
        }
        matches
    }

    fn flag(&mut self, node: Node<'_, '_>, name: &str, field: &str) -> Option<bool> {
        match node.attribute(name)?.trim() {
            "true" | "1" => Some(true),
            "false" | "0" => Some(false),
            _ => {
                self.damaged(field, "invalid boolean attribute");
                None
            }
        }
    }

    fn number(&mut self, node: Node<'_, '_>, name: &str, field: &str) -> Option<Number> {
        let raw = node.attribute(name)?.trim();
        let number = raw
            .parse::<i64>()
            .ok()
            .map(Number::from)
            .or_else(|| raw.parse::<f64>().ok().and_then(Number::from_f64));
        if number.is_none() {
            self.damaged(field, "invalid numeric attribute");
        }
        number
    }

    fn read(raw: &[u8]) -> Result<Self, String> {
        let text = std::str::from_utf8(raw).map_err(|error| error.to_string())?;
        let document = Document::parse(text).map_err(|error| error.to_string())?;
        let mut item = Self::default();
        let owner = document
            .root_element()
            .children()
            .find(|node| node.has_tag_name((MD, "StyleItem")))
            .ok_or("StyleItem owner is absent")?;
        let Some(properties) = item.child(owner, MD, "Properties", "properties") else {
            return Ok(item);
        };
        if let Some(synonym) = item.child(properties, MD, "Synonym", "synonym") {
            // Match the metadata node's existing preferred-language title convention.
            let entries = synonym
                .children()
                .filter(|node| node.has_tag_name((CORE, "item")))
                .collect::<Vec<_>>();
            let preferred = entries
                .iter()
                .find(|node| {
                    node.children()
                        .any(|node| node.has_tag_name((CORE, "lang")) && node.text() == Some("ru"))
                })
                .or(entries.first());
            if let Some(entry) = preferred {
                if let Some(content) = item.child(*entry, CORE, "content", "synonym") {
                    item.synonym = item.text(content, "synonym");
                }
            } else if !synonym.children().any(|node| node.is_element()) {
                item.synonym = Some(String::new());
            } else {
                item.damaged("synonym", "invalid localized synonym");
            }
        }
        if let Some(kind) = item.child(properties, MD, "Type", "properties.Type") {
            item.item_type = item.text(kind, "properties.Type");
        }
        let Some(value) = item.child(properties, MD, "Value", "properties.Value") else {
            return Ok(item);
        };
        match item.item_type.clone().as_deref() {
            Some("Font") if item.typed(value, UI, "Font", "properties.Value") => {
                if value.children().any(|node| {
                    node.is_element() && matches!(node.tag_name().namespace(), Some(UI | MD))
                }) || value.text().is_some_and(|text| !text.trim().is_empty())
                {
                    item.damaged("properties.Value", "Font must contain attributes only");
                    return Ok(item);
                }
                item.value = Some(StyleValue::Font(FontValue {
                    reference: value.attribute("ref").map(str::to_owned),
                    kind: value.attribute("kind").map(str::to_owned),
                    face_name: value.attribute("faceName").map(str::to_owned),
                    height: item.number(value, "height", "properties.fontHeight"),
                    scale: item.number(value, "scale", "properties.fontScale"),
                    bold: item.flag(value, "bold", "properties.fontBold"),
                    italic: item.flag(value, "italic", "properties.fontItalic"),
                    underline: item.flag(value, "underline", "properties.fontUnderline"),
                    strikeout: item.flag(value, "strikeout", "properties.fontStrikeout"),
                }));
            }
            Some("Border") if item.typed(value, UI, "Border", "properties.Value") => {
                let width = item.number(value, "width", "properties.borderWidth");
                let style = item
                    .child(value, UI, "style", "properties.borderStyle")
                    .filter(|node| {
                        item.typed(*node, UI, "ControlBorderType", "properties.borderStyle")
                    })
                    .and_then(|node| item.text(node, "properties.borderStyle"));
                item.value = Some(StyleValue::Border { width, style });
            }
            Some("Color") if item.typed(value, UI, "Color", "properties.Value") => {
                item.value = item.text(value, "properties.Value").map(StyleValue::Color);
            }
            Some("Font" | "Border" | "Color") => {}
            _ => item.damaged(
                "properties.Value",
                "Value type is outside the observed StyleItem profile",
            ),
        }
        Ok(item)
    }

    fn publish(self, payload: &mut Value) {
        let mut properties = Vec::new();
        let mut add = |key: &str, value: Option<Value>| {
            if let Some(value) = value {
                properties.push(json!({"key":key,"value":value}));
            }
        };
        add("Type", self.item_type.map(Value::String));
        match self.value {
            Some(StyleValue::Font(font)) => {
                add("fontRef", font.reference.map(Value::String));
                add("fontKind", font.kind.map(Value::String));
                add("fontFaceName", font.face_name.map(Value::String));
                add("fontHeight", font.height.map(Value::Number));
                add("fontScale", font.scale.map(Value::Number));
                add("fontBold", font.bold.map(Value::Bool));
                add("fontItalic", font.italic.map(Value::Bool));
                add("fontUnderline", font.underline.map(Value::Bool));
                add("fontStrikeout", font.strikeout.map(Value::Bool));
            }
            Some(StyleValue::Border { width, style }) => {
                add("borderWidth", width.map(Value::Number));
                add("borderStyle", style.map(Value::String));
            }
            Some(StyleValue::Color(value)) => add("Value", Some(Value::String(value))),
            None => {}
        }
        payload["properties"] = Value::Array(properties);
        if let Some(synonym) = self.synonym {
            payload["synonym"] = Value::String(synonym);
        }
        if !self.unreadable.is_empty() {
            payload["unreadable"] = Value::Array(self.unreadable);
        }
    }
}

pub(crate) fn enrich_style_item(raw: &[u8], payload: &mut Value) -> Result<(), String> {
    StyleItem::read(raw)?.publish(payload);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observed(body: &str) -> Value {
        let raw = format!(
            r#"<MetaDataObject xmlns="{MD}" xmlns:ui="{UI}" xmlns:xsi="{XSI}" xmlns:vendor="urn:vendor" version="2.20"><StyleItem><Properties>{body}</Properties></StyleItem></MetaDataObject>"#
        );
        let mut payload = json!({});
        enrich_style_item(raw.as_bytes(), &mut payload).unwrap();
        payload
    }

    fn property<'a>(payload: &'a Value, key: &str) -> Option<&'a Value> {
        payload["properties"]
            .as_array()
            .unwrap()
            .iter()
            .find(|property| property["key"] == key)
            .map(|property| &property["value"])
    }

    #[test]
    fn styleitem_color_and_partial_border_keep_independent_observations() {
        let color = observed(
            "<Type>Color</Type><Value xsi:type=\"ui:Color\" xsi:nil=\"false\">#F0F0F0</Value>",
        );
        assert_eq!(property(&color, "Value"), Some(&json!("#F0F0F0")));
        let border = observed("<Type>Border</Type><Value xsi:type=\"ui:Border\" width=\"broken\"><ui:style xsi:type=\"ui:ControlBorderType\" xsi:nil=\"0\">Single</ui:style></Value>");
        assert_eq!(property(&border, "borderStyle"), Some(&json!("Single")));
        assert_eq!(property(&border, "borderWidth"), None);
        assert_eq!(border["unreadable"][0]["field"], "properties.borderWidth");
        let duplicate = observed("<Type>Border</Type><Value xsi:type=\"ui:Border\" width=\"1\"><ui:style xsi:type=\"ui:ControlBorderType\" xsi:nil=\"0\">Single</ui:style><ui:style xsi:type=\"ui:ControlBorderType\">Double</ui:style></Value>");
        assert_eq!(property(&duplicate, "borderWidth"), Some(&json!(1)));
        assert_eq!(property(&duplicate, "borderStyle"), None);
        assert_eq!(
            duplicate["unreadable"][0]["field"],
            "properties.borderStyle"
        );
    }

    #[test]
    fn styleitem_nil_and_vendor_fields_do_not_invent_font_overrides() {
        let nil = observed(
            "<Type>Font</Type><Value xsi:type=\"ui:Font\" xsi:nil=\"true\" bold=\"true\"/>",
        );
        assert_eq!(property(&nil, "fontBold"), None);
        assert!(nil["unreadable"].is_array());
        let vendor = observed("<Type>Font</Type><Value xsi:type=\"ui:Font\" bold=\"false\" vendor:bold=\"true\"><vendor:extra>private</vendor:extra></Value>");
        assert_eq!(property(&vendor, "fontBold"), Some(&json!(false)));
        assert!(vendor.get("unreadable").is_none());
        let foreign_style = observed("<Type>Border</Type><Value xsi:type=\"ui:Border\" width=\"1\"><vendor:style>Single</vendor:style></Value>");
        assert_eq!(property(&foreign_style, "borderWidth"), Some(&json!(1)));
        assert_eq!(property(&foreign_style, "borderStyle"), None);
    }
}
