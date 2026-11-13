use super::*;
use unica_documentation::model::ArticleField;

/// Разметка той же формы, что у справки установленной платформы: классы
/// `V8SH_*`, BOM, пробел в конце заголовка раздела. Тексты свои —
/// материалы вендора в репозиторий не копируются.
const METHOD: &str = "\u{feff}<html><body>\
    <h1 class=\"V8SH_pagetitle\">ТаблицаЗначений.Скопировать (ValueTable.Copy)</h1>\
    <p class=\"V8SH_title\">ТаблицаЗначений (ValueTable)</p>\
    <p class=\"V8SH_heading\">Скопировать (Copy)</p>\
    <p class=\"V8SH_chapter\">Синтаксис:</p>Скопировать(&lt;Строки&gt;)\
    <p class=\"V8SH_chapter\">Параметры:</p>\
    <div class=\"V8SH_rubric\"> <p style=\"m\">&lt;Строки&gt; (необязательный)</div>\
    Тип: <a href=\"v8help://SyntaxHelperContext/objects/catalog234/Array.html\">Массив</a>. <br>Строки для копирования.\
    <p class=\"V8SH_chapter\">Возвращаемое значение:</p>\
    Тип: <a href=\"v8help://SyntaxHelperContext/objects/catalog234/catalog236/ValueTable.html\">ТаблицаЗначений</a>. <br>\
    <p class=\"V8SH_chapter\">Описание:</p><p>Создаёт копию.</p>\
    <p class=\"V8SH_chapter\">Доступность: </p><p>Сервер, толстый клиент, новое место.</p>\
    <p class=\"V8SH_chapter\">Использование в версии:</p>\
    <p class=\"V8SH_versionInfo\">Доступен, начиная с версии 8.1.</p>\
    </body></html>";

fn parsed() -> unica_documentation::model::HelpArticle {
    public_article(
        "syntax-context",
        &source::parse(
            "objects/catalog234/catalog236/ValueTable/methods/Copy649.html",
            METHOD,
        ),
    )
}

/// Путь страницы становится локатором своего корпуса: по ссылке статьи
/// потребитель открывает следующую статью, не собирая локатор сам.
/// rule: INV.APP.DOCUMENTATION-ARTICLE-FIELDS
#[test]
fn every_reference_arrives_as_a_locator_of_the_same_corpus() {
    let article = parsed();
    assert_eq!(
        article.owner_document_id.as_deref(),
        Some("platform-syntax-help:syntax-context:objects/catalog234/catalog236/ValueTable.html")
    );
    let ArticleField::Parsed(variants) = &article.variants else {
        panic!("вариант разобран: {:?}", article.variants);
    };
    let ArticleField::Parsed(parameters) = &variants[0].parameters else {
        panic!("параметры разобраны");
    };
    assert_eq!(
        parameters[0].types[0].document_id.as_deref(),
        Some("platform-syntax-help:syntax-context:objects/catalog234/Array.html")
    );
    // Имя типа — то, которым его назвала справка, а не отрезок пути.
    assert_eq!(parameters[0].types[0].text, "Массив");
    assert_eq!(
        article.links[0].document_id,
        "platform-syntax-help:syntax-context:objects/catalog234/Array.html"
    );
}

/// Места исполнения приходят значениями, а не текстом; место, которого
/// справка ещё не называла, приходит как написано, а не теряется.
/// rule: INV.APP.DOCUMENTATION-ARTICLE-FIELDS
#[test]
fn availability_arrives_as_values_and_keeps_an_unknown_place() {
    let article = parsed();
    let ArticleField::Parsed(places) = &article.availability else {
        panic!("доступность разобрана: {:?}", article.availability);
    };
    assert_eq!(
        places,
        &vec![
            "server".to_string(),
            "thick-client".to_string(),
            "новое место".to_string()
        ]
    );
    // Весь текст раздела остаётся рядом со значениями.
    assert!(article.availability_text.contains("новое место"));
}

/// rule: INV.APP.DOCUMENTATION-ARTICLE-FIELDS
#[test]
fn the_kind_owner_and_version_of_the_article_reach_the_public_model() {
    let article = parsed();
    assert_eq!(article.kind.as_str(), "method");
    assert_eq!(article.name.ru, "Скопировать");
    assert_eq!(article.name.en.as_deref(), Some("Copy"));
    assert_eq!(
        article.owner.as_ref().expect("владелец назван").ru,
        "ТаблицаЗначений"
    );
    assert_eq!(
        article.since_version,
        ArticleField::Parsed("8.1".to_string())
    );
    assert_eq!(
        article.description,
        ArticleField::Parsed("Создаёт копию.".to_string())
    );
    // Разделов, которых в статье нет, разбор не выдумывает.
    assert_eq!(article.example, ArticleField::Empty);
    assert_eq!(article.note, ArticleField::Empty);
}

/// Причина, по которой раздел не разобран, доходит до публичной модели
/// вместе с сырым текстом: потребитель покажет исходное.
/// rule: INV.APP.DOCUMENTATION-ARTICLE-STATES
#[test]
fn an_unparsed_section_keeps_its_reason_and_text() {
    let html = "<html><body><p class=\"V8SH_heading\">Сделать (Do)</p>\
        <p class=\"V8SH_chapter\">Параметры:</p>Параметр описан прозой.</body></html>";
    let article = public_article(
        "syntax-context",
        &source::parse("objects/c/Sample/methods/Do1.html", html),
    );
    let ArticleField::Parsed(variants) = &article.variants else {
        panic!("вариант заведён разделом параметров");
    };
    let ArticleField::Unparsed { reason, text } = &variants[0].parameters else {
        panic!(
            "раздел без рубрик не разобран: {:?}",
            variants[0].parameters
        );
    };
    assert_eq!(reason, "no-parameter-rubric");
    assert!(text.contains("прозой"));
}
