//! `props.set` на форме: свойства корня `Form.xml` (#1984).
//!
//! Каждое свойство — одна строка корня (у `Title` — несколько) на своём
//! месте по схеме 8.3.27. Значение по умолчанию платформа не выгружает,
//! поэтому оно снимает строку, а не записывается. Остальные байты формы
//! остаются прежними.

use roxmltree::{Document, Node};
use serde_json::{Map, Value};
use unica_forms::root_properties::{
    root_property, RootProperty, RootValue, INTERLEAVED_ROOT_ELEMENTS, ROOT_PROPERTIES,
};

const TITLE: &str = "Title";

/// Ставит `values` в корень формы `xml_text`. Строковый `Title` пишется на
/// языке `lang`; без языка отказ — [`crate::codec::FORM_TEXT_LANGUAGE_REQUIRED`],
/// и вызывающий повторяет правку с языком набора.
pub fn set_root_properties(
    xml_text: &mut String,
    values: &Map<String, Value>,
    lang: Option<&str>,
) -> Result<(), String> {
    let main_attribute = main_attribute_type(xml_text)?;
    let mut planned = Vec::with_capacity(values.len());
    for (name, value) in values {
        let lines = if name == TITLE {
            title_lines(value, lang)?
        } else {
            let (_, property) = root_property(name)
                .filter(|(_, property)| property.value != RootValue::Reference)
                .ok_or_else(|| format!("unknown form property `{name}`"))?;
            if !property.scope.admits(main_attribute.as_deref()) {
                return Err(format!(
                    "form property `{name}` belongs to {}; this form's main attribute is {}",
                    property.scope.describe(),
                    main_attribute.as_deref().unwrap_or("absent")
                ));
            }
            let text = property_text(property, value)?;
            if property.default == Some(text.as_str()) {
                Vec::new()
            } else {
                vec![format!("<{name}>{text}</{name}>")]
            }
        };
        planned.push((name.as_str(), lines));
    }
    for (name, lines) in planned {
        replace_root_lines(xml_text, name, &lines)?;
    }
    Ok(())
}

fn title_lines(value: &Value, lang: Option<&str>) -> Result<Vec<String>, String> {
    let mut lines = Vec::new();
    match value {
        Value::String(text) if text.is_empty() => {}
        Value::String(text) => {
            crate::codec::emit_form_mltext(&mut lines, "", TITLE, text, lang)?;
        }
        _ => crate::codec::emit_form_mltext_value(&mut lines, "", TITLE, value, lang)?,
    }
    Ok(lines)
}

fn property_text(property: &RootProperty, value: &Value) -> Result<String, String> {
    let name = property.name;
    match property.value {
        RootValue::Flag => value
            .as_bool()
            .map(|flag| flag.to_string())
            .ok_or_else(|| format!("form property `{name}` takes true or false")),
        RootValue::Size => value
            .as_u64()
            .filter(|number| *number <= u64::from(u32::MAX))
            .map(|number| number.to_string())
            .ok_or_else(|| format!("form property `{name}` takes an integer 0..4294967295")),
        RootValue::Choice(choices) => value
            .as_str()
            .filter(|text| choices.contains(text))
            .map(str::to_string)
            .ok_or_else(|| format!("form property `{name}` takes one of {}", choices.join(", "))),
        RootValue::Reference => Err(format!("unknown form property `{name}`")),
    }
}

/// Тип основного реквизита формы без префикса `cfg:`.
fn main_attribute_type(xml_text: &str) -> Result<Option<String>, String> {
    let document = parse(xml_text)?;
    let main = children(document.root_element(), "Attributes")
        .flat_map(|attributes| children(attributes, "Attribute"))
        .find(|attribute| {
            children(*attribute, "MainAttribute")
                .any(|flag| flag.text().map(str::trim) == Some("true"))
        })
        .and_then(|attribute| children(attribute, "Type").next())
        .and_then(|types| {
            types
                .children()
                .find(|node| node.tag_name().name() == "Type")
        })
        .and_then(|node| node.text())
        .map(|text| text.trim().trim_start_matches("cfg:").to_string());
    Ok(main)
}

/// Место корневого элемента среди свойств: `Title`, таблица свойств с
/// вставленными в неё `MobileDeviceCommandBarContent` и `CommandSet`, затем
/// всё остальное (`AutoCommandBar` и содержимое формы). `None` — элемент,
/// которого порядок не знает: вставка его не задевает.
fn rank(name: &str) -> Option<usize> {
    if name == TITLE {
        return Some(0);
    }
    let mut position = 1;
    for property in ROOT_PROPERTIES {
        for (element, before) in INTERLEAVED_ROOT_ELEMENTS {
            if *before == property.name {
                if *element == name {
                    return Some(position);
                }
                position += 1;
            }
        }
        if property.name == name {
            return Some(position);
        }
        position += 1;
    }
    matches!(
        name,
        "AutoCommandBar"
            | "Events"
            | "ChildItems"
            | "Attributes"
            | "Parameters"
            | "Commands"
            | "CommandInterface"
            | "BaseForm"
    )
    .then_some(usize::MAX)
}

/// Снимает строки элемента `name` корня и, если `lines` не пусто, ставит
/// новые на место по порядку схемы, с отступом и концом строки соседей.
fn replace_root_lines(xml_text: &mut String, name: &str, lines: &[String]) -> Result<(), String> {
    let existing = {
        let document = parse(xml_text)?;
        let span = children(document.root_element(), name)
            .next()
            .map(|node| line_span(xml_text, node.range()));
        span
    };
    if let Some(span) = existing {
        xml_text.replace_range(span, "");
    }
    if lines.is_empty() {
        return Ok(());
    }
    let own = rank(name).ok_or_else(|| format!("unknown form property `{name}`"))?;
    let (at, indent) = {
        let document = parse(xml_text)?;
        let root = document.root_element();
        let next = root
            .children()
            .filter(Node::is_element)
            .find(|node| rank(node.tag_name().name()).is_some_and(|other| other > own))
            .ok_or_else(|| "the form root has no AutoCommandBar".to_string())?;
        let span = line_span(xml_text, next.range());
        let indent = xml_text[span.start..next.range().start].to_string();
        (span.start, indent)
    };
    let newline = if xml_text.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let block = lines
        .iter()
        .map(|line| format!("{indent}{line}{newline}"))
        .collect::<String>();
    xml_text.insert_str(at, &block);
    Ok(())
}

/// Строки, которые занимает элемент: от начала его строки до конца
/// последней, с переводом строки.
fn line_span(text: &str, range: std::ops::Range<usize>) -> std::ops::Range<usize> {
    let start = text[..range.start].rfind('\n').map_or(0, |index| index + 1);
    let end = text[range.end..]
        .find('\n')
        .map_or(text.len(), |index| range.end + index + 1);
    start..end
}

fn parse(xml_text: &str) -> Result<Document<'_>, String> {
    Document::parse(xml_text).map_err(|error| format!("[ERROR] XML parse error: {error}"))
}

fn children<'a, 'input: 'a, 'n: 'a>(
    node: Node<'a, 'input>,
    name: &'n str,
) -> impl Iterator<Item = Node<'a, 'input>> + 'a {
    node.children()
        .filter(move |child| child.is_element() && child.tag_name().name() == name)
}
