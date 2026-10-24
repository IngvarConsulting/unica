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
        let edit = if name == TITLE {
            title_edit(value, lang)?
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
            RootEdit::Lines(if property.default == Some(text.as_str()) {
                Vec::new()
            } else {
                vec![format!("<{name}>{text}</{name}>")]
            })
        };
        planned.push((name.as_str(), edit));
    }
    for (name, edit) in planned {
        match edit {
            RootEdit::Lines(lines) => replace_root_lines(xml_text, name, &lines)?,
            RootEdit::TitleText { lang, text } => {
                if !set_title_text(xml_text, &lang, &text)? {
                    let mut lines = Vec::new();
                    crate::codec::emit_form_mltext(&mut lines, "", TITLE, &text, Some(&lang))?;
                    replace_root_lines(xml_text, name, &lines)?;
                }
            }
        }
    }
    Ok(())
}

enum RootEdit {
    /// Строки элемента целиком; пусто — элемент снимается.
    Lines(Vec<String>),
    /// Текст заголовка на одном языке: переводы на другие языки остаются.
    TitleText { lang: String, text: String },
}

fn title_edit(value: &Value, lang: Option<&str>) -> Result<RootEdit, String> {
    match value {
        Value::String(text) if text.is_empty() => Ok(RootEdit::Lines(Vec::new())),
        Value::String(text) => {
            let lang = lang
                .filter(|lang| !lang.is_empty())
                .ok_or_else(|| crate::codec::FORM_TEXT_LANGUAGE_REQUIRED.to_string())?;
            Ok(RootEdit::TitleText {
                lang: lang.to_string(),
                text: text.clone(),
            })
        }
        _ => {
            if let Some((language, _)) = value
                .as_object()
                .into_iter()
                .flatten()
                .find(|(_, text)| text.as_str().is_none_or(str::is_empty))
            {
                return Err(format!(
                    "form Title for `{language}` takes a non-empty text; an empty string Title removes the title"
                ));
            }
            let mut lines = Vec::new();
            crate::codec::emit_form_mltext_value(&mut lines, "", TITLE, value, lang)?;
            Ok(RootEdit::Lines(lines))
        }
    }
}

/// Меняет текст заголовка на языке `lang` внутри существующего `Title`, не
/// трогая переводов на другие языки. `false` — заголовка нет.
fn set_title_text(xml_text: &mut String, lang: &str, text: &str) -> Result<bool, String> {
    let escaped = unica_xml_format::text::escape_xml(text);
    let edit = {
        let document = parse(xml_text)?;
        let Some(title) = children(document.root_element(), TITLE).next() else {
            return Ok(false);
        };
        let items = title
            .children()
            .filter(|node| node.is_element() && node.tag_name().name() == "item")
            .collect::<Vec<_>>();
        let same = items.iter().copied().find(|item| {
            item.children().any(|node| {
                node.is_element()
                    && node.tag_name().name() == "lang"
                    && node.text().map(str::trim) == Some(lang)
            })
        });
        let content = match same {
            Some(item) => Some(children(item, "content").next().ok_or_else(|| {
                format!(
                    "the form Title for `{lang}` has no content; re-export the form with the platform before editing it"
                )
            })?),
            None => None,
        };
        match (content, items.last()) {
            (Some(content), _) => {
                let range = content.range();
                let prefix = content.tag_name().namespace().map_or("", |_| "v8:");
                (
                    range,
                    format!("<{prefix}content>{escaped}</{prefix}content>"),
                )
            }
            (None, Some(last)) => {
                let span = line_span(xml_text, last.range())?;
                let indent = &xml_text[span.start..last.range().start];
                let newline = if xml_text.contains("\r\n") {
                    "\r\n"
                } else {
                    "\n"
                };
                let block = [
                    format!("{indent}<v8:item>"),
                    format!(
                        "{indent}\t<v8:lang>{}</v8:lang>",
                        unica_xml_format::text::escape_xml(lang)
                    ),
                    format!("{indent}\t<v8:content>{escaped}</v8:content>"),
                    format!("{indent}</v8:item>"),
                ]
                .iter()
                .map(|line| format!("{line}{newline}"))
                .collect::<String>();
                (span.end..span.end, block)
            }
            (None, None) => return Ok(false),
        }
    };
    xml_text.replace_range(edit.0, &edit.1);
    Ok(true)
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
            .map(|node| line_span(xml_text, node.range()))
            .transpose()?;
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
        let span = line_span(xml_text, next.range())?;
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
/// последней, с переводом строки. Элемент делит строку с соседом только
/// в правленном вручную файле; такой корень не правится.
fn line_span(text: &str, range: std::ops::Range<usize>) -> Result<std::ops::Range<usize>, String> {
    let start = text[..range.start].rfind('\n').map_or(0, |index| index + 1);
    let end = text[range.end..]
        .find('\n')
        .map_or(text.len(), |index| range.end + index + 1);
    if !text[start..range.start].trim().is_empty() || !text[range.end..end].trim().is_empty() {
        return Err(
            "the form root shares a line between elements; re-export the form with the platform before editing it"
                .to_string(),
        );
    }
    Ok(start..end)
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
