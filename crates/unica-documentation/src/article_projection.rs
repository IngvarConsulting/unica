//! Проекция статьи справки в ответ `unica.docs`.
//!
//! Потребитель формата различает четыре положения поля, а не два:
//!
//! 1. **Ключа нет** — у этого вида статьи такого понятия не бывает:
//!    у статьи-понятия нет параметров. Набор ключей задаёт вид статьи,
//!    и раздел просто не рисуется.
//! 2. **Значение пустое** (`[]`, `""`, `false`) — понятие есть, значений
//!    нет: у метода нет параметров, у свойства нет примера. Человеку
//!    пишется «Параметров нет», модель вызывает без аргументов.
//! 3. **Значение разобрано.**
//! 4. **`null`** — раздел в статье есть, но Unica его не разобрала.
//!    Причина и сырой текст раздела лежат в `unparsed` рядом, под именем
//!    того же поля. Путать это с пустым нельзя: иначе модель решит, что
//!    параметров нет, когда они просто не разобрались.
//!
//! Версия схемы формата идёт в ответе: по ней потребитель знает, какие
//! поля вправе ждать.

use serde_json::{json, Map, Value};

use crate::model::*;

/// Версия схемы статьи. Поднимается, когда меняется набор полей или смысл
/// существующего поля; добавление поля к виду статьи — тоже смена набора.
pub const ARTICLE_SCHEMA_VERSION: u32 = 1;

/// Поле статьи, которое вид статьи может иметь.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArticleFieldName {
    Variants,
    ValueTypes,
    ReadOnly,
    Availability,
    SinceVersion,
    Description,
    Note,
    Example,
    CollectionElement,
    MemberLists,
}

impl ArticleFieldName {
    fn as_str(self) -> &'static str {
        match self {
            ArticleFieldName::Variants => "variants",
            ArticleFieldName::ValueTypes => "valueTypes",
            ArticleFieldName::ReadOnly => "readOnly",
            ArticleFieldName::Availability => "availability",
            ArticleFieldName::SinceVersion => "sinceVersion",
            ArticleFieldName::Description => "description",
            ArticleFieldName::Note => "note",
            ArticleFieldName::Example => "example",
            ArticleFieldName::CollectionElement => "collectionElement",
            ArticleFieldName::MemberLists => "memberLists",
        }
    }
}

/// Какие поля бывают у этого вида статьи. Остальных ключей в ответе нет:
/// их отсутствие и означает «такого понятия у этого вида не бывает».
fn fields_of(kind: ArticleKind) -> &'static [ArticleFieldName] {
    use ArticleFieldName::*;
    match kind {
        ArticleKind::Method | ArticleKind::Event | ArticleKind::Constructor => &[
            Variants,
            Availability,
            SinceVersion,
            Description,
            Note,
            Example,
        ],
        ArticleKind::Property => &[
            ValueTypes,
            ReadOnly,
            Availability,
            SinceVersion,
            Description,
            Note,
            Example,
        ],
        ArticleKind::Type => &[
            CollectionElement,
            MemberLists,
            Availability,
            SinceVersion,
            Description,
            Note,
            Example,
        ],
        // У понятия нет ни членов, ни вызова: статья — текст с разделами.
        ArticleKind::Concept => &[SinceVersion, Description, Note, Example],
    }
}

/// Разобранное значение поля, пустое значение этого же вида или `null`
/// с причиной в `unparsed`.
fn field<T>(
    field: &ArticleField<T>,
    name: &str,
    empty: Value,
    unparsed: &mut Map<String, Value>,
    render: impl FnOnce(&T) -> Value,
) -> Value {
    match field {
        ArticleField::Empty => empty,
        ArticleField::Parsed(value) => render(value),
        ArticleField::Unparsed { reason, text } => {
            unparsed.insert(name.to_string(), json!({ "reason": reason, "text": text }));
            Value::Null
        }
    }
}

fn name(value: &LocalizedName) -> Value {
    json!({ "ru": value.ru, "en": value.en })
}

fn type_mentions(types: &[TypeMention]) -> Value {
    Value::Array(
        types
            .iter()
            .map(|mention| json!({ "text": mention.text, "documentId": mention.document_id }))
            .collect(),
    )
}

fn links(items: &[ArticleLink]) -> Value {
    Value::Array(
        items
            .iter()
            .map(|link| json!({ "label": link.label, "documentId": link.document_id }))
            .collect(),
    )
}

/// Вариант синтаксиса. Неразобранный раздел варианта называется в его
/// собственном `unparsed`: иначе два варианта одного метода были бы
/// неразличимы.
fn variant(value: &ArticleVariant) -> Value {
    let mut unparsed = Map::new();
    let call = field(&value.call, "call", json!(""), &mut unparsed, |call| {
        json!(call)
    });
    let parameters = field(
        &value.parameters,
        "parameters",
        json!([]),
        &mut unparsed,
        |parameters| {
            Value::Array(
                parameters
                    .iter()
                    .map(|parameter| {
                        json!({
                            "name": parameter.name,
                            "optional": parameter.optional,
                            "types": type_mentions(&parameter.types),
                            "typeText": parameter.type_text,
                            "default": parameter.default,
                            "description": parameter.description,
                        })
                    })
                    .collect(),
            )
        },
    );
    let description = field(
        &value.description,
        "description",
        json!(""),
        &mut unparsed,
        |text| json!(text),
    );
    let returns = field(
        &value.returns,
        "returns",
        Value::Null,
        &mut unparsed,
        |returns| {
            json!({
                "types": type_mentions(&returns.types),
                "typeText": returns.type_text,
                "description": returns.description,
            })
        },
    );
    json!({
        "name": value.name,
        "call": call,
        "parameters": parameters,
        "description": description,
        "returns": returns,
        "unparsed": Value::Object(unparsed),
    })
}

/// Статья полями и `unparsed` рядом с ними.
pub fn article(value: &HelpArticle) -> Value {
    let mut out = Map::new();
    let mut unparsed = Map::new();
    out.insert("kind".to_string(), json!(value.kind.as_str()));
    out.insert("name".to_string(), name(&value.name));
    if let Some(owner) = &value.owner {
        out.insert("owner".to_string(), name(owner));
        out.insert(
            "ownerDocumentId".to_string(),
            json!(value.owner_document_id),
        );
    }
    for present in fields_of(value.kind) {
        let rendered = match present {
            ArticleFieldName::Variants => field(
                &value.variants,
                present.as_str(),
                json!([]),
                &mut unparsed,
                |variants| Value::Array(variants.iter().map(variant).collect()),
            ),
            ArticleFieldName::ValueTypes => {
                let types = field(
                    &value.value_types,
                    present.as_str(),
                    json!([]),
                    &mut unparsed,
                    |types| type_mentions(types),
                );
                out.insert("valueTypeText".to_string(), json!(value.value_type_text));
                types
            }
            ArticleFieldName::ReadOnly => field(
                &value.read_only,
                present.as_str(),
                json!(false),
                &mut unparsed,
                |flag| json!(flag),
            ),
            ArticleFieldName::Availability => {
                let places = field(
                    &value.availability,
                    present.as_str(),
                    json!([]),
                    &mut unparsed,
                    |places| json!(places),
                );
                out.insert(
                    "availabilityText".to_string(),
                    json!(value.availability_text),
                );
                places
            }
            ArticleFieldName::SinceVersion => field(
                &value.since_version,
                present.as_str(),
                json!(""),
                &mut unparsed,
                |version| json!(version),
            ),
            ArticleFieldName::Description => field(
                &value.description,
                present.as_str(),
                json!(""),
                &mut unparsed,
                |text| json!(text),
            ),
            ArticleFieldName::Note => field(
                &value.note,
                present.as_str(),
                json!(""),
                &mut unparsed,
                |text| json!(text),
            ),
            ArticleFieldName::Example => field(
                &value.example,
                present.as_str(),
                json!(""),
                &mut unparsed,
                |code| json!(code),
            ),
            ArticleFieldName::CollectionElement => field(
                &value.collection_element,
                present.as_str(),
                json!([]),
                &mut unparsed,
                |types| type_mentions(types),
            ),
            ArticleFieldName::MemberLists => Value::Array(
                value
                    .member_lists
                    .iter()
                    .map(|list| json!({ "title": list.title, "members": links(&list.members) }))
                    .collect(),
            ),
        };
        out.insert(present.as_str().to_string(), rendered);
    }
    out.insert("links".to_string(), links(&value.links));
    out.insert(
        "otherSections".to_string(),
        Value::Array(
            value
                .other_sections
                .iter()
                .map(|(title, text)| json!({ "title": title, "text": text }))
                .collect(),
        ),
    );
    out.insert("unparsed".to_string(), Value::Object(unparsed));
    Value::Object(out)
}

#[cfg(test)]
#[path = "article_projection_tests.rs"]
mod tests;
