use super::*;

fn comment() -> MethodDoc {
    MethodDoc {
        text: " Создаёт копию.".to_string(),
        description: ArticleField::Parsed("Создаёт копию.".to_string()),
        parameters: ArticleField::Empty,
        returns: ArticleField::Empty,
        example: ArticleField::Empty,
        call_options: ArticleField::Empty,
        see_also: ArticleField::Empty,
        other_sections: Vec::new(),
    }
}

/// Сырой текст комментария и версия схемы идут рядом с полями: потребитель
/// знает, какие поля вправе ждать, и видит исходное, если полей нет.
/// rule: INV.APP.DOCUMENTATION-METHOD-COMMENT
#[test]
fn the_raw_comment_and_the_schema_version_arrive_next_to_the_fields() {
    let value = method_doc(&comment());

    assert_eq!(value["schemaVersion"], json!(METHOD_DOC_SCHEMA_VERSION));
    assert_eq!(value["text"], json!(" Создаёт копию."));
    assert_eq!(value["description"], json!("Создаёт копию."));
}

/// Раздела в комментарии нет — поле приходит пустым значением своего вида:
/// модель читает «параметров нет» и вызывает без аргументов.
/// rule: INV.APP.DOCUMENTATION-ARTICLE-STATES
#[test]
fn a_section_the_comment_does_not_have_arrives_empty_not_null() {
    let value = method_doc(&comment());

    assert_eq!(value["parameters"], json!([]));
    assert_eq!(value["example"], json!(""));
    assert_eq!(value["callOptions"], json!([]));
    assert_eq!(
        value["returns"],
        json!({ "types": [], "typeText": "", "description": "", "keys": [] })
    );
    assert_eq!(value["unparsed"], json!({}));
}

/// Раздел, который в комментарии есть и не разобран, приходит `null`,
/// а причина и сырой текст — рядом в `unparsed`. Иначе «параметров нет»
/// неотличимо от «параметры не разобрались».
/// rule: INV.APP.DOCUMENTATION-ARTICLE-STATES
#[test]
fn an_unparsed_section_arrives_null_with_its_reason_and_raw_text() {
    let mut source = comment();
    source.parameters = ArticleField::Unparsed {
        reason: "no-parameter-entry".to_string(),
        text: "   описаны в стандарте предприятия.".to_string(),
    };
    let value = method_doc(&source);

    assert_eq!(value["parameters"], Value::Null);
    assert_eq!(
        value["unparsed"]["parameters"],
        json!({ "reason": "no-parameter-entry", "text": "   описаны в стандарте предприятия." })
    );
}

/// Параметр несёт имя, типы текстами и ключи коллекции со своей
/// вложенностью: панель справки по коду показывает то, что написано.
/// rule: INV.APP.DOCUMENTATION-METHOD-COMMENT
#[test]
fn a_parameter_carries_its_types_and_the_keys_of_its_collection() {
    let mut source = comment();
    source.parameters = ArticleField::Parsed(vec![MethodDocParameter {
        name: "Настройки".to_string(),
        types: vec!["Структура".to_string()],
        type_text: "Структура".to_string(),
        description: "настройки отбора.".to_string(),
        keys: vec![MethodDocParameter {
            name: "Период".to_string(),
            types: vec!["Дата".to_string()],
            type_text: "Дата".to_string(),
            description: "начало.".to_string(),
            keys: Vec::new(),
        }],
    }]);
    source.see_also = ArticleField::Parsed(vec!["ОбщегоНазначения.Скопировать".to_string()]);
    let value = method_doc(&source);

    let parameter = &value["parameters"][0];
    assert_eq!(parameter["name"], json!("Настройки"));
    assert_eq!(parameter["types"], json!(["Структура"]));
    assert_eq!(parameter["keys"][0]["name"], json!("Период"));
    assert_eq!(parameter["keys"][0]["types"], json!(["Дата"]));
    assert_eq!(value["seeAlso"], json!(["ОбщегоНазначения.Скопировать"]));
}
