use super::*;
use crate::model::{Param, PlatformType, Property};

fn reference(id: &str) -> TypeRef {
    TypeRef {
        ty: TypeName::Context(id.into()),
        element: None,
    }
}

fn method(ru: &str, returns: &[&str]) -> Method {
    Method {
        ru: ru.into(),
        en: String::new(),
        variants: vec![Signature {
            name: String::new(),
            params: vec![Param {
                ru: "Параметр".into(),
                en: "Parameter".into(),
                optional: true,
                types: Vec::new(),
                default: None,
            }],
            returns: returns.iter().map(|id| reference(id)).collect(),
        }],
        contexts: Vec::new(),
        server_call: false,
    }
}

fn ty(
    id: &str,
    ru: &str,
    en: &str,
    properties: Vec<Property>,
    methods: Vec<Method>,
) -> PlatformType {
    PlatformType {
        id: id.into(),
        ru: ru.into(),
        en: en.into(),
        properties,
        methods,
        events: Vec::new(),
        constructors: Vec::new(),
        collection_element: Vec::new(),
        contexts: Vec::new(),
    }
}

fn property(ru: &str, types: &[&str]) -> Property {
    Property {
        ru: ru.into(),
        en: String::new(),
        types: types.iter().map(|id| reference(id)).collect(),
        read_only: true,
        contexts: Vec::new(),
    }
}

fn catalog() -> PlatformCatalog {
    let mut types = vec![
        ty(
            "g",
            "Глобальный контекст",
            "Global context",
            vec![property("Справочники", &["cm"])],
            Vec::new(),
        ),
        ty(
            "cm",
            "СправочникиМенеджер",
            "CatalogsManager",
            vec![property("<Имя справочника>", &["m"])],
            Vec::new(),
        ),
        ty(
            "m",
            "СправочникМенеджер.<Имя справочника>",
            "CatalogManager.<Catalog name>",
            Vec::new(),
            vec![method("СоздатьЭлемент", &["o"])],
        ),
        ty(
            "o",
            "СправочникОбъект.<Имя справочника>",
            "CatalogObject.<Catalog name>",
            vec![property("Код", &["s"])],
            vec![method("Записать", &[])],
        ),
        ty(
            "q",
            "Запрос",
            "Query",
            Vec::new(),
            vec![method("Выполнить", &["r"])],
        ),
        ty(
            "r",
            "РезультатЗапроса",
            "QueryResult",
            Vec::new(),
            vec![method("Выбрать", &["sel"])],
        ),
        ty("s", "Строка", "String", Vec::new(), Vec::new()),
        ty("sel", "Выборка", "Selection", Vec::new(), Vec::new()),
    ];
    types[0].id = "objects/Global context".into();
    types.sort_by(|a, b| a.id.cmp(&b.id));
    PlatformCatalog {
        platform: "test".into(),
        generator: crate::GENERATOR,
        types,
    }
}

struct Configuration;

impl AppliedTypes for Configuration {
    fn names(&self, kind: &str) -> Vec<String> {
        if kind == "Catalog" {
            vec!["Валюты".into(), "Номенклатура".into()]
        } else {
            Vec::new()
        }
    }

    fn members(&self, kind: &str, name: &str, facet: &str) -> Vec<AppliedMember> {
        match (kind, name, facet) {
            ("Catalog", "Валюты", "Object") => vec![AppliedMember {
                name: "Курс".into(),
                kind: AppliedMemberKind::Attribute,
                types: Vec::new(),
                signature: None,
            }],
            ("Catalog", "Валюты", "Manager") => vec![AppliedMember {
                name: "Пересчитать".into(),
                kind: AppliedMemberKind::Method,
                types: Vec::new(),
                signature: Some(Signature {
                    name: String::new(),
                    params: Vec::new(),
                    returns: vec![reference("q")],
                }),
            }],
            _ => Vec::new(),
        }
    }
}

fn at(id: &str) -> Ty {
    Ty {
        id: id.into(),
        applied: None,
    }
}

#[test]
fn a_chain_keeps_its_applied_object_through_template_types() {
    let catalog = catalog();
    let global = at("objects/Global context");
    let managers = catalog.member_types(&global, "справочники", false, &Configuration);
    assert_eq!(managers, vec![at("cm")]);
    let manager = catalog.member_types(&managers[0], "ВАЛЮТЫ", false, &Configuration);
    assert_eq!(
        manager,
        vec![Ty {
            id: "m".into(),
            applied: Some("Валюты".into())
        }]
    );
    // An object the configuration does not have is not a type.
    assert!(catalog
        .member_types(&managers[0], "Нет", false, &Configuration)
        .is_empty());

    let object = catalog.member_types(&manager[0], "СоздатьЭлемент", true, &Configuration);
    assert_eq!(object[0].applied.as_deref(), Some("Валюты"));
    assert_eq!(catalog.ty_name(&object[0]), "СправочникОбъект.Валюты");
    // A plain platform type has no applied object.
    assert_eq!(
        catalog.member_types(&object[0], "Код", false, &Configuration),
        vec![at("s")]
    );
    // A manager module method of the configuration types its call too.
    assert_eq!(
        catalog.member_types(&manager[0], "пересчитать", true, &Configuration),
        vec![at("q")]
    );

    let names = |receiver: &Ty| {
        catalog
            .completion_members(receiver, &Configuration)
            .into_iter()
            .map(|item| (item.name, item.kind))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        names(&managers[0]),
        vec![
            ("Валюты".to_owned(), MemberItemKind::AppliedObject),
            ("Номенклатура".to_owned(), MemberItemKind::AppliedObject)
        ]
    );
    assert_eq!(
        names(&object[0]),
        vec![
            ("Курс".to_owned(), MemberItemKind::Attribute),
            ("Код".to_owned(), MemberItemKind::Property),
            ("Записать".to_owned(), MemberItemKind::Method)
        ]
    );
}

#[test]
fn a_platform_chain_follows_return_types_and_labels_its_methods() {
    let catalog = catalog();
    let result = catalog.member_types(&at("q"), "Выполнить", true, &NoAppliedTypes);
    assert_eq!(result, vec![at("r")]);
    assert_eq!(
        catalog.member_types(&result[0], "Выбрать", true, &NoAppliedTypes),
        vec![at("sel")]
    );
    // A property is not a method and a method is not a property.
    assert!(catalog
        .member_types(&at("q"), "Выполнить", false, &NoAppliedTypes)
        .is_empty());
    let items = catalog.completion_members(&at("q"), &NoAppliedTypes);
    assert_eq!(
        items[0].detail.as_deref(),
        Some("Выполнить(Параметр): РезультатЗапроса")
    );
    assert_eq!(
        template_of(catalog.type_by_id("o").unwrap()),
        Some(("Catalog", "Object".into()))
    );
    assert_eq!(catalog.template("Catalog", "Manager").unwrap().id, "m");
}
