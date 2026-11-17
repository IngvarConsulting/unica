//! Проекция документирующего комментария метода конфигурации в ответ
//! `unica.view`.
//!
//! Поля читаются так же, как поля статьи справки платформы: значение
//! пустое, значение разобрано или `null` с причиной и сырым текстом раздела
//! в `unparsed` рядом. Набор ключей здесь один — разделы стандарта
//! разработки, — потому что вид у комментария один: это описание метода.
//!
//! Сырой текст комментария идёт рядом с полями: комментарий, написанный не
//! по стандарту, доходит до потребителя целиком, а не теряется ради полей.

use serde_json::{json, Map, Value};

use crate::article_projection::field;
use crate::model::*;

/// Версия схемы комментария. Поднимается, когда меняется набор полей или
/// смысл существующего поля.
pub const METHOD_DOC_SCHEMA_VERSION: u32 = 1;

fn parameter(value: &MethodDocParameter) -> Value {
    json!({
        "name": value.name,
        "types": value.types,
        "typeText": value.type_text,
        "description": value.description,
        "keys": Value::Array(value.keys.iter().map(parameter).collect()),
    })
}

/// Комментарий метода полями и `unparsed` рядом с ними.
pub fn method_doc(value: &MethodDoc) -> Value {
    let mut unparsed = Map::new();
    let description = field(
        &value.description,
        "description",
        json!(""),
        &mut unparsed,
        |text| json!(text),
    );
    let parameters = field(
        &value.parameters,
        "parameters",
        json!([]),
        &mut unparsed,
        |parameters| Value::Array(parameters.iter().map(parameter).collect()),
    );
    // Процедура ничего не возвращает, и это обычный случай: пустое
    // возвращаемое значение приходит той же формой с пустыми полями.
    // `null` остаётся за разделом, который есть и не разобран.
    let returns = field(
        &value.returns,
        "returns",
        json!({ "types": [], "typeText": "", "description": "", "keys": [] }),
        &mut unparsed,
        |returns| {
            json!({
                "types": returns.types,
                "typeText": returns.type_text,
                "description": returns.description,
                "keys": Value::Array(returns.keys.iter().map(parameter).collect()),
            })
        },
    );
    let example = field(
        &value.example,
        "example",
        json!(""),
        &mut unparsed,
        |code| json!(code),
    );
    let call_options = field(
        &value.call_options,
        "callOptions",
        json!([]),
        &mut unparsed,
        |items| json!(items),
    );
    let see_also = field(
        &value.see_also,
        "seeAlso",
        json!([]),
        &mut unparsed,
        |items| json!(items),
    );
    json!({
        "schemaVersion": METHOD_DOC_SCHEMA_VERSION,
        "text": value.text,
        "description": description,
        "parameters": parameters,
        "returns": returns,
        "example": example,
        "callOptions": call_options,
        "seeAlso": see_also,
        "otherSections": Value::Array(
            value
                .other_sections
                .iter()
                .map(|(title, text)| json!({ "title": title, "text": text }))
                .collect(),
        ),
        "unparsed": Value::Object(unparsed),
    })
}

#[cfg(test)]
#[path = "method_doc_projection_tests.rs"]
mod tests;
