//! Types a documenting comment names, in the layout of the 1C development
//! standards:
//!
//! ```text
//! Параметры:
//!   Ссылка - СправочникСсылка.Валюты - the currency.
//!          - Строка - its code.
//!   Настройки - Структура:
//!     * Период - Дата - a key of the structure.
//! Возвращаемое значение:
//!   Неопределено - nothing was found;
//!                  the description goes on, indented deeper.
//! ```
//!
//! An entry starts at the indentation of the first entry of its section;
//! deeper lines continue a description or list structure keys. Only the
//! type text is taken; what a type means is decided elsewhere.

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
    // The indentation of the entries of the current section.
    let mut base = None::<usize>;
    // The last entry may still take alternative types (`- Тип - ...`); a
    // structure key in between takes them for itself.
    let mut open = false;
    for raw in doc.lines() {
        // The indentation counts after `//` when the comment keeps it.
        let body = raw.trim_start().strip_prefix("//").unwrap_or(raw);
        let line = body.trim();
        let indent = body.len() - body.trim_start().len();
        if line.is_empty() {
            section = Section::None;
            continue;
        }
        if line.ends_with(':') && !line.contains(" - ") {
            match line.trim_end_matches(':').to_lowercase().as_str() {
                "параметры" | "parameters" => {
                    section = Section::Params;
                    base = None;
                    open = false;
                    continue;
                }
                "возвращаемое значение" | "returns" | "return value" => {
                    section = Section::Returns;
                    base = None;
                    open = false;
                    continue;
                }
                // Other sections of the standard end the typed ones.
                "пример"
                | "примеры"
                | "example"
                | "examples"
                | "см. также"
                | "see also"
                | "варианты вызова"
                | "call options" => {
                    section = Section::None;
                    continue;
                }
                _ => {}
            }
        }
        if section == Section::None {
            continue;
        }
        // A line at the margin that is no entry header ends the section
        // (`Пример:` right after the result).
        if base.is_some_and(|base| indent < base) {
            section = Section::None;
            continue;
        }
        if line.starts_with('*') {
            open = false;
            continue;
        }
        if let Some(rest) = line.strip_prefix('-') {
            if open {
                if let Some(texts) = type_list(first_part(rest)) {
                    match section {
                        Section::Params => {
                            if let Some(last) = types.params.last_mut() {
                                last.1.extend(texts);
                            }
                        }
                        _ => types.returns.extend(texts),
                    }
                }
            }
            continue;
        }
        let base = *base.get_or_insert(indent);
        if indent > base {
            // A description going on.
            continue;
        }
        open = false;
        match section {
            Section::None => {}
            Section::Params => {
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
                    open = true;
                }
            }
            Section::Returns => {
                let text = match line.split_once(" - ") {
                    Some((text, _)) => text,
                    None => line,
                };
                if let Some(texts) = type_list(text) {
                    types.returns.extend(texts);
                    open = true;
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
/// rather than types. `Структура:` opens structure keys and is a type.
fn type_list(text: &str) -> Option<Vec<String>> {
    let text = text.trim().trim_end_matches(':');
    let texts = text
        .split(',')
        .map(|item| item.trim().trim_end_matches(['.', ';']).trim().to_string())
        .collect::<Vec<_>>();
    texts.iter().all(|item| is_type_text(item)).then_some(texts)
}

fn is_type_text(text: &str) -> bool {
    let mut leaf = text;
    while let Some(element) = strip_array(leaf) {
        leaf = element;
    }
    !leaf.is_empty() && leaf.split('.').all(is_identifier)
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
