use super::*;

fn name(ru: &str, en: &str) -> LocalizedName {
    LocalizedName {
        ru: ru.to_string(),
        en: Some(en.to_string()),
    }
}

fn article_of(kind: ArticleKind) -> HelpArticle {
    HelpArticle {
        kind,
        name: name("Скопировать", "Copy"),
        owner: Some(name("ТаблицаЗначений", "ValueTable")),
        owner_document_id: Some("platform-syntax-help:syntax-context:ValueTable.html".to_string()),
        variants: ArticleField::Empty,
        value_types: ArticleField::Empty,
        value_type_text: String::new(),
        read_only: ArticleField::Empty,
        availability: ArticleField::Empty,
        availability_text: String::new(),
        since_version: ArticleField::Empty,
        description: ArticleField::Empty,
        note: ArticleField::Empty,
        example: ArticleField::Empty,
        collection_element: ArticleField::Empty,
        member_lists: Vec::new(),
        links: Vec::new(),
        other_sections: Vec::new(),
    }
}

/// Какого понятия у вида статьи не бывает, того нет и ключом: набор ключей
/// читается из вида, а не угадывается по пустым значениям.
/// rule: INV.APP.DOCUMENTATION-ARTICLE-FIELDS
#[test]
fn the_kind_of_the_article_decides_which_keys_exist() {
    let method = article(&article_of(ArticleKind::Method));
    assert!(method.get("variants").is_some());
    assert!(
        method.get("readOnly").is_none(),
        "у метода не бывает «только чтение»: {method}"
    );
    assert!(method.get("memberLists").is_none());

    let property = article(&article_of(ArticleKind::Property));
    assert!(property.get("readOnly").is_some());
    assert!(property.get("valueTypes").is_some());
    assert!(
        property.get("variants").is_none(),
        "у свойства не бывает вариантов синтаксиса: {property}"
    );

    let ty = article(&article_of(ArticleKind::Type));
    assert!(ty.get("memberLists").is_some() && ty.get("collectionElement").is_some());
    assert!(ty.get("variants").is_none() && ty.get("readOnly").is_none());

    let concept = article(&article_of(ArticleKind::Concept));
    assert!(concept.get("description").is_some());
    assert!(
        concept.get("variants").is_none() && concept.get("availability").is_none(),
        "у понятия нет ни вызова, ни мест исполнения: {concept}"
    );
}

/// Поле, которого в статье нет, приходит пустым значением своего вида, а не
/// `null`: модель читает «параметров нет» и вызывает без аргументов.
/// rule: INV.APP.DOCUMENTATION-ARTICLE-STATES
#[test]
fn a_concept_without_values_arrives_empty_not_null() {
    let mut source = article_of(ArticleKind::Method);
    source.variants = ArticleField::Parsed(vec![ArticleVariant {
        name: None,
        call: ArticleField::Parsed("Скопировать()".to_string()),
        parameters: ArticleField::Empty,
        description: ArticleField::Empty,
        returns: ArticleField::Empty,
    }]);
    let value = article(&source);

    assert_eq!(value["variants"][0]["parameters"], json!([]));
    assert_eq!(value["variants"][0]["description"], json!(""));
    assert_eq!(value["example"], json!(""));
    assert_eq!(value["availability"], json!([]));
    assert_eq!(value["unparsed"], json!({}));
    assert_eq!(value["variants"][0]["unparsed"], json!({}));
}

/// Раздел, который в статье есть и не разобран, приходит `null`, а причина
/// и сырой текст — рядом в `unparsed`. Путать это с пустым нельзя: иначе
/// модель решит, что параметров нет, когда они просто не разобрались.
/// rule: INV.APP.DOCUMENTATION-ARTICLE-STATES
#[test]
fn an_unparsed_section_arrives_null_with_its_reason_and_raw_text() {
    let mut source = article_of(ArticleKind::Method);
    source.example = ArticleField::Unparsed {
        reason: "no-code-block".to_string(),
        text: "Пример показан картинкой.".to_string(),
    };
    source.variants = ArticleField::Parsed(vec![ArticleVariant {
        name: Some("Первый".to_string()),
        call: ArticleField::Parsed("Сделать(<А>)".to_string()),
        parameters: ArticleField::Unparsed {
            reason: "no-parameter-rubric".to_string(),
            text: "Параметр описан прозой.".to_string(),
        },
        description: ArticleField::Empty,
        returns: ArticleField::Empty,
    }]);
    let value = article(&source);

    assert_eq!(value["example"], Value::Null);
    assert_eq!(
        value["unparsed"]["example"],
        json!({ "reason": "no-code-block", "text": "Пример показан картинкой." })
    );
    assert_eq!(value["variants"][0]["parameters"], Value::Null);
    assert_eq!(
        value["variants"][0]["unparsed"]["parameters"],
        json!({ "reason": "no-parameter-rubric", "text": "Параметр описан прозой." })
    );
    // Неразобранный раздел варианта остаётся при своём варианте, а не
    // уходит в общий список: иначе два варианта метода неразличимы.
    assert_eq!(value["unparsed"].as_object().expect("объект").len(), 1);
}

/// rule: INV.APP.DOCUMENTATION-ARTICLE-FIELDS
#[test]
fn a_method_article_carries_its_parameters_types_and_links() {
    let mut source = article_of(ArticleKind::Method);
    source.variants = ArticleField::Parsed(vec![ArticleVariant {
        name: Some("Скопировать строки".to_string()),
        call: ArticleField::Parsed("Скопировать(<Строки>)".to_string()),
        parameters: ArticleField::Parsed(vec![ArticleParameter {
            name: "Строки".to_string(),
            optional: true,
            types: vec![TypeMention {
                text: "Массив".to_string(),
                document_id: Some("platform-syntax-help:syntax-context:Array.html".to_string()),
            }],
            type_text: "Массив".to_string(),
            default: Some("Неопределено".to_string()),
            description: "Строки для копирования.".to_string(),
        }]),
        description: ArticleField::Empty,
        returns: ArticleField::Parsed(ArticleReturn {
            types: vec![TypeMention {
                text: "ТаблицаЗначений".to_string(),
                document_id: Some(
                    "platform-syntax-help:syntax-context:ValueTable.html".to_string(),
                ),
            }],
            type_text: "ТаблицаЗначений".to_string(),
            description: String::new(),
        }),
    }]);
    source.availability =
        ArticleField::Parsed(vec!["server".to_string(), "thick-client".to_string()]);
    source.since_version = ArticleField::Parsed("8.1".to_string());
    source.links = vec![ArticleLink {
        label: "Массив".to_string(),
        document_id: "platform-syntax-help:syntax-context:Array.html".to_string(),
    }];
    let value = article(&source);

    assert_eq!(value["kind"], json!("method"));
    assert_eq!(value["name"], json!({ "ru": "Скопировать", "en": "Copy" }));
    assert_eq!(
        value["ownerDocumentId"],
        json!("platform-syntax-help:syntax-context:ValueTable.html")
    );
    let parameter = &value["variants"][0]["parameters"][0];
    assert_eq!(parameter["optional"], json!(true));
    assert_eq!(parameter["default"], json!("Неопределено"));
    assert_eq!(
        parameter["types"][0]["documentId"],
        json!("platform-syntax-help:syntax-context:Array.html")
    );
    assert_eq!(value["availability"], json!(["server", "thick-client"]));
    assert_eq!(value["sinceVersion"], json!("8.1"));
    assert_eq!(
        value["links"][0],
        json!({ "label": "Массив", "documentId": "platform-syntax-help:syntax-context:Array.html" })
    );
}
