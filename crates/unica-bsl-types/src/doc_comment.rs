//! Документирующий комментарий метода конфигурации, разобранный по
//! разделам стандарта разработки 1С.
//!
//! ```text
//! // Описание метода.
//! //
//! // Параметры:
//! //   Ссылка - СправочникСсылка.Валюты - валюта.
//! //          - Строка - её код.
//! //   Настройки - Структура:
//! //     * Период - Дата - ключ структуры.
//! //
//! // Возвращаемое значение:
//! //   Булево - Истина, если нашлось.
//! ```
//!
//! Запись раздела начинается на отступе первой записи этого раздела; строки
//! глубже продолжают описание, строка `- Тип - …` добавляет записи
//! альтернативный тип, строка `* Ключ - Тип - …` описывает ключ коллекции.
//!
//! Типы остаются текстом: что текст означает, решает прикладная модель, а
//! не этот разбор. Разбор ничего не выдумывает: нестандартный комментарий
//! целиком остаётся в [`DocComment::text`], а разделы, которых стандарт не
//! называет, — в [`DocComment::other_sections`] под своими заголовками.

use crate::doc_field::{Field, UnparsedReason};

/// Один параметр, как его описывает комментарий.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DocParameter {
    pub name: String,
    /// Типы текстами: «Строка», «СправочникСсылка.Валюты». Пусто, когда
    /// запись называет параметр, но не его тип.
    pub types: Vec<String>,
    /// Типы, как их пишет комментарий, одной строкой.
    pub type_text: String,
    pub description: String,
    /// Ключи коллекции (`* Ключ - Тип - …`) со своей вложенностью.
    pub keys: Vec<DocParameter>,
}

/// Возвращаемое значение, как его описывает комментарий.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DocReturn {
    pub types: Vec<String>,
    pub type_text: String,
    pub description: String,
    /// Ключи возвращаемой коллекции.
    pub keys: Vec<DocParameter>,
}

/// Комментарий метода полями. Сырой текст остаётся рядом с ними: разбор
/// добавляет поля, а не заменяет собой то, что написал человек.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DocComment {
    pub text: String,
    pub description: Field<String>,
    pub parameters: Field<Vec<DocParameter>>,
    pub returns: Field<DocReturn>,
    pub example: Field<String>,
    pub call_options: Field<Vec<String>>,
    pub see_also: Field<Vec<String>>,
    /// Разделы, которых стандарт не называет, под своими заголовками.
    pub other_sections: Vec<(String, String)>,
}

/// Раздел стандарта по его заголовку. Заголовок пишется и по-русски,
/// и по-английски: оба написания называют один раздел.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Section {
    Description,
    Parameters,
    Returns,
    Example,
    CallOptions,
    SeeAlso,
}

fn section_of(title: &str) -> Option<Section> {
    match title.trim().trim_end_matches(':').to_lowercase().as_str() {
        "параметры" | "parameters" => Some(Section::Parameters),
        "возвращаемое значение" | "returns" | "return value" => {
            Some(Section::Returns)
        }
        "пример" | "примеры" | "example" | "examples" => Some(Section::Example),
        "варианты вызова" | "call options" => Some(Section::CallOptions),
        "см. также" | "see also" => Some(Section::SeeAlso),
        _ => None,
    }
}

/// Строка комментария без ведущего `//`, с сохранённым отступом: отступ
/// внутри комментария — часть устройства разделов стандарта.
fn body(raw: &str) -> &str {
    let trimmed = raw.trim_start();
    match trimmed.strip_prefix("//") {
        Some(rest) => rest.strip_prefix('/').unwrap_or(rest),
        None => raw,
    }
}

/// Заголовок раздела: строка вида «Параметры:» без описания после
/// двоеточия. `Ссылка - Строка - описание: продолжение` заголовком не
/// является.
///
/// Раздел, названный стандартом, остаётся заголовком на любом отступе.
/// Прочий заголовок распознаётся только на краю комментария: иначе строка
/// `Структура:`, открывающая ключи записи, оборвала бы свой раздел.
fn heading(line: &str, margin: usize) -> Option<&str> {
    let indent = line.len() - line.trim_start().len();
    let line = line.trim();
    if !line.ends_with(':') || line.contains(" - ") {
        return None;
    }
    let title = line.trim_end_matches(':');
    (section_of(title).is_some() || indent <= margin).then_some(title)
}

/// Край комментария: наименьший отступ его непустых строк. Заголовки
/// разделов стоят на нём, записи — глубже.
fn margin_of(lines: &[String]) -> usize {
    lines
        .iter()
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.len() - line.trim_start().len())
        .min()
        .unwrap_or(0)
}

/// Текст раздела собранными строками.
fn joined(lines: &[String]) -> String {
    let mut text = lines.join("\n");
    while text.ends_with('\n') {
        text.pop();
    }
    text.trim_start_matches('\n').to_string()
}

/// Запись раздела: имя до первого « - », остальное — типы и описание.
/// Имя записи — идентификатор: строка с пробелами продолжает описание
/// предыдущей записи, а не открывает новую.
fn entry(line: &str) -> Option<(String, &str)> {
    let (name, rest) = match line.split_once(" - ") {
        Some((name, rest)) => (name.trim(), rest),
        None => (line.trim(), ""),
    };
    is_identifier(name).then(|| (name.to_string(), rest))
}

/// Типы и описание записи: типы — первая часть до « - », если она читается
/// типами; иначе типов нет, а вся часть — описание.
fn types_and_description(rest: &str) -> (Vec<String>, String, String) {
    let (head, tail) = match rest.split_once(" - ") {
        Some((head, tail)) => (head, tail.trim()),
        None => (rest, ""),
    };
    match type_list(head) {
        Some(types) => (types, head.trim().trim_end_matches(':').to_string(), {
            tail.to_string()
        }),
        None => (Vec::new(), String::new(), rest.trim().to_string()),
    }
}

/// Типы, перечисленные через запятую, или `None`, когда текст читается
/// описанием, а не типами. `Структура:` открывает ключи и остаётся типом.
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

/// `X` из «Массив из X» / «Array of X».
pub fn strip_array(text: &str) -> Option<&str> {
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

/// Глубина ключа по звёздочкам: `*` — ключ записи, `**` — ключ ключа.
fn key_depth(line: &str) -> Option<(usize, &str)> {
    let line = line.trim_start();
    let rest = line.trim_start_matches('*');
    let depth = line.len() - rest.len();
    (depth > 0).then(|| (depth, rest.trim()))
}

/// Записи раздела параметров: имя, типы, описание и ключи коллекций.
fn parameters(text: &str) -> Option<Vec<DocParameter>> {
    let mut found: Vec<DocParameter> = Vec::new();
    // Отступ первой записи задаёт край раздела: строки глубже продолжают
    // описание, строка на крае открывает следующую запись.
    let mut base = None::<usize>;
    // Куда ложится следующая строка продолжения: путь по глубине ключей.
    let mut path: Vec<usize> = Vec::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        let indent = raw.len() - raw.trim_start().len();
        if let Some((depth, rest)) = key_depth(line) {
            let Some((name, tail)) = entry(rest) else {
                continue;
            };
            let (types, type_text, description) = types_and_description(tail);
            let key = DocParameter {
                name,
                types,
                type_text,
                description,
                keys: Vec::new(),
            };
            // Ключ глубины 1 принадлежит записи, глубины 2 — её ключу.
            path.truncate(depth.saturating_sub(1));
            let Some(owner) = owner_of(&mut found, &path) else {
                continue;
            };
            owner.keys.push(key);
            path.push(owner.keys.len() - 1);
            continue;
        }
        if let Some(rest) = line.strip_prefix('-') {
            // Альтернативный тип предыдущей записи: `- Строка - описание`.
            // Описание альтернативы говорит, когда она применима, и
            // остаётся при записи: без него поле теряет половину смысла.
            let (types, type_text, description) = types_and_description(rest.trim());
            if !types.is_empty() {
                if let Some(target) = owner_of(&mut found, &path) {
                    append_types(target, types, &type_text);
                    continue_description(target, &description);
                }
            }
            continue;
        }
        let base = *base.get_or_insert(indent);
        if indent > base {
            if let Some(target) = owner_of(&mut found, &path) {
                continue_description(target, line);
            }
            continue;
        }
        let Some((name, rest)) = entry(line) else {
            // Строка на крае, не открывающая запись, продолжает описание
            // предыдущей: так пишут длинное описание без отступа.
            if let Some(target) = owner_of(&mut found, &path) {
                continue_description(target, line);
            }
            continue;
        };
        let (types, type_text, description) = types_and_description(rest);
        found.push(DocParameter {
            name,
            types,
            type_text,
            description,
            keys: Vec::new(),
        });
        path.clear();
    }
    (!found.is_empty()).then_some(found)
}

/// Запись, к которой относится следующая строка: последняя запись раздела,
/// спущенная по пути ключей.
fn owner_of<'a>(found: &'a mut [DocParameter], path: &[usize]) -> Option<&'a mut DocParameter> {
    let mut current = found.last_mut()?;
    for step in path {
        current = current.keys.get_mut(*step)?;
    }
    Some(current)
}

fn append_types(target: &mut DocParameter, types: Vec<String>, type_text: &str) {
    if !target.type_text.is_empty() {
        target.type_text.push_str("; ");
    }
    target.type_text.push_str(type_text);
    target.types.extend(types);
}

fn continue_description(target: &mut DocParameter, line: &str) {
    if line.is_empty() {
        return;
    }
    if !target.description.is_empty() {
        target.description.push(' ');
    }
    target.description.push_str(line);
}

/// Раздел возвращаемого значения записан без имени: `Тип - описание`.
/// Несколько строк называют несколько возможных типов одного значения.
fn returns(text: &str) -> Option<DocReturn> {
    let mut result = DocReturn::default();
    let mut base = None::<usize>;
    let mut path: Vec<usize> = Vec::new();
    let mut described = false;
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        let indent = raw.len() - raw.trim_start().len();
        if let Some((depth, rest)) = key_depth(line) {
            let Some((name, tail)) = entry(rest) else {
                continue;
            };
            let (types, type_text, description) = types_and_description(tail);
            let key = DocParameter {
                name,
                types,
                type_text,
                description,
                keys: Vec::new(),
            };
            path.truncate(depth.saturating_sub(1));
            let owner = match key_owner(&mut result, &path) {
                Some(owner) => owner,
                None => continue,
            };
            owner.push(key);
            path.push(owner.len() - 1);
            continue;
        }
        let base = *base.get_or_insert(indent);
        let alternative = line.starts_with('-');
        let head = line.strip_prefix('-').unwrap_or(line).trim();
        // Строка на крае раздела, не открывающая альтернативу, говорит
        // о самом значении: следующий его тип. Открытый ключ на ней
        // закрывается, иначе тип значения достался бы ключу.
        if !alternative && indent <= base {
            path.clear();
        }
        // Альтернативный тип и продолжение описания после ключа коллекции
        // принадлежат этому ключу, а не самому значению: иначе тип ключа
        // уезжал бы в типы возврата, а его оговорка — в его описание.
        if let Some(key) = active_key(&mut result, &path) {
            let (types, type_text, description) = types_and_description(head);
            if types.is_empty() {
                continue_description(key, head);
            } else {
                append_types(key, types, &type_text);
                continue_description(key, &description);
            }
            continue;
        }
        if indent <= base || line.starts_with('-') {
            let (types, type_text, description) = types_and_description(head);
            if !types.is_empty() {
                if !result.type_text.is_empty() {
                    result.type_text.push_str("; ");
                }
                result.type_text.push_str(&type_text);
                result.types.extend(types);
                if !description.is_empty() {
                    push_sentence(&mut result.description, &description);
                    described = true;
                }
                continue;
            }
        }
        // Строка без типа продолжает описание значения.
        push_sentence(&mut result.description, head);
        described = true;
    }
    (!result.types.is_empty() || described).then_some(result)
}

/// Ключ, открытый последним: к нему относятся его альтернативные типы и
/// продолжения описания. Без пути открытого ключа нет — строка относится
/// к самому значению.
fn active_key<'a>(result: &'a mut DocReturn, path: &[usize]) -> Option<&'a mut DocParameter> {
    let (last, parents) = path.split_last()?;
    let mut keys = &mut result.keys;
    for step in parents {
        keys = &mut keys.get_mut(*step)?.keys;
    }
    keys.get_mut(*last)
}

/// Ключи той записи, к которой они относятся: без пути — ключи самого
/// значения.
fn key_owner<'a>(result: &'a mut DocReturn, path: &[usize]) -> Option<&'a mut Vec<DocParameter>> {
    let mut keys = &mut result.keys;
    for step in path {
        keys = &mut keys.get_mut(*step)?.keys;
    }
    Some(keys)
}

fn push_sentence(target: &mut String, text: &str) {
    if text.is_empty() {
        return;
    }
    if !target.is_empty() {
        target.push(' ');
    }
    target.push_str(text);
}

/// Пункты перечисления раздела: каждая непустая строка — свой пункт.
fn items(text: &str) -> Option<Vec<String>> {
    let found = text
        .lines()
        .map(|line| line.trim().trim_start_matches('-').trim())
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    (!found.is_empty()).then_some(found)
}

/// Комментарий метода по разделам стандарта. `text` — комментарий как есть,
/// без `//`: он остаётся полным, даже когда разделов в нём нет.
pub fn parse(comment: &str) -> DocComment {
    let mut result = DocComment::default();
    let mut lines = Vec::<String>::new();
    for raw in comment.lines() {
        lines.push(body(raw).to_string());
    }
    result.text = joined(&lines);

    // Разделы по их заголовкам: всё до первого заголовка — описание.
    let mut current: Option<(Section, String)> = Some((Section::Description, String::new()));
    let mut sections = Vec::<(Section, String, Vec<String>)>::new();
    let mut other = Vec::<(String, Vec<String>)>::new();
    let mut collected = Vec::<String>::new();
    let mut other_title: Option<String> = None;
    let margin = margin_of(&lines);
    for line in &lines {
        if let Some(title) = heading(line, margin) {
            match (current.take(), other_title.take()) {
                (Some((section, name)), _) => {
                    sections.push((section, name, std::mem::take(&mut collected)))
                }
                (None, Some(name)) => other.push((name, std::mem::take(&mut collected))),
                (None, None) => {}
            }
            match section_of(title) {
                Some(section) => current = Some((section, title.to_string())),
                None => other_title = Some(title.to_string()),
            }
            continue;
        }
        collected.push(line.clone());
    }
    match (current, other_title) {
        (Some((section, name)), _) => sections.push((section, name, collected)),
        (None, Some(name)) => other.push((name, collected)),
        (None, None) => {}
    }

    for (section, _, body) in sections {
        let text = joined(&body);
        match section {
            Section::Description => {
                result.description = Field::from(
                    (!text.trim().is_empty()).then(|| text.trim().to_string()),
                    UnparsedReason::EmptySection,
                    "",
                )
            }
            Section::Parameters => {
                result.parameters =
                    Field::from(parameters(&text), UnparsedReason::NoParameterEntry, &text)
            }
            Section::Returns => {
                result.returns = Field::from(returns(&text), UnparsedReason::NoReturnType, &text)
            }
            Section::Example => {
                result.example = Field::from(
                    (!text.trim().is_empty()).then(|| trim_blank_lines(&text)),
                    UnparsedReason::EmptySection,
                    &text,
                )
            }
            Section::CallOptions => {
                result.call_options = Field::from(items(&text), UnparsedReason::EmptySection, &text)
            }
            Section::SeeAlso => {
                result.see_also = Field::from(items(&text), UnparsedReason::EmptySection, &text)
            }
        }
    }
    result.other_sections = other
        .into_iter()
        .map(|(title, body)| (title, joined(&body)))
        .collect();
    result
}

/// Пример сохраняет свои отступы: это код. Снимаются только пустые строки
/// по краям.
fn trim_blank_lines(text: &str) -> String {
    let lines = text.lines().collect::<Vec<_>>();
    let first = lines.iter().position(|line| !line.trim().is_empty());
    let last = lines.iter().rposition(|line| !line.trim().is_empty());
    match (first, last) {
        (Some(first), Some(last)) => lines[first..=last].join("\n"),
        _ => String::new(),
    }
}

#[cfg(test)]
#[path = "doc_comment_tests.rs"]
mod tests;
