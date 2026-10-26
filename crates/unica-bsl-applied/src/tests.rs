use super::*;
use unica_bsl_types::Context;

/// A catalog stub: template pages and collections by English name, the
/// two applied prefixes the tests write, and a few platform types.
struct Catalog;

impl Templates for Catalog {
    fn template(&self, ty: &AppliedType) -> Option<TypeId> {
        let facet = match &ty.facet {
            AppliedFacet::Manager => "Manager",
            AppliedFacet::Object => "Object",
            AppliedFacet::Ref => "Ref",
            AppliedFacet::TabularSection(_) => return Some("tabular/Section".into()),
            AppliedFacet::TabularSectionRow(_) => return Some("tabular/Line".into()),
            AppliedFacet::Module => return None,
            _ => "Other",
        };
        Some(format!("templates/{}{facet}", ty.kind))
    }

    fn collection(&self, kind: &str) -> Option<TypeId> {
        Some(format!("collections/{kind}sManager"))
    }

    fn applied_prefix(&self, prefix: &str) -> Option<(String, AppliedFacet)> {
        match prefix {
            "СправочникСсылка" | "CatalogRef" => {
                Some(("Catalog".into(), AppliedFacet::Ref))
            }
            "СправочникОбъект" => Some(("Catalog".into(), AppliedFacet::Object)),
            _ => None,
        }
    }

    fn platform(&self, name: &str) -> Option<TypeName> {
        match name {
            "Строка" | "String" => Some(TypeName::Primitive("String".into())),
            "Неопределено" => Some(TypeName::Primitive("Undefined".into())),
            "Массив" => Some(TypeName::Context("objects/Array".into())),
            "Структура" => Some(TypeName::Context("objects/Structure".into())),
            "ХранилищеЗначения" | "ValueStorage" => {
                Some(TypeName::Context("objects/ValueStorage".into()))
            }
            _ => None,
        }
    }
}

fn applied(kind: &str, name: &str, facet: AppliedFacet) -> TypeName {
    TypeName::Applied(AppliedType {
        kind: kind.into(),
        name: name.into(),
        facet,
    })
}

fn field(name: &str, role: FieldRole, types: Vec<FieldType>) -> FieldFacts {
    FieldFacts {
        name: name.into(),
        at: format!("main:X.{name}"),
        role,
        types,
    }
}

fn method(name: &str, export: bool, function: bool, doc: &str) -> MethodFacts {
    MethodFacts {
        name: name.into(),
        at: format!("main:X.Method.{name}"),
        export,
        function,
        area: Some("public".into()),
        params: vec![ParamFacts {
            name: "Ссылка".into(),
            optional: false,
            default: None,
        }],
        doc: Some(doc.into()),
        contexts: vec![Context::Server],
    }
}

fn currencies() -> ObjectFacts {
    ObjectFacts {
        kind: "Catalog".into(),
        name: "Валюты".into(),
        at: "main:Catalog.Валюты".into(),
        fields: vec![
            field(
                "Основная",
                FieldRole::Attribute,
                vec![FieldType::Reference {
                    kind: "Catalog".into(),
                    name: "Валюты".into(),
                }],
            ),
            field("Код2", FieldRole::Attribute, vec![FieldType::Defined("Код".into())]),
            field("Странный", FieldRole::Attribute, vec![FieldType::Unknown]),
        ],
        tabular_sections: vec![SectionFacts {
            name: "Курсы".into(),
            at: "main:Catalog.Валюты.TabularSection.Курсы".into(),
            fields: vec![field(
                "Курс",
                FieldRole::Attribute,
                vec![FieldType::Primitive("Number".into())],
            )],
        }],
        predefined: vec!["Рубль".into()],
        modules: vec![ModuleFacts {
            kind: ModuleKind::Manager,
            methods: vec![
                method(
                    "НайтиПоКоду",
                    true,
                    true,
                    "Параметры:\n  Ссылка - СправочникСсылка.Валюты, Строка - the currency.\n\nВозвращаемое значение:\n  СправочникСсылка.Валюты - found.\n  Неопределено - nothing.",
                ),
                method("Скрытый", false, false, ""),
            ],
        }],
        partial: false,
    }
}

fn model() -> AppliedModel {
    let module = ObjectFacts {
        kind: "CommonModule".into(),
        name: "ОбщегоНазначения".into(),
        at: "main:CommonModule.ОбщегоНазначения".into(),
        fields: Vec::new(),
        tabular_sections: Vec::new(),
        predefined: Vec::new(),
        modules: vec![ModuleFacts {
            kind: ModuleKind::Common,
            methods: vec![method(
                "Свойства",
                true,
                true,
                "Возвращаемое значение:\n  Структура:\n    * Ключ - Строка - a key.\n  Массив из СправочникСсылка.Валюты - refs.\n  Произвольный - anything.",
            )],
        }],
        partial: false,
    };
    build(
        &[currencies(), module],
        &[DefinedTypeFacts {
            name: "Код".into(),
            types: vec![
                FieldType::Primitive("String".into()),
                FieldType::Platform("ХранилищеЗначения".into()),
            ],
        }],
        &Catalog,
    )
}

#[test]
fn managers_objects_and_references_carry_typed_members_and_their_templates() {
    let model = model();
    let manager = model
        .members(&applied("Catalog", "Валюты", AppliedFacet::Manager))
        .unwrap();
    assert_eq!(
        manager.platform.as_deref(),
        Some("templates/CatalogManager")
    );
    // A predefined item is a reference of the catalog.
    assert_eq!(manager.properties[0].property.ru, "Рубль");
    assert_eq!(
        manager.properties[0].property.types[0].ty,
        applied("Catalog", "Валюты", AppliedFacet::Ref)
    );
    // Only exported methods, with documented parameter and result types.
    assert_eq!(manager.methods.len(), 1);
    let found = &manager.methods[0];
    assert_eq!(found.area.as_deref(), Some("public"));
    let signature = &found.method.variants[0];
    assert_eq!(
        signature.params[0]
            .types
            .iter()
            .map(|ty| ty.ty.clone())
            .collect::<Vec<_>>(),
        [
            applied("Catalog", "Валюты", AppliedFacet::Ref),
            TypeName::Primitive("String".into())
        ]
    );
    assert_eq!(
        signature
            .returns
            .iter()
            .map(|ty| ty.ty.clone())
            .collect::<Vec<_>>(),
        [
            applied("Catalog", "Валюты", AppliedFacet::Ref),
            TypeName::Primitive("Undefined".into())
        ]
    );

    let object = model
        .members(&applied("Catalog", "Валюты", AppliedFacet::Object))
        .unwrap();
    let names = object
        .properties
        .iter()
        .map(|property| property.property.ru.as_str())
        .collect::<Vec<_>>();
    assert_eq!(names, ["Основная", "Код2", "Странный", "Курсы"]);
    // A reference attribute continues the chain; a defined type expands; an
    // unknown type leaves the member without types.
    assert_eq!(
        object.properties[0].property.types[0].ty,
        applied("Catalog", "Валюты", AppliedFacet::Ref)
    );
    assert_eq!(
        object.properties[1]
            .property
            .types
            .iter()
            .map(|ty| ty.ty.clone())
            .collect::<Vec<_>>(),
        [
            TypeName::Primitive("String".into()),
            TypeName::Context("objects/ValueStorage".into())
        ]
    );
    assert!(object.properties[2].property.types.is_empty());
    assert!(!object.properties[0].property.read_only);
    assert_eq!(
        object.properties[3].property.types[0].ty,
        applied(
            "Catalog",
            "Валюты",
            AppliedFacet::TabularSection("Курсы".into())
        )
    );
    let reference = model
        .members(&applied("Catalog", "Валюты", AppliedFacet::Ref))
        .unwrap();
    assert!(reference.properties.iter().all(|p| p.property.read_only));
    assert!(reference.methods.is_empty());

    let row = model
        .members(&applied(
            "Catalog",
            "Валюты",
            AppliedFacet::TabularSectionRow("Курсы".into()),
        ))
        .unwrap();
    assert_eq!(row.platform.as_deref(), Some("tabular/Line"));
    assert_eq!(row.properties[0].property.ru, "Курс");
}

#[test]
fn collections_and_common_modules_answer_their_names() {
    let model = model();
    let catalogs = model
        .members(&TypeName::Context("collections/CatalogsManager".into()))
        .unwrap();
    assert_eq!(catalogs.properties[0].property.ru, "Валюты");
    assert_eq!(
        catalogs.properties[0].property.types[0].ty,
        applied("Catalog", "Валюты", AppliedFacet::Manager)
    );
    assert_eq!(
        model.global("общегоназначения").unwrap().ty,
        applied("CommonModule", "ОбщегоНазначения", AppliedFacet::Module)
    );
    assert!(model.global("Справочники").is_none());
    let module = model
        .members(&applied(
            "CommonModule",
            "ОбщегоНазначения",
            AppliedFacet::Module,
        ))
        .unwrap();
    let returns = &module.methods[0].method.variants[0].returns;
    // Keys of a structure are not results; an array keeps its element; a
    // name nothing knows stays as written.
    assert_eq!(
        returns.iter().map(|ty| ty.ty.clone()).collect::<Vec<_>>(),
        [
            TypeName::Context("objects/Structure".into()),
            TypeName::Context("objects/Array".into()),
            TypeName::Unresolved("Произвольный".into()),
        ]
    );
    assert_eq!(
        returns[1].element,
        Some(applied("Catalog", "Валюты", AppliedFacet::Ref))
    );
    assert!(model
        .members(&TypeName::Context("collections/DocumentsManager".into()))
        .is_none());
}

#[test]
fn registers_split_dimensions_from_resources_and_partial_facts_mark_members() {
    let rates = ObjectFacts {
        kind: "InformationRegister".into(),
        name: "Курсы".into(),
        at: "main:InformationRegister.Курсы".into(),
        fields: vec![
            field(
                "Валюта",
                FieldRole::Dimension,
                vec![FieldType::Reference {
                    kind: "Catalog".into(),
                    name: "Валюты".into(),
                }],
            ),
            field(
                "Курс",
                FieldRole::Resource,
                vec![FieldType::Primitive("Number".into())],
            ),
        ],
        tabular_sections: Vec::new(),
        predefined: Vec::new(),
        modules: Vec::new(),
        partial: true,
    };
    let model = build(&[rates], &[], &Catalog);
    let names = |facet: AppliedFacet| {
        let members = model
            .members(&applied("InformationRegister", "Курсы", facet))
            .unwrap();
        assert!(members.partial);
        members
            .properties
            .iter()
            .map(|property| property.property.ru.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(names(AppliedFacet::Record), ["Валюта", "Курс"]);
    assert_eq!(names(AppliedFacet::RecordManager), ["Валюта", "Курс"]);
    assert_eq!(names(AppliedFacet::RecordKey), ["Валюта"]);
    assert!(names(AppliedFacet::RecordSet).is_empty());
    assert!(model
        .members(&applied(
            "InformationRegister",
            "Курсы",
            AppliedFacet::Object
        ))
        .is_none());
}

#[test]
fn a_comment_yields_only_type_texts_of_its_parameter_and_result_sections() {
    let types = doc::doc_types(
        "Описание.\n\nПараметры:\n  Ссылка - ЛюбаяСсылка - описание с - дефисом.\n         - Строка - alternative.\n  Текст продолжения описания - не параметр\n  Флаг - Булево\n\nВозвращаемое значение:\n  Булево, Неопределено - result.\n\nПример:\n  Строка - not a type section.",
    );
    assert_eq!(
        types.params,
        [
            (
                "Ссылка".to_string(),
                vec!["ЛюбаяСсылка".to_string(), "Строка".to_string()]
            ),
            ("Флаг".to_string(), vec!["Булево".to_string()]),
        ]
    );
    assert_eq!(types.returns, ["Булево", "Неопределено"]);
}
