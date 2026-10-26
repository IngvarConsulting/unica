//! One page of the platform help, read by its sections.
//!
//! A page is HTML with a fixed shape: the name in `V8SH_heading` (members)
//! or `V8SH_title` (types), then sections, each opened by
//! `<p class="V8SH_chapter">Name:</p>`. A type is a link to its page
//! (`v8help://SyntaxHelperContext/objects/.../Query.html`) or to a language
//! type (`v8help://SyntaxHelperLanguage/def_String`), so types resolve by
//! page, not by name, and the same in every language.

use crate::model::{Context, TypeName, TypeRef};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PageKind {
    Type,
    Method,
    Property,
    Event,
    Constructor,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Page {
    /// The page path without `.html`.
    pub(crate) id: String,
    pub(crate) kind: PageKind,
    /// The type page a member page belongs to.
    pub(crate) owner: Option<String>,
    /// The name before the parentheses of the heading.
    pub(crate) name: String,
    /// The name in the parentheses, when the heading has one.
    pub(crate) alias: Option<String>,
    pub(crate) variants: Vec<RawSignature>,
    /// A property's type, from its description.
    pub(crate) types: Vec<TypeRef>,
    pub(crate) read_only: bool,
    pub(crate) contexts: Vec<Context>,
    /// The help notes that a call goes to the server.
    pub(crate) server_call: bool,
    pub(crate) collection_element: Vec<TypeRef>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct RawSignature {
    pub(crate) name: String,
    pub(crate) params: Vec<RawParam>,
    pub(crate) returns: Vec<TypeRef>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RawParam {
    pub(crate) name: String,
    pub(crate) optional: bool,
    pub(crate) types: Vec<TypeRef>,
    pub(crate) default: Option<String>,
}

const CHAPTER: &str = "<p class=\"V8SH_chapter\">";
const RUBRIC: &str = "<div class=\"V8SH_rubric\">";
/// How far after «Массив из» the element type link is looked for, in
/// characters: the phrase and the link stand together in the help.
const ARRAY_ELEMENT_WINDOW: usize = 200;

/// The kind and owner of a page, from its path; `None` for a page that is
/// neither a type nor one of its members.
fn classify(path: &str) -> Option<(String, PageKind, Option<String>)> {
    let id = path.strip_suffix(".html")?;
    if !id.starts_with("objects/") {
        return None;
    }
    let parts = id.split('/').collect::<Vec<_>>();
    // A `catalog<N>` page is a section of the help that groups types.
    let last = parts.last().copied().unwrap_or_default();
    if last.strip_prefix("catalog").is_some_and(|number| {
        !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit())
    }) {
        return None;
    }
    // `objects/<type>.html` is a type page. A member lies in a kind
    // directory under its type, sometimes grouped one level deeper
    // (`objects/Global context/events/catalog200/OnExit203.html`).
    let member = parts
        .iter()
        .enumerate()
        .skip(1)
        .take(parts.len().saturating_sub(2))
        .rev()
        .find_map(|(index, part)| {
            let kind = match *part {
                "methods" => Some(PageKind::Method),
                "properties" => Some(PageKind::Property),
                "events" => Some(PageKind::Event),
                "ctors" => Some(PageKind::Constructor),
                "fields" | "params" | "tables" | "formparams" | "objects" => None,
                _ => return None,
            };
            Some((index, kind))
        });
    match member {
        Some((index, Some(kind))) => Some((id.to_owned(), kind, Some(parts[..index].join("/")))),
        Some((_, None)) => None,
        None => Some((id.to_owned(), PageKind::Type, None)),
    }
}

pub(crate) fn parse(path: &str, html: &str) -> Option<Page> {
    let (id, kind, owner) = classify(path)?;
    let heading = match kind {
        PageKind::Type => paragraph(html, "V8SH_title"),
        _ => paragraph(html, "V8SH_heading"),
    }?;
    let (name, alias) = split_alias(&heading);
    let mut page = Page {
        id,
        kind,
        owner,
        name,
        alias,
        variants: Vec::new(),
        types: Vec::new(),
        read_only: false,
        contexts: Vec::new(),
        server_call: false,
        collection_element: Vec::new(),
    };
    // A syntax variant opens its own signature; parameters and the return
    // value belong to the signature opened last.
    let mut syntax_seen = false;
    for (title, content) in chapters(html) {
        let title = title.trim().trim_end_matches(':').trim();
        if let Some(name) = title
            .strip_prefix("Вариант синтаксиса:")
            .or_else(|| title.strip_prefix("Syntax variant:"))
        {
            page.variants.push(RawSignature {
                name: name.trim().to_owned(),
                ..RawSignature::default()
            });
            syntax_seen = false;
            continue;
        }
        match title {
            "Синтаксис" | "Syntax" => {
                // A second syntax without a variant name is a signature of
                // its own.
                if syntax_seen || page.variants.is_empty() {
                    page.variants.push(RawSignature::default());
                }
                syntax_seen = true;
            }
            "Параметры" | "Parameters" => {
                current(&mut page.variants).params = parameters(content)
            }
            "Возвращаемое значение" | "Returned value" => {
                current(&mut page.variants).returns = type_list(content);
            }
            "Описание" | "Description" if kind == PageKind::Property => {
                page.types = type_list(content);
            }
            "Использование" | "Use" => {
                let text = text(content);
                page.read_only = text.contains("Только чтение") || text.contains("Read only");
            }
            "Доступность" | "Availability" => {
                let text = text(content);
                // The places come first; notes follow after a period.
                page.contexts = contexts(text.split('.').next().unwrap_or_default());
                let lower = text.to_lowercase();
                page.server_call =
                    lower.contains("обращение к серверу") || lower.contains("server call");
            }
            "Элементы коллекции" | "Collection items" => {
                page.collection_element = links(content)
                    .into_iter()
                    .map(|ty| TypeRef { ty, element: None })
                    .collect();
            }
            _ => {}
        }
    }
    Some(page)
}

fn current(variants: &mut Vec<RawSignature>) -> &mut RawSignature {
    if variants.is_empty() {
        variants.push(RawSignature::default());
    }
    variants.last_mut().expect("a signature was just ensured")
}

/// The text of the first `<p class="{class}">`.
fn paragraph(html: &str, class: &str) -> Option<String> {
    let open = format!("<p class=\"{class}\">");
    let start = html.find(&open)? + open.len();
    let end = html[start..].find("</p>")? + start;
    Some(text(&html[start..end]))
}

/// `Запрос (Query)` as `("Запрос", Some("Query"))`.
fn split_alias(heading: &str) -> (String, Option<String>) {
    let heading = heading.trim();
    match heading
        .strip_suffix(')')
        .and_then(|rest| rest.rsplit_once(" ("))
    {
        Some((name, alias)) => (name.trim().to_owned(), Some(alias.trim().to_owned())),
        None => (heading.to_owned(), None),
    }
}

/// Every section: its title and the markup up to the next section.
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

fn parameters(content: &str) -> Vec<RawParam> {
    content
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
            let description = text(rest);
            let default = description
                .split_once("Значение по умолчанию:")
                .or_else(|| description.split_once("Default value:"))
                .map(|(_, value)| value.trim().trim_end_matches('.').trim().to_owned())
                .filter(|value| !value.is_empty());
            Some(RawParam {
                name,
                optional,
                types: type_list(rest),
                default,
            })
        })
        .collect()
}

/// The types after `Тип:` (`Type:`): links and words separated by `,` or
/// `;`. A word is a type only when it is a language type spelled exactly
/// (`Строка`); any other word (`Произвольный`) stays as written, unknown.
/// An array whose element the same sentence names
/// (`Массив из элементов типа X`) carries X.
fn type_list(content: &str) -> Vec<TypeRef> {
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

/// A language type the help names in text, as its link would name it.
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

/// X of `Массив из элементов типа <a>X</a>` / `Array of ... <a>X</a>`: the
/// link must stand in the same sentence as the phrase.
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
    // A sentence ends at a period followed by a space or a tag; periods in
    // a link's address are followed by neither.
    let sentence = window
        .match_indices('.')
        .find(|(index, _)| matches!(window.as_bytes().get(index + 1), Some(b' ' | b'<')))
        .map_or(window, |(index, _)| &window[..index]);
    links(sentence).into_iter().next()
}

/// The types of every help link in `markup`.
fn links(markup: &str) -> Vec<TypeName> {
    let mut found = Vec::new();
    let mut rest = markup;
    // ASCII lower-casing keeps byte offsets, so positions found in the copy
    // cut the original.
    while let Some(at) = rest.to_ascii_lowercase().find("href=\"") {
        let after = &rest[at + 6..];
        let Some(end) = after.find('"') else {
            break;
        };
        let href = &after[..end];
        if let Some(path) = href.strip_prefix("v8help://SyntaxHelperContext/") {
            found.push(TypeName::Context(
                path.strip_suffix(".html").unwrap_or(path).to_owned(),
            ));
        } else if let Some(name) = href.strip_prefix("v8help://SyntaxHelperLanguage/def_") {
            found.push(TypeName::Primitive(name.to_owned()));
        }
        rest = &after[end..];
    }
    found
}

fn contexts(text: &str) -> Vec<Context> {
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

/// The text of a markup fragment: tags dropped, entities decoded.
pub(crate) fn text(markup: &str) -> String {
    let mut out = String::with_capacity(markup.len());
    let mut in_tag = false;
    for character in markup.chars() {
        match character {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => out.push(character),
            _ => {}
        }
    }
    out.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&nbsp;", " ")
        .replace("&#160;", " ")
        .replace("&quot;", "\"")
        .replace("&amp;", "&")
        .trim_start_matches('\u{feff}')
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
#[path = "page_tests.rs"]
mod tests;
