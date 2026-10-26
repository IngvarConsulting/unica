use super::*;

fn page(body: &str) -> String {
    format!("\u{feff}<html><head></head><body>{body}</body></html>")
}

fn chapter(name: &str) -> String {
    format!("<p class=\"V8SH_chapter\">{name}</p>")
}

fn rubric(name: &str, optional: bool) -> String {
    let kind = if optional {
        "необязательный"
    } else {
        "обязательный"
    };
    format!("<div class=\"V8SH_rubric\"> <p style=\"m\">&lt;{name}&gt; ({kind})</div>")
}

const STRING: &str = "<a href=\"v8help://SyntaxHelperLanguage/def_String\">Строка</a>";
const NUMBER: &str = "<a href=\"v8help://SyntaxHelperLanguage/def_Number\">Число</a>";
const ARRAY: &str =
    "<a href=\"v8help://SyntaxHelperContext/objects/catalog1/Array.html\">Массив</a>";
const ITEM: &str =
    "<a href=\"v8help://SyntaxHelperContext/objects/catalog9/Item.html\">Элемент</a>";

#[test]
fn a_method_page_gives_its_variants_parameters_returns_and_contexts() {
    let html = page(&format!(
        "<h1 class=\"V8SH_pagetitle\">Пример.Найти (Sample.Find)</h1>\
         <p class=\"V8SH_title\">Пример (Sample)</p><p class=\"V8SH_heading\">Найти (Find)</p>\
         {}Из строки</p>{}Найти(&lt;Текст&gt;, &lt;Сколько&gt;){}{}Тип: {STRING}. <br>Что искать.{}Тип: {STRING}; {NUMBER}. <br>Значение по умолчанию: 10.\
         {}Тип: {ARRAY}. <br>Массив из элементов типа {ITEM}, найденных по тексту.\
         {}Пустой</p>{}Найти(){}Тип: {ITEM}.\
         {}<p>Сервер, толстый клиент, мобильное приложение (сервер), новое место.</p>",
        chapter("Вариант синтаксиса:").trim_end_matches("</p>"),
        chapter("Синтаксис:"),
        chapter("Параметры:"),
        rubric("Текст", false),
        rubric("Сколько", true),
        chapter("Возвращаемое значение:"),
        chapter("Вариант синтаксиса:").trim_end_matches("</p>"),
        chapter("Синтаксис:"),
        chapter("Возвращаемое значение:"),
        chapter("Доступность: "),
    ));
    let page = parse("objects/catalog9/Sample/methods/Find123.html", &html).unwrap();
    assert_eq!(page.kind, PageKind::Method);
    assert_eq!(page.owner.as_deref(), Some("objects/catalog9/Sample"));
    assert_eq!(
        (page.name.as_str(), page.alias.as_deref()),
        ("Найти", Some("Find"))
    );
    assert_eq!(page.variants.len(), 2);
    let first = &page.variants[0];
    assert_eq!(first.name, "Из строки");
    assert_eq!(first.params.len(), 2);
    assert_eq!(first.params[0].name, "Текст");
    assert!(!first.params[0].optional && first.params[1].optional);
    assert_eq!(
        first.params[1].types,
        vec![
            TypeRef {
                ty: TypeName::Primitive("String".into()),
                element: None
            },
            TypeRef {
                ty: TypeName::Primitive("Number".into()),
                element: None
            },
        ]
    );
    assert_eq!(first.params[1].default.as_deref(), Some("10"));
    assert_eq!(
        first.returns,
        vec![TypeRef {
            ty: TypeName::Context("objects/catalog1/Array".into()),
            element: Some(TypeName::Context("objects/catalog9/Item".into())),
        }]
    );
    assert_eq!(page.variants[1].name, "Пустой");
    assert!(page.variants[1].params.is_empty());
    assert_eq!(
        page.variants[1].returns[0].ty,
        TypeName::Context("objects/catalog9/Item".into())
    );
    assert_eq!(
        page.contexts,
        vec![
            Context::Server,
            Context::ThickClient,
            Context::MobileAppServer,
            Context::Other("новое место".into())
        ]
    );
}

#[test]
fn a_property_reads_its_type_and_use_and_a_type_page_its_collection() {
    let property = page(&format!(
        "<p class=\"V8SH_heading\">Текст (Text)</p>{}Только чтение.{}Тип: {STRING}. <br>Текст.",
        chapter("Использование:"),
        chapter("Описание:"),
    ));
    let property = parse("objects/c/Sample/properties/Text1.html", &property).unwrap();
    assert_eq!(property.kind, PageKind::Property);
    assert!(property.read_only);
    assert_eq!(property.types[0].ty, TypeName::Primitive("String".into()));

    let any = page(&format!(
        "<p class=\"V8SH_heading\">Значение (Value)</p>{}Тип: Произвольный.",
        chapter("Описание:")
    ));
    let any = parse("objects/c/Sample/properties/Value2.html", &any).unwrap();
    assert_eq!(any.types[0].ty, TypeName::Unresolved("Произвольный".into()));

    let ty = page(&format!(
        "<p class=\"V8SH_title\">Пример (Sample)</p>{}{ITEM}{}<p>Сервер.</p>",
        chapter("Элементы коллекции:"),
        chapter("Доступность:"),
    ));
    let ty = parse("objects/catalog9/Sample.html", &ty).unwrap();
    assert_eq!(ty.kind, PageKind::Type);
    assert_eq!(ty.owner, None);
    assert_eq!(
        ty.collection_element[0].ty,
        TypeName::Context("objects/catalog9/Item".into())
    );
    assert_eq!(ty.contexts, vec![Context::Server]);

    // Pages that are not types or their members are not read.
    assert!(parse("objects/catalog9/Sample/fields/F1.html", &ty_html()).is_none());
    assert!(parse("languages/x.html", &ty_html()).is_none());
    // A section page groups types and is not one.
    assert!(parse("objects/catalog2/catalog259.html", &ty_html()).is_none());
    // Members grouped one level deeper still belong to their type.
    let grouped = parse(
        "objects/Global context/events/catalog200/OnExit203.html",
        &page("<p class=\"V8SH_heading\">ПриЗавершении (OnExit)</p>"),
    )
    .unwrap();
    assert_eq!(grouped.kind, PageKind::Event);
    assert_eq!(grouped.owner.as_deref(), Some("objects/Global context"));
}

fn ty_html() -> String {
    page("<p class=\"V8SH_title\">X (X)</p>")
}
