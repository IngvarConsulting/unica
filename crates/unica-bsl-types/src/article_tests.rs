//! Разбор статьи проверяется на разметке, снятой со справки установленной
//! платформы 8.5.4.1878: классы, порядок тегов, `&nbsp;` и `<BR>` в примере,
//! BOM в начале файла, пробел в конце заголовка «Доступность: » — всё это
//! факты формата, а не удобные упрощения. Тексты статей здесь свои:
//! материалы вендора в репозиторий не копируются
//! (INV.PKG.PLATFORM-DOCUMENTATION-CORPUS).

use super::*;

/// Страница справки целиком: BOM, шапка документа и тело.
fn page(body: &str) -> String {
    format!(
        "\u{feff}<html><head><meta http-equiv=\"Content-Type\" content=\"text/html; charset=utf-8\">\
         </head><body>{body}</body></html>"
    )
}

fn chapter(name: &str) -> String {
    format!("<p class=\"V8SH_chapter\">{name}</p>")
}

/// Рубрика параметра: `<div>` без закрытия внутри, как пишет справка.
fn rubric(name: &str, optional: bool) -> String {
    let kind = if optional {
        "необязательный"
    } else {
        "обязательный"
    };
    format!(
        "<div class=\"V8SH_rubric\"> <p style=\"margin-top: 2px; margin-bottom: 1px\">\
         &lt;{name}&gt; ({kind})</div>"
    )
}

/// Блок примера: таблица, моноширинный абзац, `<BR>` и `&nbsp;`.
fn sample(code: &str) -> String {
    format!(
        "<TABLE width=\"100%\" bgColor=\"#f7f7f7\"><TBODY><TR><TD>\
         <p class=\"V8SH_codesample\"><font color=\"#0000ff\">{code}</p></font></font></TD></TR></TBODY></TABLE>"
    )
}

const ARRAY: &str =
    "<a href=\"v8help://SyntaxHelperContext/objects/catalog234/Array.html\">Массив</a>";
const STRUCTURE: &str =
    "<a href=\"v8help://SyntaxHelperContext/objects/catalog234/Structure.html\">Структура</a>";
const STRING: &str = "<a href=\"v8help://SyntaxHelperLanguage/def_String\">Строка</a>";
const BOOLEAN: &str = "<a href=\"v8help://SyntaxHelperLanguage/def_Boolean\">Булево</a>";
const FALSE: &str = "<a href=\"v8help://SyntaxHelperLanguage/def_BooleanFalse\">Ложь</a>";
const VALUE_TABLE: &str = "<a href=\"v8help://SyntaxHelperContext/objects/catalog234/catalog236/ValueTable.html\">ТаблицаЗначений</a>";

const COPY_PATH: &str = "objects/catalog234/catalog236/ValueTable/methods/Copy649.html";

/// Метод с двумя вариантами синтаксиса и одним разделом возврата на оба.
fn copy_article() -> String {
    page(&format!(
        "<h1 class=\"V8SH_pagetitle\">ТаблицаЗначений.Скопировать (ValueTable.Copy)</h1>\
         <p class=\"V8SH_title\">ТаблицаЗначений (ValueTable)</p>\
         <p class=\"V8SH_heading\">Скопировать (Copy)</p>\
         <div class=\"__SINCE_SHOW_STYLE__\"><p class=\"not_used\">Доступен, начиная с версии 8.1.</p></div>\
         {}Скопировать(&lt;Строки&gt;, &lt;Колонки&gt;)\
         {}{}Тип: {ARRAY}. <br>Строки для копирования.\
         {}Тип: {STRING}. <br>Список колонок.\
         {}Копируются указанные строки.\
         {}Скопировать(&lt;Отбор&gt;, &lt;Колонки&gt;)\
         {}{}Тип: {STRUCTURE}. <br>Ключ структуры — имя колонки.\
         {}Копируются строки отбора.\
         {}Тип: {VALUE_TABLE}. <br>\
         {}<p>Создаёт копию таблицы значений.</p>\
         {}<p>Сервер, толстый клиент, внешнее соединение, мобильное приложение (сервер).</p>\
         {}{}{}<p class=\"V8SH_versionInfo\">Доступен, начиная с версии 8.1.</p>",
        chapter("Вариант синтаксиса: Скопировать строки") + &chapter("Синтаксис:"),
        chapter("Параметры:"),
        rubric("Строки", true),
        rubric("Колонки", true),
        chapter("Описание варианта метода:"),
        chapter("Вариант синтаксиса: Скопировать по отбору") + &chapter("Синтаксис:"),
        chapter("Параметры:"),
        rubric("Отбор", true),
        chapter("Описание варианта метода:"),
        chapter("Возвращаемое значение:"),
        chapter("Описание:"),
        chapter("Доступность: "),
        chapter("Пример:"),
        sample("НовТаблица&nbsp;<font color=\"#ff0000\">=</font>&nbsp;Цены<font color=\"#ff0000\">.</font>Скопировать<font color=\"#ff0000\">(</font><font color=\"#ff0000\">)</font><font color=\"#ff0000\">;</font>"),
        chapter("Использование в версии:"),
    ))
}

#[test]
fn a_method_article_names_its_owner_variants_and_parameters() {
    let article = parse(COPY_PATH, &copy_article());

    assert_eq!(article.kind, ArticleKind::Method);
    assert_eq!(
        article.title,
        "ТаблицаЗначений.Скопировать (ValueTable.Copy)"
    );
    assert_eq!(
        (article.name.ru.as_str(), article.name.en.as_deref()),
        ("Скопировать", Some("Copy"))
    );
    let owner = article.owner.as_ref().expect("у метода есть владелец");
    assert_eq!(
        (owner.ru.as_str(), owner.en.as_deref()),
        ("ТаблицаЗначений", Some("ValueTable"))
    );
    assert_eq!(
        article.owner_id.as_deref(),
        Some("objects/catalog234/catalog236/ValueTable")
    );

    assert_eq!(article.variants.len(), 2);
    let first = &article.variants[0];
    assert_eq!(first.name.as_deref(), Some("Скопировать строки"));
    assert_eq!(
        first.call,
        Field::Parsed("Скопировать(<Строки>, <Колонки>)".to_string())
    );
    let params = first.parameters.parsed().expect("параметры разобраны");
    assert_eq!(params.len(), 2);
    assert_eq!(params[0].name, "Строки");
    assert!(params[0].optional);
    assert_eq!(params[0].type_text, "Массив");
    assert_eq!(params[0].description, "Строки для копирования.");
    assert_eq!(
        params[0].types,
        vec![TypeRef {
            ty: TypeName::Context("objects/catalog234/Array".into()),
            element: None
        }]
    );
    assert_eq!(
        first.description,
        Field::Parsed("Копируются указанные строки.".to_string())
    );

    let second = &article.variants[1];
    assert_eq!(second.name.as_deref(), Some("Скопировать по отбору"));
    assert_eq!(
        second
            .parameters
            .parsed()
            .expect("параметры второго варианта разобраны")[0]
            .name,
        "Отбор"
    );
}

/// Возврат, названный один раз на два варианта, — возврат метода: иначе
/// первый вариант выглядел бы ничего не возвращающим.
///
/// rule: INV.ANALYSIS.PLATFORM-TYPE-CATALOG
#[test]
fn a_single_return_section_belongs_to_every_variant() {
    let article = parse(COPY_PATH, &copy_article());
    for variant in &article.variants {
        let returns = variant.returns.parsed().expect("возврат известен варианту");
        assert_eq!(returns.type_text, "ТаблицаЗначений");
        assert_eq!(
            returns.types[0].ty,
            TypeName::Context("objects/catalog234/catalog236/ValueTable".into())
        );
    }
}

#[test]
fn a_method_article_carries_description_availability_example_and_version() {
    let article = parse(COPY_PATH, &copy_article());
    assert_eq!(
        article.description,
        Field::Parsed("Создаёт копию таблицы значений.".to_string())
    );
    assert_eq!(
        article.availability,
        Field::Parsed(vec![
            Context::Server,
            Context::ThickClient,
            Context::ExternalConnection,
            Context::MobileAppServer,
        ])
    );
    assert_eq!(
        article.example,
        Field::Parsed("НовТаблица = Цены.Скопировать();".to_string())
    );
    assert_eq!(article.since_version, Field::Parsed("8.1".to_string()));
}

#[test]
fn an_optional_parameter_keeps_its_default_apart_from_its_description() {
    let html = page(&format!(
        "<h1 class=\"V8SH_pagetitle\">КоллекцияСтрокДереваЗначений.НайтиСтроки (ValueTreeRowCollection.FindRows)</h1>\
         <p class=\"V8SH_title\">КоллекцияСтрокДереваЗначений (ValueTreeRowCollection)</p>\
         <p class=\"V8SH_heading\">НайтиСтроки (FindRows)</p>\
         {}НайтиСтроки(&lt;Отбор&gt;, &lt;ВключатьПодчиненные&gt;)\
         {}{}Тип: {STRUCTURE}. <br>Условия поиска.\
         {}Тип: {BOOLEAN}. <br>Искать ли в подчинённых коллекциях.<br>Значение по умолчанию: {FALSE}.\
         {}Тип: {ARRAY}. <br>Найденные строки.\
         {}Ищет строки по условию.\
         {}Полезен для неуникальных значений.",
        chapter("Синтаксис:"),
        chapter("Параметры:"),
        rubric("Отбор", false),
        rubric("ВключатьПодчиненные", true),
        chapter("Возвращаемое значение:"),
        chapter("Описание:"),
        chapter("Примечание:"),
    ));
    let article = parse(
        "objects/catalog234/catalog264/ValueTreeRowCollection/methods/FindRows679.html",
        &html,
    );

    let params = article.variants[0]
        .parameters
        .parsed()
        .expect("параметры разобраны");
    assert!(!params[0].optional && params[1].optional);
    assert_eq!(params[1].default.as_deref(), Some("Ложь"));
    // Строка умолчания разобрана в своё поле и не остаётся в описании.
    assert_eq!(params[1].description, "Искать ли в подчинённых коллекциях.");
    let returns = article.variants[0]
        .returns
        .parsed()
        .expect("возврат разобран");
    assert_eq!(returns.description, "Найденные строки.");
    assert_eq!(
        article.note,
        Field::Parsed("Полезен для неуникальных значений.".to_string())
    );
}

#[test]
fn a_read_only_property_takes_its_type_from_the_description() {
    let html = page(&format!(
        "<h1 class=\"V8SH_pagetitle\">ОбъектМетаданныхКонфигурация.ХранилищаНастроек (ConfigurationMetadataObject.SettingsStorages)</h1>\
         <p class=\"V8SH_title\">ОбъектМетаданныхКонфигурация (ConfigurationMetadataObject)</p>\
         <p class=\"V8SH_heading\">ХранилищаНастроек (SettingsStorages)</p>\
         {}Только чтение.\
         {}Тип: {STRING}. <br>Коллекция хранилищ настроек конфигурации.\
         {}<p>Сервер, толстый клиент, внешнее соединение.</p>\
         {}<p class=\"V8SH_versionInfo\">Доступен, начиная с версии 8.2.</p>",
        chapter("Использование:"),
        chapter("Описание:"),
        chapter("Доступность: "),
        chapter("Использование в версии:"),
    ));
    let article = parse(
        "objects/catalog63/catalog272/ConfigurationMetadataObject/properties/SettingsStorages9585.html",
        &html,
    );

    assert_eq!(article.kind, ArticleKind::Property);
    assert_eq!(article.read_only, Field::Parsed(true));
    assert_eq!(article.value_type_text, "Строка");
    assert_eq!(
        article.value_types.parsed().expect("тип свойства разобран")[0].ty,
        TypeName::Primitive("String".into())
    );
    // Тип стоит первой строкой «Описания» и в описание не попадает.
    assert_eq!(
        article.description,
        Field::Parsed("Коллекция хранилищ настроек конфигурации.".to_string())
    );
    assert_eq!(article.since_version, Field::Parsed("8.2".to_string()));
    // У свойства нет вариантов синтаксиса: раздела в статье нет.
    assert!(article.variants.is_empty());
}

#[test]
fn a_type_article_lists_its_members_with_both_names() {
    let html = page(&format!(
        "<h1 class=\"V8SH_pagetitle\">ТаблицаЗначений (ValueTable)</h1>\
         <p class=\"V8SH_title\">ТаблицаЗначений (ValueTable)</p>\
         {}<a href=\"v8help://SyntaxHelperContext/objects/catalog234/catalog236/ValueTableRow.html\">СтрокаТаблицыЗначений</a><br>Обход доступен.<br>\
         {}<a href=\"ValueTable/properties/Columns1030.html\">Колонки (Columns)</a><br>\
         {}<a href=\"ValueTable/methods/Add110.html\">Добавить (Add)</a><br>\
         {}<p>Сервер. Сериализуется. Имя типа XDTO: ValueTable.</p>\
         {}{}",
        chapter("Элементы коллекции:"),
        chapter("Свойства:"),
        chapter("Методы:"),
        chapter("Доступность: "),
        chapter("Пример:"),
        sample("<font color=\"#008000\">//&nbsp;Создание<BR></font><BR>Таблица&nbsp;<font color=\"#ff0000\">=</font><font color=\"#ff0000\">&nbsp;Новый&nbsp;</font>ТаблицаЗначений<font color=\"#ff0000\">;</font>"),
    ));
    let article = parse("objects/catalog234/catalog236/ValueTable.html", &html);

    assert_eq!(article.kind, ArticleKind::Type);
    assert_eq!(article.owner, None);
    assert_eq!(
        article
            .collection_element
            .parsed()
            .expect("элемент разобран")[0]
            .ty,
        TypeName::Context("objects/catalog234/catalog236/ValueTableRow".into())
    );
    let titles = article
        .member_lists
        .iter()
        .map(|list| list.title.as_str())
        .collect::<Vec<_>>();
    assert_eq!(titles, vec!["Свойства", "Методы"]);
    assert_eq!(
        article.member_lists[1].members[0],
        ArticleLink {
            label: "Добавить (Add)".to_string(),
            path: "objects/catalog234/catalog236/ValueTable/methods/Add110".to_string(),
        }
    );
    // Весь текст «Доступности» сохранён: у типа там же сериализация и XDTO.
    assert!(article
        .availability_text
        .contains("Имя типа XDTO: ValueTable"));
    assert_eq!(article.availability, Field::Parsed(vec![Context::Server]));
    // Пустая строка после комментария — та же, что показывает Конфигуратор.
    assert_eq!(
        article.example,
        Field::Parsed("// Создание\n\nТаблица = Новый ТаблицаЗначений;".to_string())
    );
}

/// Три состояния раздела: раздела нет, раздел разобран, раздел есть и не
/// разобран. Пустое и неразобранное не совпадают: иначе «параметров нет»
/// стало бы неотличимо от «параметры не разобрались».
#[test]
fn a_section_that_is_present_but_unparsed_is_not_an_empty_one() {
    let html = page(&format!(
        "<p class=\"V8SH_heading\">Сделать (Do)</p>\
         {}Сделать(&lt;А&gt;){}Первый параметр описан прозой, без рубрики.",
        chapter("Синтаксис:"),
        chapter("Параметры:"),
    ));
    let article = parse("objects/c/Sample/methods/Do1.html", &html);
    let Field::Unparsed { reason, text } = &article.variants[0].parameters else {
        panic!(
            "раздел без рубрик не разобран, а не пуст: {:?}",
            article.variants[0].parameters
        );
    };
    assert_eq!(*reason, UnparsedReason::NoParameterRubric);
    assert!(
        text.contains("без рубрики"),
        "сырой текст раздела сохранён: {text}"
    );
    // Разделов, которых в статье нет, разбор не выдумывает.
    assert!(article.example.is_empty());
    assert!(article.note.is_empty());
    assert!(article.availability.is_empty());

    let without_parameters = page(&format!(
        "<p class=\"V8SH_heading\">Сделать (Do)</p>{}Сделать()",
        chapter("Синтаксис:"),
    ));
    let without_parameters = parse("objects/c/Sample/methods/Do1.html", &without_parameters);
    assert!(without_parameters.variants[0].parameters.is_empty());
}

#[test]
fn a_page_that_is_neither_a_type_nor_a_member_is_still_an_article() {
    let html = page(&format!(
        "<h1 class=\"V8SH_pagetitle\">Работа с таблицей значений</h1>\
         {}Таблица значений живёт в памяти.\
         {}Этот раздел разбор не типизирует.",
        chapter("Описание:"),
        chapter("Особенности платформы:"),
    ));
    let article = parse("guides/ValueTables.html", &html);

    assert_eq!(article.kind, ArticleKind::Concept);
    assert_eq!(article.id, "guides/ValueTables");
    assert_eq!(article.name.ru, "Работа с таблицей значений");
    assert_eq!(
        article.description,
        Field::Parsed("Таблица значений живёт в памяти.".to_string())
    );
    // Незнакомый раздел не теряется молча.
    assert_eq!(
        article.other_sections,
        vec![(
            "Особенности платформы".to_string(),
            "Этот раздел разбор не типизирует.".to_string()
        )]
    );
}

/// Отдельного раздела «См. также» у статей платформы нет: связанные статьи
/// названы ссылками внутри текста, и ссылка несёт путь, по которому статья
/// открывается.
#[test]
fn links_inside_the_text_address_other_articles() {
    let article = parse(COPY_PATH, &copy_article());
    let paths = article
        .links
        .iter()
        .map(|link| link.path.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        paths,
        vec![
            "objects/catalog234/Array",
            "objects/catalog234/Structure",
            "objects/catalog234/catalog236/ValueTable",
        ],
        "ссылка на тип языка страницей не адресуется и в список не идёт"
    );
    assert_eq!(article.links[0].label, "Массив");
}

/// Адрес ссылки в справке бывает без кавычек: он кончается скобкой тега,
/// а не пробелом. Иначе такая ссылка теряется, а при пробеле дальше в путь
/// уходит разметка до него.
#[test]
fn an_unquoted_link_target_ends_at_the_tag() {
    let html = page(&format!(
        "<p class=\"V8SH_title\">Пример (Sample)</p>\
         {}<a href=Sample/methods/Add110.html>Добавить (Add)</a><br>\
         <a href=Sample/methods/Delete2.html target=_blank>Удалить (Delete)</a><br>",
        chapter("Методы:"),
    ));
    let article = parse("objects/catalog9/Sample.html", &html);
    assert_eq!(
        article.member_lists[0]
            .members
            .iter()
            .map(|link| link.path.as_str())
            .collect::<Vec<_>>(),
        vec![
            "objects/catalog9/Sample/methods/Add110",
            "objects/catalog9/Sample/methods/Delete2",
        ]
    );
    assert_eq!(article.member_lists[0].members[0].label, "Добавить (Add)");
}

/// Вызова не бывает у свойства: раздел вызова на его странице —
/// неожиданность, и она уходит в прочие разделы. Молча статья его не теряет
/// и варианта, которому у свойства не место, не заводит.
#[test]
fn a_call_section_on_a_property_page_stays_among_the_other_sections() {
    let html = page(&format!(
        "<p class=\"V8SH_heading\">Колонки (Columns)</p>\
         {}Только чтение.{}Параметры описаны прозой.",
        chapter("Использование:"),
        chapter("Параметры:"),
    ));
    let article = parse("objects/c/Sample/properties/Columns2.html", &html);
    assert!(article.variants.is_empty());
    assert_eq!(
        article.other_sections,
        vec![(
            "Параметры".to_string(),
            "Параметры описаны прозой.".to_string()
        )]
    );
}
