use roxmltree::Node;
use std::ops::Range;
use unica_metadata::{
    MetaDiagnostic, MetaDiagnosticCode, MetaObservedPropertyValue, MetadataInputSetting,
    MetadataInputValue, MetadataType,
};

fn opening_end(raw: &str) -> usize {
    let mut quote = None;
    raw.bytes()
        .enumerate()
        .find_map(|(index, byte)| {
            if let Some(delimiter) = quote {
                if byte == delimiter {
                    quote = None;
                }
            } else if matches!(byte, b'\'' | b'"') {
                quote = Some(byte);
            } else if byte == b'>' {
                return Some(index + 1);
            }
            None
        })
        .expect("parsed XML opening element")
}
fn qname(raw: &str) -> &str {
    raw[1..]
        .split(|c: char| c.is_whitespace() || c == '>' || c == '/')
        .next()
        .expect("parsed element name")
}
fn wrap_content(raw: &str, content: &str) -> String {
    let end = opening_end(raw);
    let open = &raw[..end];
    let open = if let Some(stripped) = open.strip_suffix("/>") {
        format!("{stripped}>")
    } else {
        open.to_owned()
    };
    format!("{open}{content}</{}>", qname(raw))
}

/// Existing properties only; ranges refer to the original descriptor, so the
/// caller can combine them with other staged replacements without re-reading.
pub(crate) fn replacements(
    source: &str,
    properties: Node<'_, '_>,
    ty: &MetadataType,
    settings: &[(MetadataInputSetting, MetadataInputValue)],
    lang: &str,
) -> Result<Vec<(Range<usize>, String)>, MetaDiagnostic> {
    let mut result = Vec::new();
    for (setting, value) in settings {
        let (xml, field) = setting.names();
        let refused = |message: String| {
            crate::edit::typed_diagnostic(
                MetaDiagnosticCode::InvalidArguments,
                message,
                Some(field),
            )
        };
        if !setting.allows(ty) {
            return Err(refused(format!("{xml} is not supported for this Type")));
        }
        let nodes: Vec<_> = properties
            .children()
            .filter(|n| n.is_element() && n.tag_name().name() == xml)
            .collect();
        let [property] = nodes.as_slice() else {
            return Err(refused(format!(
                "existing {xml} is missing or duplicated; restore a confirmed valid source"
            )));
        };
        if property.tag_name().namespace() != Some("http://v8.1c.ru/8.3/MDClasses")
            || property.attributes().len() != 0
        {
            return Err(refused(format!(
                "existing {xml} has a foreign namespace or attributes"
            )));
        }
        let raw = &source[property.range()];
        let replacement = if setting.localized() {
            let MetadataInputValue::Text(value) = value else {
                return Err(refused(format!("{xml} needs text")));
            };
            let Some(MetaObservedPropertyValue::LocalizedString { values }) =
                crate::info_projection::strict_localized_meta_info_property_value(*property)
            else {
                return Err(refused(format!(
                    "existing {xml} is not a valid localized string"
                )));
            };
            if lang.is_empty() {
                return Err(refused(
                    "a confirmed default text language is required".into(),
                ));
            }
            let current = values
                .iter()
                .find(|v| v.language == lang)
                .map(|v| v.content.as_str())
                .unwrap_or("");
            if current == value {
                continue;
            }
            let indent = crate::edit::meta_edit_line_indent(source, property.range().start);
            let changed = unica_xml_format::localized::replace_language_value(
                source, *property, &indent, lang, value,
            );
            let end = opening_end(&changed);
            let content = if changed[..end].ends_with("/>") {
                ""
            } else {
                &changed[end..changed.rfind("</").expect("localized closing element")]
            };
            wrap_content(raw, content)
        } else {
            if property.children().any(|n| n.is_element()) {
                return Err(refused(format!("existing {xml} is not a scalar")));
            }
            let current: String = property
                .children()
                .filter(|n| n.is_text())
                .filter_map(|n| n.text())
                .collect();
            let text = match (setting.boolean(), value) {
                (true, MetadataInputValue::Boolean(value))
                    if matches!(current.as_str(), "true" | "false") =>
                {
                    value.to_string()
                }
                (false, MetadataInputValue::Text(value)) => value.clone(),
                _ => {
                    return Err(refused(format!(
                        "{xml} has the wrong value type or a damaged source value"
                    )))
                }
            };
            if current == text {
                continue;
            }
            wrap_content(raw, &unica_xml_format::text::escape_xml(&text))
        };
        result.push((property.range(), replacement));
    }
    Ok(result)
}
