//! Types a documenting comment names, in the layout of the 1C development
//! standards:
//!
//! ```text
//! Параметры:
//!   Ссылка - СправочникСсылка.Валюты - the currency.
//!          - Строка - its code.
//! Возвращаемое значение:
//!   Структура:
//!     * Код - Строка - a key of the structure.
//!   Неопределено - nothing was found.
//! ```
//!
//! Only the type text is taken; what a type means is decided elsewhere.

/// The type texts of the parameters, by parameter name, and of the result.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct DocTypes {
    pub(crate) params: Vec<(String, Vec<String>)>,
    pub(crate) returns: Vec<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    None,
    Params,
    Returns,
}

pub(crate) fn doc_types(doc: &str) -> DocTypes {
    let mut types = DocTypes::default();
    let mut section = Section::None;
    for raw in doc.lines() {
        let line = raw.trim_start_matches('/').trim();
        let header = line.trim_end_matches(':').to_lowercase();
        if line.ends_with(':') && !line.contains(" - ") {
            match header.as_str() {
                "параметры" | "parameters" => {
                    section = Section::Params;
                    continue;
                }
                "возвращаемое значение" | "returns" | "return value" => {
                    section = Section::Returns;
                    continue;
                }
                _ => {}
            }
        }
        if line.is_empty() {
            section = Section::None;
            continue;
        }
        // Keys of a structure belong to its description, not to the method.
        if line.starts_with('*') {
            continue;
        }
        match section {
            Section::None => {}
            Section::Params => {
                if let Some(rest) = line.strip_prefix('-') {
                    if let (Some(texts), Some(last)) =
                        (type_list(first_part(rest)), types.params.last_mut())
                    {
                        last.1.extend(texts);
                    }
                    continue;
                }
                let mut parts = line.splitn(3, " - ");
                let (Some(name), Some(text)) = (parts.next(), parts.next()) else {
                    continue;
                };
                let name = name.trim();
                if !is_identifier(name) {
                    continue;
                }
                if let Some(texts) = type_list(text) {
                    types.params.push((name.to_string(), texts));
                }
            }
            Section::Returns => {
                let rest = line.strip_prefix('-').unwrap_or(line);
                let text = match rest.split_once(" - ") {
                    Some((text, _)) => text,
                    // `Структура:` opens the keys that follow.
                    None => rest.trim().trim_end_matches(':'),
                };
                if let Some(texts) = type_list(text) {
                    types.returns.extend(texts);
                }
            }
        }
    }
    types
}

fn first_part(text: &str) -> &str {
    text.split_once(" - ").map_or(text, |(first, _)| first)
}

/// Comma-separated type texts, or `None` when the text is a description
/// rather than types.
fn type_list(text: &str) -> Option<Vec<String>> {
    let texts = text
        .split(',')
        .map(|item| item.trim().trim_end_matches('.').trim().to_string())
        .collect::<Vec<_>>();
    texts.iter().all(|item| is_type_text(item)).then_some(texts)
}

fn is_type_text(text: &str) -> bool {
    let text = strip_array(text).unwrap_or(text);
    !text.is_empty() && text.split('.').all(is_identifier)
}

/// `X` of `Массив из X` / `Array of X`.
pub(crate) fn strip_array(text: &str) -> Option<&str> {
    ["Массив из ", "массив из ", "Array of ", "array of "]
        .iter()
        .find_map(|prefix| text.strip_prefix(prefix))
        .map(str::trim)
}

fn is_identifier(text: &str) -> bool {
    let mut chars = text.chars();
    chars
        .next()
        .is_some_and(|first| first.is_alphabetic() || first == '_')
        && chars.all(|ch| ch.is_alphanumeric() || ch == '_')
}
