//! Запись очереди: меняется только текст названного свойства обработки,
//! прочие байты дескриптора остаются.
use crate::descriptor_fields::MD;
use roxmltree::Document;

/// Ставит свойству `key` очереди `name` текст `value`. То же значение
/// оставляет дескриптор байт в байт.
pub fn patch_property(source: &str, name: &str, key: &str, value: &str) -> Result<String, String> {
    let document = Document::parse(source).map_err(|error| error.to_string())?;
    let owner = document
        .root_element()
        .children()
        .find(|node| node.has_tag_name((MD, "Queue")))
        .ok_or("Queue owner is absent")?;
    let properties = owner
        .children()
        .find(|node| node.has_tag_name((MD, "Properties")))
        .ok_or("Queue Properties are absent")?;
    let named = properties
        .children()
        .find(|node| node.has_tag_name((MD, "Name")))
        .and_then(|node| node.text());
    if named != Some(name) {
        return Err(format!("Queue descriptor is not `{name}`"));
    }
    let mut found = properties
        .children()
        .filter(|node| node.has_tag_name((MD, key)));
    let node = found.next().ok_or(format!("Queue {key} is absent"))?;
    // Свойство обработки — один текстовый узел. Комментарий или разметка
    // рядом с ним не правится: иначе замена потеряла бы соседнее содержимое.
    let mut content = node.children();
    let text = content.next().ok_or(format!("Queue {key} is damaged"))?;
    if found.next().is_some() || content.next().is_some() || !text.is_text() {
        return Err(format!("Queue {key} is damaged"));
    }
    if node.text() == Some(value) {
        return Ok(source.to_string());
    }
    let mut patched = source.to_string();
    patched.replace_range(text.range(), value);
    Ok(patched)
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXPORT: &str = include_str!(
        "../../../tests/fixtures/formats/platform_xml/2.22/all_kinds/src/Queues/Probe.xml"
    );

    /// Новое значение меняет только текст свойства, то же значение оставляет
    /// выгрузку байт в байт, и обратная замена её возвращает.
    #[test]
    fn a_new_value_changes_only_the_property_text() {
        let patched = patch_property(EXPORT, "Probe", "ProcessingPortionSize", "500").unwrap();
        assert_eq!(
            patched,
            EXPORT.replace(
                "<ProcessingPortionSize>1000</ProcessingPortionSize>",
                "<ProcessingPortionSize>500</ProcessingPortionSize>"
            )
        );
        assert_eq!(
            patch_property(EXPORT, "Probe", "ProcessingPortionSize", "1000").unwrap(),
            EXPORT
        );
        assert_eq!(
            patch_property(&patched, "Probe", "ProcessingPortionSize", "1000").unwrap(),
            EXPORT
        );
        assert!(patch_property(EXPORT, "Other", "AutoProcessing", "false").is_err());
    }

    /// Свойство с лишним содержимым рядом с текстом и пустое свойство
    /// не правятся, а отвергаются.
    #[test]
    fn a_property_with_more_than_its_text_is_refused() {
        for damaged in [
            "<Transactioned><!--note-->true</Transactioned>",
            "<Transactioned>true<!--note--></Transactioned>",
            "<Transactioned/>",
        ] {
            let source = EXPORT.replace("<Transactioned>true</Transactioned>", damaged);
            assert!(
                patch_property(&source, "Probe", "Transactioned", "false").is_err(),
                "{damaged}"
            );
        }
    }
}
