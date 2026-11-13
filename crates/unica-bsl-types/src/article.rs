//! Одна статья справки платформы, разобранная по разделам.
//!
//! Статья — HTML с устойчивым устройством: заголовок страницы
//! (`V8SH_pagetitle`), шапка владельца (`V8SH_title`), имя члена
//! (`V8SH_heading`), затем разделы, каждый открытый
//! `<p class="V8SH_chapter">Имя:</p>`. Порядок разделов фиксирован, пустые
//! не выводятся. Типы названы ссылками (`v8help://SyntaxHelperContext/…`,
//! `v8help://SyntaxHelperLanguage/def_String`), поэтому разрешаются
//! страницей, а не именем, и одинаково в любом языке.
//!
//! Разбор отличает три состояния раздела: раздела в статье нет
//! ([`Field::Empty`]), раздел разобран ([`Field::Parsed`]) и раздел есть,
//! но не разобран ([`Field::Unparsed`]) — с причиной и сырым текстом.
//! Четвёртое состояние, «у этого вида статьи такого понятия не бывает»,
//! следует из [`ArticleKind`] и решается потребителем формата: здесь оно
//! неотличимо от пустого, потому что отсутствие раздела выглядит одинаково.

use crate::model::{Context, TypeName, TypeRef};

/// Вид статьи: что она описывает.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArticleKind {
    Type,
    Method,
    Property,
    Event,
    Constructor,
    /// Страница, не описывающая тип платформы или его член: раздел
    /// руководства, понятие, описание механизма.
    Concept,
}

impl ArticleKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ArticleKind::Type => "type",
            ArticleKind::Method => "method",
            ArticleKind::Property => "property",
            ArticleKind::Event => "event",
            ArticleKind::Constructor => "constructor",
            ArticleKind::Concept => "concept",
        }
    }
}

/// Имя статьи: русское и, если справка его называет, английское.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArticleName {
    pub ru: String,
    pub en: Option<String>,
}

/// Почему раздел остался неразобранным. Причина называет, чего в разделе не
/// нашлось, а не что потребителю делать: решение — за ним.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnparsedReason {
    /// В разделе «Параметры» нет ни одной рубрики параметра.
    NoParameterRubric,
    /// В разделе нет строки «Тип: …», которой он типизируется.
    NoTypeLine,
    /// В разделе «Пример» нет блока кода.
    NoCodeBlock,
    /// В разделе «Доступность» не распознано ни одного контекста.
    NoContext,
    /// В разделе «Использование в версии» нет версии появления.
    NoVersion,
    /// Раздел «Использование» говорит не о доступе на чтение и запись.
    UnknownUse,
    /// В разделе «Синтаксис» нет строки вызова.
    NoCallLine,
}

impl UnparsedReason {
    pub fn as_str(self) -> &'static str {
        match self {
            UnparsedReason::NoParameterRubric => "no-parameter-rubric",
            UnparsedReason::NoTypeLine => "no-type-line",
            UnparsedReason::NoCodeBlock => "no-code-block",
            UnparsedReason::NoContext => "no-context",
            UnparsedReason::NoVersion => "no-version",
            UnparsedReason::UnknownUse => "unknown-use",
            UnparsedReason::NoCallLine => "no-call-line",
        }
    }
}

/// Значение раздела статьи.
///
/// `Empty` и `Parsed` различают «раздела в статье нет» и «раздел разобран»;
/// `Unparsed` держит причину и сырой текст раздела, чтобы потребитель мог
/// показать исходное вместо выдуманного. Пустое значение и неразобранный
/// раздел никогда не совпадают: иначе «параметров нет» стало бы неотличимо
/// от «параметры не разобрались».
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Field<T> {
    #[default]
    Empty,
    Parsed(T),
    Unparsed {
        reason: UnparsedReason,
        text: String,
    },
}

impl<T> Field<T> {
    pub fn parsed(&self) -> Option<&T> {
        match self {
            Field::Parsed(value) => Some(value),
            _ => None,
        }
    }

    pub fn is_empty(&self) -> bool {
        matches!(self, Field::Empty)
    }

    /// Разобранное значение либо названная причина: раздел с текстом, из
    /// которого значения не вышло, остаётся неразобранным, а не пустым.
    fn from(value: Option<T>, reason: UnparsedReason, text: &str) -> Self {
        match value {
            Some(value) => Field::Parsed(value),
            None if text.trim().is_empty() => Field::Empty,
            None => Field::Unparsed {
                reason,
                text: text.to_owned(),
            },
        }
    }
}

/// Один параметр варианта синтаксиса.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parameter {
    pub name: String,
    pub optional: bool,
    /// Типы параметра страницами справки: пусто, если тип не назван ссылкой
    /// и не совпал с типом языка.
    pub types: Vec<TypeRef>,
    /// Типы, как их пишет справка: «Массив», «Строка; Число».
    pub type_text: String,
    pub default: Option<String>,
    pub description: String,
}

/// Возвращаемое значение варианта.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReturnValue {
    pub types: Vec<TypeRef>,
    pub type_text: String,
    pub description: String,
}

/// Вариант синтаксиса. У метода с одним вариантом имя варианта отсутствует.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyntaxVariant {
    pub name: Option<String>,
    /// Строка вызова без типов: `Скопировать(<Строки>, <Колонки>)`.
    pub call: Field<String>,
    pub parameters: Field<Vec<Parameter>>,
    /// «Описание варианта метода».
    pub description: Field<String>,
    pub returns: Field<ReturnValue>,
}

/// Ссылка статьи на другую статью: подпись и путь страницы.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArticleLink {
    pub label: String,
    /// Путь страницы без `.html`, как его называет локатор документа.
    pub path: String,
}

/// Список членов на странице типа: «Свойства», «Методы», «Конструкторы»,
/// «События».
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberList {
    pub title: String,
    pub members: Vec<ArticleLink>,
}

/// Статья справки целиком.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Article {
    /// Путь страницы без `.html`.
    pub id: String,
    pub kind: ArticleKind,
    /// Заголовок страницы, как его пишет справка: `Владелец.Член (Owner.Member)`.
    pub title: String,
    /// Владелец члена: тип или коллекция, которой он принадлежит.
    pub owner: Option<ArticleName>,
    /// Путь статьи владельца без `.html`: по нему она открывается.
    pub owner_id: Option<String>,
    pub name: ArticleName,
    pub variants: Vec<SyntaxVariant>,
    /// Тип свойства: справка пишет его первой строкой «Описания».
    pub value_types: Field<Vec<TypeRef>>,
    pub value_type_text: String,
    pub read_only: Field<bool>,
    pub availability: Field<Vec<Context>>,
    /// Весь текст «Доступности»: у типа там же сериализация и тип XDTO.
    pub availability_text: String,
    pub server_call: bool,
    pub since_version: Field<String>,
    pub description: Field<String>,
    pub note: Field<String>,
    pub example: Field<String>,
    pub collection_element: Field<Vec<TypeRef>>,
    pub member_lists: Vec<MemberList>,
    /// Ссылки статьи на другие статьи в порядке появления, без повторов.
    /// Отдельного раздела «См. также» у статей платформы нет: связанные
    /// статьи названы ссылками внутри текста.
    pub links: Vec<ArticleLink>,
    /// Разделы, которым разбор не знает места: заголовок и сырой текст.
    /// Статья не теряет их молча.
    pub other_sections: Vec<(String, String)>,
}

const CHAPTER: &str = "<p class=\"V8SH_chapter\">";
const RUBRIC: &str = "<div class=\"V8SH_rubric\">";
const CODE_SAMPLE: &str = "V8SH_codesample";
const VERSION_INFO: &str = "V8SH_versionInfo";
/// Сколько символов после «Массив из» на странице справки просматривается
/// в поисках ссылки на тип элемента массива.
// resource-limit: bsl-types.array-element-window
// class: format
// unit: count
// scope: record
// value: 200
// resource: Сколько символов после «Массив из» на странице справки просматривается в поисках ссылки на тип элемента массива.
// status: unproven
// origin: #2238
// problem: тип элемента, названный дальше окна, не попадает в каталог
// on-reach: тихо: массив остаётся без типа элемента, то есть с неизвестным, а не с угаданным элементом
const ARRAY_ELEMENT_WINDOW: usize = 200;

/// Вид и владелец страницы по её пути. `None` — страница, не описывающая тип
/// платформы или его член.
pub(crate) fn classify(path: &str) -> Option<(String, ArticleKind, Option<String>)> {
    let id = path.strip_suffix(".html")?;
    if !id.starts_with("objects/") {
        return None;
    }
    let parts = id.split('/').collect::<Vec<_>>();
    // Страница `catalog<N>` — раздел справки, группирующий типы.
    let last = parts.last().copied().unwrap_or_default();
    if last.strip_prefix("catalog").is_some_and(|number| {
        !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit())
    }) {
        return None;
    }
    // `objects/<type>.html` — страница типа. Член лежит в каталоге своего
    // вида под типом, иногда уровнем глубже
    // (`objects/Global context/events/catalog200/OnExit203.html`).
    let member = parts
        .iter()
        .enumerate()
        .skip(1)
        .take(parts.len().saturating_sub(2))
        .rev()
        .find_map(|(index, part)| {
            let kind = match *part {
                "methods" => Some(ArticleKind::Method),
                "properties" => Some(ArticleKind::Property),
                "events" => Some(ArticleKind::Event),
                "ctors" => Some(ArticleKind::Constructor),
                "fields" | "params" | "tables" | "formparams" | "objects" => None,
                _ => return None,
            };
            Some((index, kind))
        });
    match member {
        Some((index, Some(kind))) => Some((id.to_owned(), kind, Some(parts[..index].join("/")))),
        Some((_, None)) => None,
        None => Some((id.to_owned(), ArticleKind::Type, None)),
    }
}

/// Статья по пути страницы и её разметке. Разбирается любая страница:
/// у страницы, не описывающей тип или его член, вид — [`ArticleKind::Concept`],
/// и статья несёт заголовок, текст разделов и ссылки.
pub fn parse(path: &str, html: &str) -> Article {
    let (id, kind, owner_path) = classify(path).unwrap_or_else(|| {
        (
            path.strip_suffix(".html").unwrap_or(path).to_owned(),
            ArticleKind::Concept,
            None,
        )
    });
    // Каталог страницы: относительные ссылки статьи адресуются от него.
    let base = id
        .rsplit_once('/')
        .map_or(String::new(), |(head, _)| head.to_owned());
    let title = element_text(html, "V8SH_pagetitle").unwrap_or_default();
    let owner_heading = paragraph(html, "V8SH_title");
    let heading = match kind {
        ArticleKind::Type => owner_heading.clone(),
        ArticleKind::Concept => paragraph(html, "V8SH_heading")
            .or_else(|| owner_heading.clone())
            .or_else(|| Some(title.clone())),
        _ => paragraph(html, "V8SH_heading"),
    };
    // Имя члена берётся только из его шапки: страница без шапки записи
    // не даёт, и заголовок страницы её не подменяет.
    let name = heading.as_deref().map(split_name).unwrap_or(ArticleName {
        ru: String::new(),
        en: None,
    });
    // Шапка владельца у страницы типа — имя самого типа, а не владельца.
    let owner = match kind {
        ArticleKind::Type | ArticleKind::Concept => None,
        _ => owner_heading.as_deref().map(split_name),
    };
    let mut article = Article {
        id,
        kind,
        title,
        owner,
        owner_id: owner_path,
        name,
        variants: Vec::new(),
        value_types: Field::Empty,
        value_type_text: String::new(),
        read_only: Field::Empty,
        availability: Field::Empty,
        availability_text: String::new(),
        server_call: false,
        since_version: Field::Empty,
        description: Field::Empty,
        note: Field::Empty,
        example: Field::Empty,
        collection_element: Field::Empty,
        member_lists: Vec::new(),
        links: Vec::new(),
        other_sections: Vec::new(),
    };

    // Вариант синтаксиса открывает свою сигнатуру; параметры и возвращаемое
    // значение принадлежат последней открытой.
    let mut syntax_seen = false;
    for (raw_title, content) in chapters(html) {
        let title = raw_title.trim().trim_end_matches(':').trim().to_owned();
        if let Some(variant) = title
            .strip_prefix("Вариант синтаксиса:")
            .or_else(|| title.strip_prefix("Syntax variant:"))
        {
            article.variants.push(SyntaxVariant {
                name: Some(variant.trim().to_owned()),
                ..SyntaxVariant::default()
            });
            syntax_seen = false;
            continue;
        }
        match title.as_str() {
            "Синтаксис" | "Syntax" => {
                // Второй «Синтаксис» без имени варианта — отдельная сигнатура.
                if syntax_seen || article.variants.is_empty() {
                    article.variants.push(SyntaxVariant::default());
                }
                syntax_seen = true;
                let call = text(content);
                current(&mut article.variants).call = Field::from(
                    Some(call.clone()).filter(|c| !c.is_empty()),
                    UnparsedReason::NoCallLine,
                    content,
                );
            }
            "Параметры" | "Parameters" => {
                let parsed = parameters(content);
                current(&mut article.variants).parameters = Field::from(
                    parsed.filter(|list| !list.is_empty()),
                    UnparsedReason::NoParameterRubric,
                    content,
                );
            }
            "Возвращаемое значение" | "Returned value" => {
                current(&mut article.variants).returns =
                    Field::from(return_value(content), UnparsedReason::NoTypeLine, content);
            }
            "Описание варианта метода" | "Description of the method variant" =>
            {
                current(&mut article.variants).description = Field::Parsed(text(content));
            }
            "Описание" | "Description" => {
                let body = text(content);
                if kind == ArticleKind::Property {
                    let types = type_list(content);
                    article.value_type_text = type_text(content);
                    article.value_types = Field::from(
                        Some(types).filter(|list| !list.is_empty()),
                        UnparsedReason::NoTypeLine,
                        content,
                    );
                    // Тип свойства стоит первой строкой «Описания»; описанием
                    // остаётся текст после неё.
                    article.description = Field::Parsed(without_type_line(&body));
                } else {
                    article.description = Field::Parsed(body);
                }
            }
            "Использование" | "Use" => {
                let body = text(content);
                let read_only = if body.contains("Только чтение") || body.contains("Read only")
                {
                    Some(true)
                } else if body.contains("Чтение и запись") || body.contains("Read and write")
                {
                    Some(false)
                } else {
                    None
                };
                article.read_only = Field::from(read_only, UnparsedReason::UnknownUse, content);
            }
            "Доступность" | "Availability" => {
                let body = text(content);
                article.availability_text = body.clone();
                // Сначала места, за первой точкой — примечания.
                let places = contexts(body.split('.').next().unwrap_or_default());
                article.availability = Field::from(
                    Some(places).filter(|list| !list.is_empty()),
                    UnparsedReason::NoContext,
                    content,
                );
                let lower = body.to_lowercase();
                article.server_call =
                    lower.contains("обращение к серверу") || lower.contains("server call");
            }
            "Примечание" | "Note" => {
                article.note = Field::Parsed(text(content));
            }
            "Пример" | "Example" => {
                article.example =
                    Field::from(code_sample(content), UnparsedReason::NoCodeBlock, content);
            }
            "Использование в версии" | "Use in version" => {
                article.since_version =
                    Field::from(since_version(content), UnparsedReason::NoVersion, content);
            }
            "Элементы коллекции" | "Collection items" => {
                let element = links(content)
                    .into_iter()
                    .map(|ty| TypeRef { ty, element: None })
                    .collect::<Vec<_>>();
                article.collection_element = Field::from(
                    Some(element).filter(|list| !list.is_empty()),
                    UnparsedReason::NoTypeLine,
                    content,
                );
            }
            "Свойства"
            | "Properties"
            | "Методы"
            | "Methods"
            | "Конструкторы"
            | "Constructors"
            | "События"
            | "Events" => {
                article.member_lists.push(MemberList {
                    title: title.clone(),
                    members: article_links(content, &base),
                });
            }
            _ => {
                let body = text(content);
                if !title.is_empty() && !body.is_empty() {
                    article.other_sections.push((title.clone(), body));
                }
            }
        }
    }

    if article.since_version.is_empty() {
        // Версия появления стоит и отдельным абзацем, до разделов.
        article.since_version = Field::from(
            element_text(html, VERSION_INFO)
                .as_deref()
                .and_then(version_number),
            UnparsedReason::NoVersion,
            "",
        );
    }
    // Возвращаемое значение, названное один раз на несколько вариантов,
    // принадлежит методу целиком: справка пишет его после всех вариантов,
    // и Конфигуратор показывает его для каждого.
    spread_single_return(&mut article.variants);
    article.links = article_links(html, &base);
    article
}

fn current(variants: &mut Vec<SyntaxVariant>) -> &mut SyntaxVariant {
    if variants.is_empty() {
        variants.push(SyntaxVariant::default());
    }
    variants.last_mut().expect("сигнатура только что заведена")
}

/// Единственное возвращаемое значение достаётся всем вариантам: иначе
/// вариант, у которого раздела не было, выглядел бы ничего не возвращающим.
fn spread_single_return(variants: &mut [SyntaxVariant]) {
    if variants.len() < 2 {
        return;
    }
    let mut stated = variants
        .iter()
        .filter(|variant| !variant.returns.is_empty());
    let (Some(single), None) = (stated.next(), stated.next()) else {
        return;
    };
    let single = single.returns.clone();
    for variant in variants.iter_mut() {
        variant.returns = single.clone();
    }
}

/// Текст первого элемента с этим классом: заголовок страницы или абзац
/// версии. Содержимое заголовков вложенных тегов не несёт.
fn element_text(html: &str, class: &str) -> Option<String> {
    let needle = format!("class=\"{class}\"");
    let at = html.find(&needle)?;
    let start = at + html[at..].find('>')? + 1;
    let end = html[start..].find("</")? + start;
    let value = text(&html[start..end]);
    (!value.is_empty()).then_some(value)
}

/// Текст первого `<p class="{class}">`.
fn paragraph(html: &str, class: &str) -> Option<String> {
    let open = format!("<p class=\"{class}\">");
    let start = html.find(&open)? + open.len();
    let end = html[start..].find("</p>")? + start;
    let value = text(&html[start..end]);
    (!value.is_empty()).then_some(value)
}

/// `Запрос (Query)` как `ArticleName { ru: "Запрос", en: Some("Query") }`.
fn split_name(heading: &str) -> ArticleName {
    let heading = heading.trim();
    match heading
        .strip_suffix(')')
        .and_then(|rest| rest.rsplit_once(" ("))
    {
        Some((ru, en)) => ArticleName {
            ru: ru.trim().to_owned(),
            en: Some(en.trim().to_owned()),
        },
        None => ArticleName {
            ru: heading.to_owned(),
            en: None,
        },
    }
}

/// Каждый раздел: заголовок и разметка до следующего раздела.
fn chapters(html: &str) -> Vec<(String, &str)> {
    let mut sections = Vec::new();
    let mut rest = html;
    while let Some(start) = rest.find(CHAPTER) {
        let after = &rest[start + CHAPTER.len()..];
        let Some(close) = after.find("</p>") else {
            break;
        };
        let title = text(&after[..close]);
        let body = &after[close + "</p>".len()..];
        let end = body.find(CHAPTER).unwrap_or(body.len());
        sections.push((title, &body[..end]));
        rest = &body[end..];
    }
    sections
}

fn parameters(content: &str) -> Option<Vec<Parameter>> {
    let parsed = content
        .split(RUBRIC)
        .skip(1)
        .filter_map(|rubric| {
            let (head, rest) = rubric.split_once("</div>").unwrap_or((rubric, ""));
            let head = text(head);
            let name = head
                .split_once('<')
                .and_then(|(_, rest)| rest.split_once('>'))
                .map(|(name, _)| name.trim().to_owned())?;
            let optional = head.contains("необязательный") || head.contains("optional");
            let body = text(rest);
            let default = body
                .split_once("Значение по умолчанию:")
                .or_else(|| body.split_once("Default value:"))
                .map(|(_, value)| value.trim().trim_end_matches('.').trim().to_owned())
                .filter(|value| !value.is_empty());
            let description = without_default_line(&without_type_line(&body));
            Some(Parameter {
                name,
                optional,
                types: type_list(rest),
                type_text: type_text(rest),
                default,
                description,
            })
        })
        .collect::<Vec<_>>();
    (!parsed.is_empty()).then_some(parsed)
}

fn return_value(content: &str) -> Option<ReturnValue> {
    let types = type_list(content);
    if types.is_empty() {
        return None;
    }
    Some(ReturnValue {
        types,
        type_text: type_text(content),
        description: without_type_line(&text(content)),
    })
}

/// Текст без строки «Тип: …»: она уже разобрана в типы.
fn without_type_line(body: &str) -> String {
    let Some(at) = body.find("Тип:").or_else(|| body.find("Type:")) else {
        return body.trim().to_owned();
    };
    let before = &body[..at];
    let after = &body[at..];
    // Предложение «Тип: X.» может быть всем текстом раздела: тогда описания
    // в нём нет, и остаток пуст.
    let tail = after.find(". ").map_or("", |end| &after[end + 2..]);
    format!("{} {}", before.trim(), tail.trim())
        .trim()
        .to_owned()
}

/// Текст без строки «Значение по умолчанию: …»: она уже разобрана отдельно.
fn without_default_line(body: &str) -> String {
    let Some(at) = body
        .find("Значение по умолчанию:")
        .or_else(|| body.find("Default value:"))
    else {
        return body.trim().to_owned();
    };
    let before = &body[..at];
    let after = &body[at..];
    let tail = after.find(". ").map(|end| &after[end + 2..]).unwrap_or("");
    format!("{} {}", before.trim(), tail.trim())
        .trim()
        .to_owned()
}

/// Типы, как их пишет справка: текст после «Тип:» до конца предложения.
fn type_text(content: &str) -> String {
    let body = text(content);
    let Some(at) = body.find("Тип:").or_else(|| body.find("Type:")) else {
        return String::new();
    };
    let after = &body[at..];
    let after = after.split_once(':').map_or("", |(_, rest)| rest);
    let end = after
        .find(". ")
        .map(|index| index + 1)
        .unwrap_or(after.len());
    after[..end].trim().trim_end_matches('.').trim().to_owned()
}

/// Код примера: блок `V8SH_codesample`, где перевод строки — `<BR>`,
/// а неразрывный пробел — обычный.
fn code_sample(content: &str) -> Option<String> {
    let needle = format!("class=\"{CODE_SAMPLE}\"");
    let at = content.find(&needle)?;
    let start = at + content[at..].find('>')? + 1;
    let end = content[start..]
        .to_ascii_lowercase()
        .find("</p>")
        .map(|index| index + start)
        .unwrap_or(content.len());
    let code = code_text(&content[start..end]);
    (!code.trim().is_empty()).then_some(code)
}

/// Текст блока кода: теги сняты, `<BR>` стал переводом строки.
fn code_text(markup: &str) -> String {
    let mut out = String::with_capacity(markup.len());
    let mut rest = markup;
    loop {
        let Some(open) = rest.find('<') else {
            out.push_str(rest);
            break;
        };
        out.push_str(&rest[..open]);
        let after = &rest[open..];
        let Some(close) = after.find('>') else {
            break;
        };
        let tag = after[1..close].trim();
        if tag.eq_ignore_ascii_case("br") || tag.eq_ignore_ascii_case("br/") {
            out.push('\n');
        }
        rest = &after[close + 1..];
    }
    let decoded = decode(&out);
    decoded
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
        .trim_matches('\n')
        .to_owned()
}

/// Версия появления из «Доступен, начиная с версии 8.1.».
fn since_version(content: &str) -> Option<String> {
    version_number(&text(content))
}

/// Номер версии из «Доступен, начиная с версии 8.1.»: цифры и точки сразу
/// за словом «версии». Дальше в разделе идёт конец страницы — черта и ссылка
/// «Методическая информация», — и требование «весь остаток раздела
/// состоит из цифр» отвергало бы любую настоящую статью.
fn version_number(line: &str) -> Option<String> {
    let at = line
        .find("версии")
        .map(|index| index + "версии".len())
        .or_else(|| line.find("version").map(|index| index + "version".len()))?;
    let value = line[at..]
        .trim_start()
        .chars()
        .take_while(|character| character.is_ascii_digit() || *character == '.')
        .collect::<String>()
        .trim_end_matches('.')
        .to_owned();
    (!value.is_empty() && value.starts_with(|c: char| c.is_ascii_digit())).then_some(value)
}

/// Типы после «Тип:» (`Type:`): ссылки и слова через `,` или `;`. Слово —
/// тип, только когда это точно написанный тип языка (`Строка`); любое другое
/// слово (`Произвольный`) остаётся как написано, нераспознанным. Массив,
/// элемент которого назван в том же предложении (`Массив из элементов типа X`),
/// несёт X.
pub(crate) fn type_list(content: &str) -> Vec<TypeRef> {
    let Some(start) = content.find("Тип:").or_else(|| content.find("Type:")) else {
        return Vec::new();
    };
    let mut rest = content[start..]
        .split_once(':')
        .map_or("", |(_, rest)| rest);
    let mut types = Vec::new();
    loop {
        rest = rest.trim_start_matches([' ', '\n', '\r', '\t']);
        if rest
            .get(..3)
            .is_some_and(|head| head.eq_ignore_ascii_case("<a "))
        {
            let Some(end) = rest.to_ascii_lowercase().find("</a>") else {
                break;
            };
            types.extend(links(&rest[..end + 4]));
            rest = &rest[end + 4..];
        } else {
            let word: String = rest.chars().take_while(|c| c.is_alphanumeric()).collect();
            if word.is_empty() {
                break;
            }
            rest = &rest[word.len()..];
            types.push(primitive(&word).unwrap_or(TypeName::Unresolved(word)));
        }
        rest = rest.trim_start_matches([' ', '\n']);
        match rest.chars().next() {
            Some(',' | ';') => rest = &rest[1..],
            _ => break,
        }
    }
    let element = array_element(content);
    types
        .into_iter()
        .map(|ty| {
            let element = element.clone().filter(|_| {
                matches!(&ty, TypeName::Context(id) if id.ends_with("/Array") || id.ends_with("/FixedArray"))
            });
            TypeRef { ty, element }
        })
        .collect()
}

/// Тип языка, названный справкой в тексте, — так, как назвала бы его ссылка.
fn primitive(word: &str) -> Option<TypeName> {
    let name = match word {
        "Строка" | "String" => "String",
        "Число" | "Number" => "Number",
        "Булево" | "Boolean" => "Boolean",
        "Дата" | "Date" => "Date",
        "Неопределено" | "Undefined" => "Undefined",
        "Null" => "Null",
        _ => return None,
    };
    Some(TypeName::Primitive(name.to_owned()))
}

/// X из `Массив из элементов типа <a>X</a>` / `Array of … <a>X</a>`: ссылка
/// должна стоять в том же предложении, что и фраза.
fn array_element(content: &str) -> Option<TypeName> {
    let at = content
        .find("Массив из")
        .or_else(|| content.find("Array of"))?;
    let window = &content[at..];
    let cut = window
        .char_indices()
        .nth(ARRAY_ELEMENT_WINDOW)
        .map_or(window.len(), |(index, _)| index);
    let window = &window[..cut];
    // Предложение кончается точкой, за которой пробел или тег; точки внутри
    // адреса ссылки не сопровождаются ни тем, ни другим.
    let sentence = window
        .match_indices('.')
        .find(|(index, _)| matches!(window.as_bytes().get(index + 1), Some(b' ' | b'<')))
        .map_or(window, |(index, _)| &window[..index]);
    links(sentence).into_iter().next()
}

/// Типы всех ссылок справки в `markup`.
pub(crate) fn links(markup: &str) -> Vec<TypeName> {
    hrefs(markup)
        .into_iter()
        .filter_map(|(href, _)| {
            if let Some(path) = href.strip_prefix("v8help://SyntaxHelperContext/") {
                Some(TypeName::Context(
                    path.strip_suffix(".html").unwrap_or(path).to_owned(),
                ))
            } else {
                href.strip_prefix("v8help://SyntaxHelperLanguage/def_")
                    .map(|name| TypeName::Primitive(name.to_owned()))
            }
        })
        .collect()
}

/// Ссылки статьи на другие статьи справки: подпись и путь страницы, без
/// повторов и в порядке появления. Ссылка на статью языка (`def_String`)
/// страницей не адресуется и в список не идёт. `base` — каталог самой
/// страницы: относительные ссылки разрешаются от него, иначе путь из списка
/// членов типа не открывает статью.
fn article_links(markup: &str, base: &str) -> Vec<ArticleLink> {
    let mut found: Vec<ArticleLink> = Vec::new();
    for (href, label) in hrefs(markup) {
        let absolute;
        let path = match href.strip_prefix("v8help://SyntaxHelperContext/") {
            Some(path) => path,
            None if href.starts_with("v8help://") || href.contains("://") => continue,
            // Страница типа называет своих членов путём относительно себя.
            None if base.is_empty() => href,
            None => {
                absolute = format!("{base}/{href}");
                absolute.as_str()
            }
        };
        let path = path.strip_suffix(".html").unwrap_or(path).to_owned();
        let label = text(label);
        if label.is_empty() || found.iter().any(|link| link.path == path) {
            continue;
        }
        found.push(ArticleLink { label, path });
    }
    found
}

/// Адрес и разметка подписи каждой ссылки.
fn hrefs(markup: &str) -> Vec<(&str, &str)> {
    let mut found = Vec::new();
    let mut rest = markup;
    // Приведение к нижнему регистру по ASCII сохраняет смещения, поэтому
    // найденное в копии режет оригинал.
    while let Some(at) = rest.to_ascii_lowercase().find("href=") {
        let after = &rest[at + "href=".len()..];
        let quoted = after.starts_with('"');
        let value = if quoted { &after[1..] } else { after };
        // Адрес без кавычек кончается пробелом или самой скобкой тега:
        // `<a href=X.html>` в справке встречается, и поиск одного лишь
        // пробела либо терял такую ссылку, либо уводил в её путь разметку
        // до следующего пробела.
        let end = if quoted {
            value.find('"')
        } else {
            value.find(['>', ' ', '\t', '\r', '\n'])
        };
        let Some(end) = end else {
            break;
        };
        let href = &value[..end];
        let tail = &value[end..];
        let label = tail
            .find('>')
            .map(|open| &tail[open + 1..])
            .and_then(|body| {
                body.to_ascii_lowercase()
                    .find("</a>")
                    .map(|close| &body[..close])
            })
            .unwrap_or("");
        found.push((href, label));
        rest = tail;
    }
    found
}

pub(crate) fn contexts(text: &str) -> Vec<Context> {
    let mut found = text
        .split(',')
        .map(|item| item.trim().trim_end_matches('.').trim().to_lowercase())
        .filter(|item| !item.is_empty())
        .map(|item| match item.as_str() {
            "тонкий клиент" | "thin client" => Context::ThinClient,
            "веб-клиент" | "web client" => Context::WebClient,
            "мобильный клиент" | "mobile client" => Context::MobileClient,
            "сервер" | "server" => Context::Server,
            "толстый клиент" | "thick client" => Context::ThickClient,
            "внешнее соединение" | "external connection" => {
                Context::ExternalConnection
            }
            "мобильное приложение (клиент)" | "mobile application (client)" => {
                Context::MobileAppClient
            }
            "мобильное приложение (сервер)" | "mobile application (server)" => {
                Context::MobileAppServer
            }
            "мобильный автономный сервер" | "mobile standalone server" => {
                Context::MobileStandaloneServer
            }
            "интеграция" | "integration" => Context::Integration,
            _ => Context::Other(item),
        })
        .collect::<Vec<_>>();
    found.sort();
    found.dedup();
    found
}

/// Текст фрагмента разметки: теги сняты, сущности раскрыты.
pub(crate) fn text(markup: &str) -> String {
    let mut out = String::with_capacity(markup.len());
    let mut tag = String::new();
    let mut in_tag = false;
    for character in markup.chars() {
        match character {
            '<' => {
                in_tag = true;
                tag.clear();
            }
            '>' if in_tag => {
                in_tag = false;
                // Перевод строки разделяет предложения: без него соседние
                // строки раздела склеиваются («поиска.Замечание!»). Прочие
                // теги стоят внутри предложения и разделителя не несут:
                // ссылка на тип не должна отрываться от своей точки.
                if tag.trim().eq_ignore_ascii_case("br") || tag.trim().eq_ignore_ascii_case("br/") {
                    out.push(' ');
                }
            }
            _ if in_tag => tag.push(character),
            _ => out.push(character),
        }
    }
    decode(&out)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn decode(raw: &str) -> String {
    raw.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&nbsp;", " ")
        .replace("&#160;", " ")
        .replace("&quot;", "\"")
        .replace("&amp;", "&")
        .trim_start_matches('\u{feff}')
        .to_owned()
}

#[cfg(test)]
#[path = "article_tests.rs"]
mod tests;
