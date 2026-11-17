use super::*;
use serde_json::json;

/// Комментарий приходит сюда, как его отдаёт модуль: `//` и один пробел за
/// ним уже сняты, отступы разделов сохранены.
fn standard() -> String {
    [
        "Создаёт копию таблицы.",
        "",
        "Параметры:",
        "  Строки - Массив из Строка - строки для копирования.",
        "",
        "Возвращаемое значение:",
        "  ТаблицаЗначений - копия.",
    ]
    .join("\n")
}

/// Комментарий стандарта доходит до ответа полями: описание, параметры
/// с типами и описаниями, возвращаемое значение и версия схемы.
/// rule: INV.APP.DOCUMENTATION-METHOD-COMMENT
#[test]
fn a_standard_comment_reaches_the_answer_by_its_fields() {
    let value = doc_fields(&standard());

    assert_eq!(value["description"], json!("Создаёт копию таблицы."));
    assert_eq!(value["parameters"][0]["name"], json!("Строки"));
    assert_eq!(value["parameters"][0]["types"], json!(["Массив из Строка"]));
    assert_eq!(
        value["parameters"][0]["description"],
        json!("строки для копирования.")
    );
    assert_eq!(value["returns"]["types"], json!(["ТаблицаЗначений"]));
    assert_eq!(value["returns"]["description"], json!("копия."));
    assert_eq!(value["schemaVersion"], json!(1));
}

/// Нестандартный комментарий остаётся текстом без потерь: полей из него
/// не вышло, но написанное человеком доходит целиком.
/// rule: INV.APP.DOCUMENTATION-METHOD-COMMENT
#[test]
fn a_comment_written_outside_the_standard_keeps_its_whole_text() {
    let value = doc_fields("Проверяет заказ.\nВызывать только из обработчика.");

    assert_eq!(
        value["text"],
        json!("Проверяет заказ.\nВызывать только из обработчика.")
    );
    assert_eq!(value["parameters"], json!([]));
    assert_eq!(value["unparsed"], json!({}));
}

/// Раздел, который в комментарии есть и не разобран, приходит `null`
/// с причиной и сырым текстом рядом, а не пустым значением.
/// rule: INV.APP.DOCUMENTATION-ARTICLE-STATES
#[test]
fn a_parameter_section_without_entries_arrives_null_with_its_reason() {
    let comment = [
        "Печатает заказ.",
        "",
        "Параметры:",
        "  описаны в стандарте предприятия.",
    ]
    .join("\n");
    let value = doc_fields(&comment);

    assert_eq!(value["parameters"], Value::Null);
    assert_eq!(
        value["unparsed"]["parameters"]["reason"],
        json!("no-parameter-entry")
    );
    assert_eq!(
        value["unparsed"]["parameters"]["text"],
        json!("  описаны в стандарте предприятия.")
    );
}
