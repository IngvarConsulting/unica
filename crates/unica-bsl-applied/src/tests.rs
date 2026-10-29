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
            "Число" | "Number" => Some(TypeName::Primitive("Number".into())),
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
            server_call: false,
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
        subordinate: false,
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
            server_call: false,
            kind: ModuleKind::Common,
            methods: vec![method(
                "Свойства",
                true,
                true,
                "Возвращаемое значение:\n  Структура:\n    * Ключ - Строка - a key.\n  Массив из СправочникСсылка.Валюты - refs.\n  Произвольный - anything.",
            )],
        }],
        subordinate: false,
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
        &(),
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
    // A tabular section walks and adds its rows.
    let section = model
        .members(&applied(
            "Catalog",
            "Валюты",
            AppliedFacet::TabularSection("Курсы".into()),
        ))
        .unwrap();
    assert_eq!(
        section.element.as_ref().unwrap().ty,
        applied(
            "Catalog",
            "Валюты",
            AppliedFacet::TabularSectionRow("Курсы".into())
        )
    );
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
        subordinate: false,
        partial: true,
    };
    let model = build(&[rates], &[], &(), &Catalog);
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
    let set = model
        .members(&applied(
            "InformationRegister",
            "Курсы",
            AppliedFacet::RecordSet,
        ))
        .unwrap();
    assert_eq!(
        set.element.as_ref().unwrap().ty,
        applied("InformationRegister", "Курсы", AppliedFacet::Record)
    );
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

#[test]
fn a_subordinate_register_keys_by_recorder_and_has_no_record_manager() {
    let movements = ObjectFacts {
        kind: "AccumulationRegister".into(),
        name: "Остатки".into(),
        at: "main:AccumulationRegister.Остатки".into(),
        fields: vec![field("Склад", FieldRole::Dimension, Vec::new())],
        tabular_sections: Vec::new(),
        predefined: Vec::new(),
        modules: Vec::new(),
        subordinate: true,
        partial: false,
    };
    let constant = ObjectFacts {
        kind: "Constant".into(),
        name: "ОсновнаяВалюта".into(),
        at: "main:Constant.ОсновнаяВалюта".into(),
        fields: vec![field(
            "Значение",
            FieldRole::Attribute,
            vec![FieldType::Reference {
                kind: "Catalog".into(),
                name: "Валюты".into(),
            }],
        )],
        tabular_sections: Vec::new(),
        predefined: Vec::new(),
        modules: Vec::new(),
        subordinate: false,
        partial: false,
    };
    let unknown = ObjectFacts {
        kind: "FilterCriterion".into(),
        name: "Связи".into(),
        at: "main:FilterCriterion.Связи".into(),
        fields: Vec::new(),
        tabular_sections: Vec::new(),
        predefined: Vec::new(),
        modules: Vec::new(),
        subordinate: false,
        partial: false,
    };
    let model = build(&[movements, constant, unknown], &[], &(), &Catalog);
    let key = model
        .members(&applied(
            "AccumulationRegister",
            "Остатки",
            AppliedFacet::RecordKey,
        ))
        .unwrap();
    assert!(key.properties.is_empty());
    assert!(model
        .members(&applied(
            "AccumulationRegister",
            "Остатки",
            AppliedFacet::RecordManager
        ))
        .is_none());
    // The value of a constant continues the chain.
    let value = model
        .members(&applied(
            "Constant",
            "ОсновнаяВалюта",
            AppliedFacet::ValueManager,
        ))
        .unwrap();
    assert_eq!(
        value.properties[0].property.types[0].ty,
        applied("Catalog", "Валюты", AppliedFacet::Ref)
    );
    // A kind the model does not know gets no dangling manager.
    assert!(model
        .members(&TypeName::Context(
            "collections/FilterCriterionsManager".into()
        ))
        .is_none());
}

#[test]
fn comment_types_skip_descriptions_structure_keys_and_other_sections() {
    let types = doc::doc_types(
        "Описание.\n\nПараметры:\n  Флаг - Булево - признак\n  Настройки - Структура:\n    * Период - Дата\n             - СтандартныйПериод\n  Ссылка - ЛюбаяСсылка - описание с - дефисом;\n           Иначе - Ложь.\n         - Строка - alternative.\n\nВозвращаемое значение:\n  Строка - представление\n    документа.\n  Булево - Истина, если\n    объект - новый.\nПример:\n  Строка - not a type section.",
    );
    assert_eq!(
        types.params,
        [
            ("Флаг".to_string(), vec!["Булево".to_string()]),
            ("Настройки".to_string(), vec!["Структура".to_string()]),
            (
                "Ссылка".to_string(),
                vec!["ЛюбаяСсылка".to_string(), "Строка".to_string()]
            ),
        ]
    );
    assert_eq!(types.returns, ["Строка", "Булево"]);
}

#[test]
fn comment_names_resolve_to_declared_objects_only() {
    let model = model();
    let module = model
        .members(&applied(
            "CommonModule",
            "ОбщегоНазначения",
            AppliedFacet::Module,
        ))
        .unwrap();
    assert!(!module.methods[0].method.server_call);
    let builder_model = build(&[currencies()], &[], &(), &Catalog);
    let manager = builder_model
        .members(&applied("Catalog", "Валюты", AppliedFacet::Manager))
        .unwrap();
    // `СправочникСсылка.Валюты` in the comment matched the declared spelling;
    // an object the configuration does not declare is not invented.
    assert_eq!(
        manager.methods[0].method.variants[0].returns[0].ty,
        applied("Catalog", "Валюты", AppliedFacet::Ref)
    );
    let mut lower = currencies();
    lower.modules[0].methods[0].doc = Some(
        "Возвращаемое значение:\n  СправочникСсылка.валюты - ok.\n  СправочникСсылка.Чужой - unknown.".into(),
    );
    let model = build(&[lower], &[], &(), &Catalog);
    let manager = model
        .members(&applied("Catalog", "Валюты", AppliedFacet::Manager))
        .unwrap();
    assert_eq!(
        manager.methods[0].method.variants[0]
            .returns
            .iter()
            .map(|ty| ty.ty.clone())
            .collect::<Vec<_>>(),
        [
            applied("Catalog", "Валюты", AppliedFacet::Ref),
            TypeName::Unresolved("СправочникСсылка.Чужой".into()),
        ]
    );
}

#[test]
fn a_model_of_one_object_resolves_comment_names_through_the_declared_objects() {
    struct Configuration;
    impl Declared for Configuration {
        fn declared(&self, kind: &str, name: &str) -> Option<String> {
            (kind == "Catalog" && name.to_lowercase() == "контрагенты")
                .then(|| "Контрагенты".to_string())
        }
    }
    let mut currencies = currencies();
    currencies.modules[0].methods[0].doc =
        Some("Возвращаемое значение:\n  СправочникСсылка.КОНТРАГЕНТЫ - another object.".into());
    let model = build(&[currencies], &[], &Configuration, &Catalog);
    let manager = model
        .members(&applied("Catalog", "Валюты", AppliedFacet::Manager))
        .unwrap();
    assert_eq!(
        manager.methods[0].method.variants[0].returns[0].ty,
        applied("Catalog", "Контрагенты", AppliedFacet::Ref)
    );
}

fn documented_signature(doc: &str) -> Signature {
    let mut object = currencies();
    object.modules[0].methods = vec![method("Проверить", true, true, doc)];
    object.modules[0].methods[0].params[0].name = "Значение".into();
    build(&[object], &[], &(), &Catalog)
        .members(&applied("Catalog", "Валюты", AppliedFacet::Manager))
        .unwrap()
        .methods[0]
        .method
        .variants[0]
        .clone()
}

#[test]
fn nested_array_alternatives_preserve_parameter_and_return_types() {
    let number = TypeRef {
        ty: TypeName::Primitive("Number".into()),
        element: None,
    };
    let nested = TypeRef {
        ty: TypeName::Context("objects/Array".into()),
        element: Some(TypeName::Context("objects/Array".into())),
    };
    let mut observed = Vec::new();
    let mut wanted = Vec::new();
    for (types, expected) in [
        (
            "Число, Массив из Массив из Строка",
            vec![number.clone(), nested.clone()],
        ),
        (
            "Массив из Массив из Строка, Число",
            vec![nested.clone(), number.clone()],
        ),
        (
            "Number, Array of Array of String",
            vec![number.clone(), nested.clone()],
        ),
        (
            "Array of Array of String, Number",
            vec![nested.clone(), number.clone()],
        ),
    ] {
        let doc = format!("Параметры:\n  Значение - {types} - описание.\nВозвращаемое значение:\n  {types} - описание.");
        let signature = documented_signature(&doc);
        observed.push((types, signature.params[0].types.clone(), signature.returns));
        wanted.push((types, expected.clone(), expected));
    }
    assert_eq!(observed, wanted);
}

#[test]
fn ordinary_arrays_unknown_names_and_prose_keep_their_boundaries() {
    let n = TypeRef {
        ty: TypeName::Primitive("Number".into()),
        element: None,
    };
    let string = TypeRef {
        ty: TypeName::Primitive("String".into()),
        element: None,
    };
    let array = TypeRef {
        ty: TypeName::Context("objects/Array".into()),
        element: Some(TypeName::Primitive("String".into())),
    };
    for (text, expected) in [
        ("Число", vec![n.clone()]),
        ("Массив из Строка", vec![array.clone()]),
        ("Число, Массив из Строка", vec![n.clone(), array]),
        (
            "Число, СправочникСсылка.Чужой",
            vec![
                n.clone(),
                TypeRef {
                    ty: TypeName::Unresolved("СправочникСсылка.Чужой".into()),
                    element: None,
                },
            ],
        ),
        (
            "Число, НераспознанныйТип",
            vec![
                n,
                TypeRef {
                    ty: TypeName::Unresolved("НераспознанныйТип".into()),
                    element: None,
                },
            ],
        ),
        ("Массив из ", vec![]),
        ("List<Строка>", vec![]),
        ("Array[Строка]", vec![]),
        ("Массив из Строка[]", vec![]),
        ("Число, если успешно", vec![]),
        ("Число, List<Строка>", vec![]),
    ] {
        let doc = format!("Параметры:\n  Значение - {text} - описание.\nВозвращаемое значение:\n  {text} - описание.");
        let signature = documented_signature(&doc);
        assert_eq!(signature.params[0].types, expected, "parameter: {text}");
        assert_eq!(signature.returns, expected, "return: {text}");
    }
    let signature = documented_signature("Параметры:\n  Значение - Число - основной тип.\n    Массив из Массив из Строка - продолжение описания.\nВозвращаемое значение:\n  Строка - результат.\n    Число, Массив из Массив из Строка - подробное описание.\nПример:\n  Массив из Массив из Строка - пример.");
    assert_eq!(
        signature.params[0].types,
        vec![TypeRef {
            ty: TypeName::Primitive("Number".into()),
            element: None
        }]
    );
    assert_eq!(signature.returns, vec![string]);
    let signature =
        documented_signature("Описание.\n  Число, Массив из Массив из Строка - не типовой раздел.");
    assert!(signature.params[0].types.is_empty());
    assert!(signature.returns.is_empty());
}

#[test]
fn deeply_nested_arrays_keep_only_the_representable_element_without_recursion() {
    let text = format!("{}Строка", "Массив из ".repeat(10_000));
    let signature = documented_signature(&format!("Возвращаемое значение:\n  {text} - глубина."));
    assert_eq!(
        signature.returns,
        vec![TypeRef {
            ty: TypeName::Context("objects/Array".into()),
            element: Some(TypeName::Context("objects/Array".into()))
        }]
    );
}

#[test]
fn method_addresses_and_areas_are_preserved() {
    for area in [
        Some("public"),
        Some("internal"),
        Some("crossSubsystem"),
        Some("override"),
        Some("private"),
        Some("deprecated"),
        None,
    ] {
        let mut object = currencies();
        let mut facts = method("Проверить", true, true, "");
        facts.at = "ext:Catalog.Валюты.Module.Manager.Method.Проверить".into();
        facts.area = area.map(str::to_string);
        object.modules[0].methods = vec![facts.clone()];
        let model = build(&[object], &[], &(), &Catalog);
        let members = model
            .members(&applied("Catalog", "Валюты", AppliedFacet::Manager))
            .unwrap();
        assert_eq!(members.methods.len(), 1);
        assert_eq!(members.methods[0].at, facts.at);
        assert_eq!(members.methods[0].area, facts.area);
    }
}

#[test]
fn collection_partial_is_the_union_of_each_distinct_object() {
    for (first_partial, second_partial) in [(false, true), (true, false), (false, false)] {
        let objects = [first_partial, second_partial]
            .into_iter()
            .enumerate()
            .map(|(index, partial)| {
                let mut object = currencies();
                object.name = format!("Справочник{index}");
                object.at = format!("main:Catalog.Справочник{index}");
                object.fields.clear();
                object.modules.clear();
                object.predefined.clear();
                object.tabular_sections.clear();
                object.partial = partial;
                object
            })
            .collect::<Vec<_>>();
        let model = build(&objects, &[], &(), &Catalog);
        let collection = model
            .members(&TypeName::Context("collections/CatalogsManager".into()))
            .unwrap();
        assert_eq!(collection.partial, first_partial || second_partial);
        assert_eq!(collection.properties.len(), 2);
        for (index, object) in objects.iter().enumerate() {
            let property = &collection.properties[index];
            assert_eq!(property.at.as_deref(), Some(object.at.as_str()));
            assert_eq!(property.property.ru, object.name);
            assert_eq!(
                property.property.types[0].ty,
                applied("Catalog", &object.name, AppliedFacet::Manager)
            );
            for facet in facets(object) {
                assert_eq!(
                    model
                        .members(&applied("Catalog", &object.name, facet.clone()))
                        .unwrap()
                        .partial,
                    object.partial
                );
            }
        }
    }
}
