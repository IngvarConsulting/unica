use super::*;

fn page(body: String) -> String {
    format!("<html><body>{body}</body></html>")
}

const QUERY_RESULT: &str =
    "<a href=\"v8help://SyntaxHelperContext/objects/c/Result.html\">Результат</a>";

fn russian() -> Vec<(String, String)> {
    vec![
        ("objects/c/Sample.html".into(), page("<p class=\"V8SH_title\">Пример (Sample)</p><p class=\"V8SH_chapter\">Доступность:</p><p>Сервер.</p>".into())),
        ("objects/c/Result.html".into(), page("<p class=\"V8SH_title\">Результат (Result)</p>".into())),
        ("objects/c/Sample/methods/Run1.html".into(), page(format!(
            "<p class=\"V8SH_heading\">Выполнить (Run)</p><p class=\"V8SH_chapter\">Синтаксис:</p>Выполнить(&lt;Режим&gt;)\
             <p class=\"V8SH_chapter\">Параметры:</p><div class=\"V8SH_rubric\"> <p>&lt;Режим&gt; (необязательный)</div>Тип: <a href=\"v8help://SyntaxHelperContext/objects/gone/Mode.html\">Режим</a>.\
             <p class=\"V8SH_chapter\">Возвращаемое значение:</p>Тип: {QUERY_RESULT}."))),
        ("objects/c/Sample/ctors/ctor1.html".into(), page(
            "<p class=\"V8SH_heading\">По тексту</p><p class=\"V8SH_chapter\">Синтаксис:</p>Новый Пример(&lt;Текст&gt;)\
             <p class=\"V8SH_chapter\">Параметры:</p><div class=\"V8SH_rubric\"> <p>&lt;Текст&gt; (обязательный)</div>Тип: <a href=\"v8help://SyntaxHelperLanguage/def_String\">Строка</a>.".into())),
        ("objects/c/Sample/properties/Text1.html".into(), page(
            "<p class=\"V8SH_heading\">Текст (Text)</p><p class=\"V8SH_chapter\">Использование:</p>Чтение и запись.".into())),
        ("objects/Global context.html".into(), page("<p class=\"V8SH_title\">Глобальный контекст (Global context)</p>".into())),
        ("objects/Global context/methods/Filled1.html".into(), page(
            "<p class=\"V8SH_heading\">ЗначениеЗаполнено (ValueIsFilled)</p><p class=\"V8SH_chapter\">Синтаксис:</p>ЗначениеЗаполнено(&lt;Значение&gt;)\
             <p class=\"V8SH_chapter\">Параметры:</p><div class=\"V8SH_rubric\"> <p>&lt;Значение&gt; (обязательный)</div>Тип: Произвольный.\
             <p class=\"V8SH_chapter\">Возвращаемое значение:</p>Тип: <a href=\"v8help://SyntaxHelperLanguage/def_Boolean\">Булево</a>.".into())),
    ]
}

fn english() -> Vec<(String, String)> {
    vec![
        ("objects/c/Sample/methods/Run1.html".into(), page(
            "<p class=\"V8SH_heading\">Run</p><p class=\"V8SH_chapter\">Syntax:</p>Run(&lt;Mode&gt;)\
             <p class=\"V8SH_chapter\">Parameters:</p><div class=\"V8SH_rubric\"> <p>&lt;Mode&gt; (optional)</div>Type: x.".into())),
        ("objects/c/Sample/ctors/ctor1.html".into(), page(
            "<p class=\"V8SH_heading\">Based on text</p><p class=\"V8SH_chapter\">Syntax:</p>New Sample(&lt;Text&gt;)\
             <p class=\"V8SH_chapter\">Parameters:</p><div class=\"V8SH_rubric\"> <p>&lt;Text&gt; (required)</div>Type: x.".into())),
    ]
}

/// rule: INV.ANALYSIS.PLATFORM-TYPE-CATALOG
#[test]
fn one_record_per_type_and_member_in_both_languages_with_types_by_page() {
    let catalog = build_catalog("8.3.27.0", &russian(), &english());
    assert_eq!(catalog.generator, GENERATOR);
    assert_eq!(catalog.types.len(), 3);
    let sample = catalog.type_by_name("sample").unwrap();
    assert_eq!(
        (sample.ru.as_str(), sample.en.as_str()),
        ("Пример", "Sample")
    );
    assert_eq!(sample.contexts, vec![Context::Server]);

    let run = sample.method("выполнить").unwrap();
    assert_eq!(run.en, "Run");
    let variant = &run.variants[0];
    assert_eq!(
        (variant.params[0].ru.as_str(), variant.params[0].en.as_str()),
        ("Режим", "Mode")
    );
    assert!(variant.params[0].optional);
    // A link to a page the help does not have stays as written.
    assert_eq!(
        variant.params[0].types[0].ty,
        TypeName::Unresolved("objects/gone/Mode".into())
    );
    assert_eq!(
        catalog
            .type_by_id(match &variant.returns[0].ty {
                TypeName::Context(id) => id,
                other => panic!("{other:?}"),
            })
            .unwrap()
            .en,
        "Result"
    );

    assert_eq!(sample.constructors[0].name, "По тексту");
    assert_eq!(sample.constructors[0].params[0].en, "Text");
    let text = sample.property("TEXT").unwrap();
    assert!(!text.read_only);
    assert!(
        text.types.is_empty(),
        "no type in the help is no type, not a guess"
    );

    let global = catalog.type_by_name("Глобальный контекст").unwrap();
    let filled = global.method("ValueIsFilled").unwrap();
    assert_eq!(
        filled.variants[0].returns[0].ty,
        TypeName::Primitive("Boolean".into())
    );
    assert_eq!(filled.variants[0].params[0].en, "");
}

/// The applied seam (stage 2): an applied type keeps its facet through
/// serialization and orders after the platform names, and a source of the
/// configuration answers through the trait object inference holds.
#[test]
fn an_applied_type_round_trips_and_answers_through_the_seam() {
    let manager = TypeName::Applied(AppliedType {
        kind: "Catalog".into(),
        name: "Валюты".into(),
        facet: AppliedFacet::Manager,
    });
    let row = TypeName::Applied(AppliedType {
        kind: "Document".into(),
        name: "Заказ".into(),
        facet: AppliedFacet::TabularSectionRow("Товары".into()),
    });
    for name in [&manager, &row] {
        let json = serde_json::to_string(name).unwrap();
        assert_eq!(
            &serde_json::from_str::<TypeName>(&json).unwrap(),
            name,
            "{json}"
        );
    }
    assert!(TypeName::Unresolved("Произвольный".into()) < manager);

    struct Configuration;
    impl AppliedTypes for Configuration {
        fn global(&self, name: &str) -> Option<TypeRef> {
            (name.to_lowercase() == "общегоназначения").then(|| TypeRef {
                ty: TypeName::Applied(AppliedType {
                    kind: "CommonModule".into(),
                    name: "ОбщегоНазначения".into(),
                    facet: AppliedFacet::Module,
                }),
                element: None,
            })
        }

        fn members(&self, ty: &TypeName) -> Option<std::sync::Arc<AppliedMembers>> {
            matches!(ty, TypeName::Context(id) if id.ends_with("/CatalogsManager")).then(|| {
                std::sync::Arc::new(AppliedMembers {
                    platform: None,
                    properties: vec![AppliedProperty {
                        property: Property {
                            ru: "Валюты".into(),
                            en: "Валюты".into(),
                            types: vec![TypeRef {
                                ty: TypeName::Applied(AppliedType {
                                    kind: "Catalog".into(),
                                    name: "Валюты".into(),
                                    facet: AppliedFacet::Manager,
                                }),
                                element: None,
                            }],
                            read_only: true,
                            contexts: Vec::new(),
                        },
                        at: Some("main:Catalog.Валюты".into()),
                    }],
                    methods: Vec::new(),
                    element: None,
                    partial: false,
                })
            })
        }
    }
    let shared: std::sync::Arc<dyn AppliedTypes> = std::sync::Arc::new(Configuration);
    std::thread::spawn(move || shared.global("общегоназначения"))
        .join()
        .unwrap()
        .expect("a source is shared across threads");
    let sources: [&dyn AppliedTypes; 2] = [&Configuration, &NoAppliedTypes];
    assert!(sources[0].global("ОбщегоНазначения").is_some());
    assert!(sources[1].global("ОбщегоНазначения").is_none());
    let catalogs = TypeName::Context("objects/c/CatalogsManager".into());
    let members = sources[0].members(&catalogs).unwrap();
    assert_eq!(members.properties[0].property.types[0].ty, manager);
    assert!(sources[1].members(&catalogs).is_none());
}
