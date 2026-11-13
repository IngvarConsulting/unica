//! Страница справки платформы в том виде, в каком её читает каталог типов.
//!
//! Сам разбор страницы по разделам живёт в [`crate::article`]: он общий для
//! каталога типов и для справки по полям, которую отдаёт `unica.docs`.
//! Здесь — только то, что каталогу нужно от статьи: виды страниц, сигнатуры
//! и типы. Расхождения между двумя разборами быть не может, потому что
//! разбор один.

use crate::article::{self, Article, ArticleKind, Field};
use crate::model::{Context, TypeRef};

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

#[cfg(test)]
pub(crate) use article::type_list;

/// The page of a type or of one of its members; `None` for any other page.
pub(crate) fn parse(path: &str, html: &str) -> Option<Page> {
    let (_, kind, _) = article::classify(path)?;
    let kind = match kind {
        ArticleKind::Type => PageKind::Type,
        ArticleKind::Method => PageKind::Method,
        ArticleKind::Property => PageKind::Property,
        ArticleKind::Event => PageKind::Event,
        ArticleKind::Constructor => PageKind::Constructor,
        // `classify` не называет страницу членом или типом только вместе
        // с `None`, который уже отсеян выше.
        ArticleKind::Concept => return None,
    };
    let article = article::parse(path, html);
    // Шапка нужна каталогу: без имени члена страница не даёт записи.
    if article.name.ru.is_empty() {
        return None;
    }
    Some(from_article(article, kind))
}

fn from_article(article: Article, kind: PageKind) -> Page {
    Page {
        kind,
        variants: article
            .variants
            .iter()
            .map(|variant| RawSignature {
                name: variant.name.clone().unwrap_or_default(),
                params: variant
                    .parameters
                    .parsed()
                    .map(|params| {
                        params
                            .iter()
                            .map(|param| RawParam {
                                name: param.name.clone(),
                                optional: param.optional,
                                types: param.types.clone(),
                                default: param.default.clone(),
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
                returns: variant
                    .returns
                    .parsed()
                    .map(|value| value.types.clone())
                    .unwrap_or_default(),
            })
            .collect(),
        types: article.value_types.parsed().cloned().unwrap_or_default(),
        read_only: matches!(article.read_only, Field::Parsed(true)),
        contexts: article.availability.parsed().cloned().unwrap_or_default(),
        server_call: article.server_call,
        collection_element: article
            .collection_element
            .parsed()
            .cloned()
            .unwrap_or_default(),
        owner: article.owner_id,
        name: article.name.ru,
        alias: article.name.en,
        id: article.id,
    }
}

#[cfg(test)]
#[path = "page_tests.rs"]
mod tests;
