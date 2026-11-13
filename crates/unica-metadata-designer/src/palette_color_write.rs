//! Запись цвета палитры: меняется только текст `Color` дескриптора, прочие
//! байты остаются.
use roxmltree::Document;

const MD: &str = "http://v8.1c.ru/8.3/MDClasses";

/// Ставит цвет `#RRGGBB` цвету палитры `name`. Тот же цвет в другом регистре
/// оставляет дескриптор байт в байт.
pub fn patch_color(source: &str, name: &str, color: &str) -> Result<String, String> {
    let document = Document::parse(source).map_err(|error| error.to_string())?;
    let owner = document
        .root_element()
        .children()
        .find(|node| node.has_tag_name((MD, "PaletteColor")))
        .ok_or("PaletteColor owner is absent")?;
    let properties = owner
        .children()
        .find(|node| node.has_tag_name((MD, "Properties")))
        .ok_or("PaletteColor Properties are absent")?;
    let named = properties
        .children()
        .find(|node| node.has_tag_name((MD, "Name")))
        .and_then(|node| node.text());
    if named != Some(name) {
        return Err(format!("PaletteColor descriptor is not `{name}`"));
    }
    let mut colors = properties
        .children()
        .filter(|node| node.has_tag_name((MD, "Color")));
    let node = colors.next().ok_or("PaletteColor Color is absent")?;
    if colors.next().is_some() || node.children().any(|child| child.is_element()) {
        return Err("PaletteColor Color is damaged".into());
    }
    let current = node.text().unwrap_or_default();
    if current.eq_ignore_ascii_case(color) {
        return Ok(source.to_string());
    }
    let range = match node.first_child() {
        Some(text) => text.range(),
        None => {
            // `<Color/>`: the element is rewritten whole.
            let range = node.range();
            let mut patched = source.to_string();
            patched.replace_range(range, &format!("<Color>{color}</Color>"));
            return Ok(patched);
        }
    };
    let mut patched = source.to_string();
    patched.replace_range(range, color);
    Ok(patched)
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXPORT: &str = include_str!(
        "../../../tests/fixtures/formats/platform_xml/2.22/all_kinds/src/PaletteColors/Probe.xml"
    );

    /// Новый цвет меняет только текст `Color`; тот же цвет в другом регистре
    /// оставляет выгрузку байт в байт, и обратная замена её возвращает.
    #[test]
    fn a_new_color_changes_only_the_color_text() {
        let patched = patch_color(EXPORT, "Probe", "#ff0000").unwrap();
        assert_eq!(
            patched,
            EXPORT.replace("<Color>#1A2B3C</Color>", "<Color>#ff0000</Color>")
        );
        assert_eq!(patch_color(EXPORT, "Probe", "#1a2b3c").unwrap(), EXPORT);
        assert_eq!(patch_color(&patched, "Probe", "#1A2B3C").unwrap(), EXPORT);
        assert!(patch_color(EXPORT, "Other", "#ff0000").is_err());
    }
}
