//! Чтение цвета палитры (`PaletteColor`, линия 8.5.1 и позже): синоним
//! и цвет `#RRGGBB`. Комментарий читает общий путь дескриптора.
use crate::descriptor_fields::{single, synonym, MD};
use roxmltree::{Document, Node};
use serde_json::{json, Value};

/// Цвет палитры записан `#RRGGBB`.
fn color(node: Node<'_, '_>) -> Result<String, String> {
    let text = node.text().unwrap_or_default();
    let digits = text.strip_prefix('#').unwrap_or_default();
    if node.children().any(|child| child.is_element())
        || digits.len() != 6
        || !digits.chars().all(|digit| digit.is_ascii_hexdigit())
    {
        return Err(format!("expected #RRGGBB, found {text:?}"));
    }
    Ok(text.to_string())
}

/// Дополняет ответ чтения цвета палитры синонимом и свойством `Color`.
/// Нечитаемое поле попадает в `unreadable`, остальные читаются.
pub fn enrich_palette_color(raw: &[u8], payload: &mut Value) -> Result<(), String> {
    let text = std::str::from_utf8(raw).map_err(|error| error.to_string())?;
    let document = Document::parse(text).map_err(|error| error.to_string())?;
    let owner = document
        .root_element()
        .children()
        .find(|node| node.has_tag_name((MD, "PaletteColor")))
        .ok_or("PaletteColor owner is absent")?;
    let mut unreadable = Vec::new();
    let mut properties = Vec::new();
    match single(owner, MD, "Properties") {
        Ok(Some(owned)) => {
            match single(owned, MD, "Synonym").and_then(|node| node.map(synonym).transpose()) {
                Ok(Some(synonym)) => payload["synonym"] = Value::String(synonym),
                Ok(None) => {}
                Err(message) => unreadable.push(json!({"field":"synonym","message":message})),
            }
            match single(owned, MD, "Color").and_then(|node| node.map(color).transpose()) {
                Ok(Some(color)) => properties.push(json!({"key":"Color","value":color})),
                Ok(None) => {}
                Err(message) => {
                    unreadable.push(json!({"field":"properties.Color","message":message}))
                }
            }
        }
        Ok(None) => {}
        Err(message) => unreadable.push(json!({"field":"properties","message":message})),
    }
    payload["properties"] = Value::Array(properties);
    if !unreadable.is_empty() {
        payload["unreadable"] = Value::Array(unreadable);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Цвет палитры выгрузки 8.5.1 и 8.5.4 читается синонимом и цветом.
    #[test]
    fn a_palette_color_of_the_platform_export_shows_its_synonym_and_color() {
        for raw in [
            include_str!("../../../tests/fixtures/formats/platform_xml/2.21/all_kinds/src/PaletteColors/Probe.xml"),
            include_str!("../../../tests/fixtures/formats/platform_xml/2.22/all_kinds/src/PaletteColors/Probe.xml"),
        ] {
            let mut payload = json!({});
            enrich_palette_color(raw.as_bytes(), &mut payload).unwrap();
            assert_eq!(payload["synonym"], "Проба");
            assert_eq!(
                payload["properties"],
                json!([{"key":"Color","value":"#1A2B3C"}])
            );
            assert!(payload.get("unreadable").is_none(), "{payload}");
        }
    }

    /// Цвет не в виде `#RRGGBB` нечитаем, синоним при этом читается.
    #[test]
    fn a_damaged_color_is_unreadable_and_the_synonym_is_kept() {
        let raw = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core"><PaletteColor><Properties><Name>X</Name><Synonym/><Comment/><Color>red</Color></Properties></PaletteColor></MetaDataObject>"#;
        let mut payload = json!({});
        enrich_palette_color(raw.as_bytes(), &mut payload).unwrap();
        assert_eq!(payload["synonym"], "");
        assert_eq!(payload["properties"], json!([]));
        assert_eq!(payload["unreadable"][0]["field"], "properties.Color");
    }
}
