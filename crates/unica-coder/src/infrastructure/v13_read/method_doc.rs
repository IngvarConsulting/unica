//! Документирующий комментарий метода конфигурации полями.
//!
//! Разбор устройства комментария лежит в `unica-bsl-types` рядом с разбором
//! статьи справки платформы: грамматика стандарта разработки одна, и
//! прикладная модель типов (#2238) читает её тем же разбором. Контракт
//! формата принадлежит координатору документации. Здесь только сшивка:
//! разобранный комментарий становится публичной моделью, а она — полями
//! ответа.

use serde_json::Value;
use unica_bsl_types::doc_comment;
use unica_documentation::model as public;

fn field<T, U>(
    value: &unica_bsl_types::Field<T>,
    convert: impl FnOnce(&T) -> U,
) -> public::ArticleField<U> {
    match value {
        unica_bsl_types::Field::Empty => public::ArticleField::Empty,
        unica_bsl_types::Field::Parsed(value) => public::ArticleField::Parsed(convert(value)),
        unica_bsl_types::Field::Unparsed { reason, text } => public::ArticleField::Unparsed {
            reason: reason.as_str().to_string(),
            text: text.clone(),
        },
    }
}

fn parameter(value: &doc_comment::DocParameter) -> public::MethodDocParameter {
    public::MethodDocParameter {
        name: value.name.clone(),
        types: value.types.clone(),
        type_text: value.type_text.clone(),
        description: value.description.clone(),
        keys: value.keys.iter().map(parameter).collect(),
    }
}

fn parameters(list: &[doc_comment::DocParameter]) -> Vec<public::MethodDocParameter> {
    list.iter().map(parameter).collect()
}

/// Поля комментария метода. Сырой текст остаётся внутри: нестандартный
/// комментарий доходит до потребителя целиком.
pub(super) fn doc_fields(comment: &str) -> Value {
    let parsed = doc_comment::parse(comment);
    let doc = public::MethodDoc {
        text: parsed.text.clone(),
        description: field(&parsed.description, Clone::clone),
        parameters: field(&parsed.parameters, |list| parameters(list)),
        returns: field(&parsed.returns, |returns| public::MethodDocReturn {
            types: returns.types.clone(),
            type_text: returns.type_text.clone(),
            description: returns.description.clone(),
            keys: parameters(&returns.keys),
        }),
        example: field(&parsed.example, Clone::clone),
        call_options: field(&parsed.call_options, Clone::clone),
        see_also: field(&parsed.see_also, Clone::clone),
        other_sections: parsed.other_sections.clone(),
    };
    unica_documentation::method_doc_projection::method_doc(&doc)
}

#[cfg(test)]
#[path = "method_doc_tests.rs"]
mod tests;
