//! Статья справки платформы в публичном виде `unica.docs`.
//!
//! Разбор страницы живёт в `unica-bsl-types` и ничего не знает о локаторах
//! документов: он называет статьи путями страниц. Здесь путь становится
//! локатором своего корпуса, по которому статья открывается, а места
//! исполнения — устойчивыми значениями вместо текста.

use std::collections::BTreeMap;

use unica_bsl_types::article as source;
use unica_documentation::model as public;

/// Локатор документа по пути страницы в этом корпусе.
fn document_id(corpus: &str, path: &str) -> String {
    format!("platform-syntax-help:{corpus}:{path}.html")
}

/// Место исполнения устойчивым значением. Место, которого справка ещё не
/// называла, приходит как написано под пометкой `other:`: выдумывать для
/// него имя нельзя, молчать о нём — значит потерять ограничение, а подать
/// его наравне с известными — значит выдать неразобранный текст за
/// значение, которое потребитель вправе сравнивать. Двоеточия в известных
/// значениях нет, поэтому пометка их ни с чем не смешивает.
fn context(value: &unica_bsl_types::Context) -> String {
    use unica_bsl_types::Context::*;
    match value {
        ThinClient => "thin-client".to_string(),
        WebClient => "web-client".to_string(),
        MobileClient => "mobile-client".to_string(),
        Server => "server".to_string(),
        ThickClient => "thick-client".to_string(),
        ExternalConnection => "external-connection".to_string(),
        MobileAppClient => "mobile-app-client".to_string(),
        MobileAppServer => "mobile-app-server".to_string(),
        MobileStandaloneServer => "mobile-standalone-server".to_string(),
        Integration => "integration".to_string(),
        Other(written) => format!("other:{written}"),
    }
}

/// Тип, названный статьёй: текст и документ, если тип описан страницей.
/// Тип языка (`Строка`) своей страницы в этом корпусе не имеет.
///
/// Имя типа берётся из подписи ссылки, которой справка его назвала
/// («Массив»), а не из последнего отрезка пути («Array»): путь у статьи
/// один на обе локали, а потребитель показывает имя человеку.
fn type_mention(
    corpus: &str,
    labels: &BTreeMap<&str, &str>,
    reference: &unica_bsl_types::TypeRef,
) -> public::TypeMention {
    match &reference.ty {
        unica_bsl_types::TypeName::Context(path) => public::TypeMention {
            text: labels
                .get(path.as_str())
                .map(|label| (*label).to_string())
                .unwrap_or_else(|| path.rsplit('/').next().unwrap_or(path).to_string()),
            document_id: Some(document_id(corpus, path)),
        },
        unica_bsl_types::TypeName::Primitive(name) => public::TypeMention {
            text: name.clone(),
            document_id: None,
        },
        unica_bsl_types::TypeName::Unresolved(written) => public::TypeMention {
            text: written.clone(),
            document_id: None,
        },
        // Прикладной тип конфигурации страницы в справке платформы
        // не имеет: его описывает конфигурация, а не вендор.
        unica_bsl_types::TypeName::Applied(applied) => public::TypeMention {
            text: applied.name.clone(),
            document_id: None,
        },
    }
}

fn kind(value: source::ArticleKind) -> public::ArticleKind {
    match value {
        source::ArticleKind::Type => public::ArticleKind::Type,
        source::ArticleKind::Method => public::ArticleKind::Method,
        source::ArticleKind::Property => public::ArticleKind::Property,
        source::ArticleKind::Event => public::ArticleKind::Event,
        source::ArticleKind::Constructor => public::ArticleKind::Constructor,
        source::ArticleKind::Concept => public::ArticleKind::Concept,
    }
}

fn name(value: &source::ArticleName) -> public::LocalizedName {
    public::LocalizedName {
        ru: value.ru.clone(),
        en: value.en.clone(),
    }
}

fn link(corpus: &str, value: &source::ArticleLink) -> public::ArticleLink {
    public::ArticleLink {
        label: value.label.clone(),
        document_id: document_id(corpus, &value.path),
    }
}

/// Поле статьи с сохранением состояния: пусто, разобрано или не разобрано
/// с причиной и сырым текстом.
fn field<T, U>(value: &source::Field<T>, convert: impl FnOnce(&T) -> U) -> public::ArticleField<U> {
    match value {
        source::Field::Empty => public::ArticleField::Empty,
        source::Field::Parsed(value) => public::ArticleField::Parsed(convert(value)),
        source::Field::Unparsed { reason, text } => public::ArticleField::Unparsed {
            reason: reason.as_str().to_string(),
            text: text.clone(),
        },
    }
}

fn types(
    corpus: &str,
    labels: &BTreeMap<&str, &str>,
    list: &[unica_bsl_types::TypeRef],
) -> Vec<public::TypeMention> {
    list.iter()
        .map(|reference| type_mention(corpus, labels, reference))
        .collect()
}

fn variant(
    corpus: &str,
    labels: &BTreeMap<&str, &str>,
    value: &source::SyntaxVariant,
) -> public::ArticleVariant {
    public::ArticleVariant {
        name: value.name.clone(),
        call: field(&value.call, Clone::clone),
        parameters: field(&value.parameters, |parameters| {
            parameters
                .iter()
                .map(|parameter| public::ArticleParameter {
                    name: parameter.name.clone(),
                    optional: parameter.optional,
                    types: types(corpus, labels, &parameter.types),
                    type_text: parameter.type_text.clone(),
                    default: parameter.default.clone(),
                    description: parameter.description.clone(),
                })
                .collect()
        }),
        description: field(&value.description, Clone::clone),
        returns: field(&value.returns, |returns| public::ArticleReturn {
            types: types(corpus, labels, &returns.types),
            type_text: returns.type_text.clone(),
            description: returns.description.clone(),
        }),
    }
}

/// Разобранная статья в публичном виде. `corpus` — корпус локатора,
/// которым открыта сама статья: её ссылки ведут в тот же корпус.
pub fn public_article(corpus: &str, value: &source::Article) -> public::HelpArticle {
    // Подписи ссылок статьи: по ним тип получает имя на языке статьи.
    let labels: BTreeMap<&str, &str> = value
        .links
        .iter()
        .map(|link| (link.path.as_str(), link.label.as_str()))
        .collect();
    public::HelpArticle {
        kind: kind(value.kind),
        name: name(&value.name),
        owner: value.owner.as_ref().map(name),
        owner_document_id: value
            .owner_id
            .as_ref()
            .map(|path| document_id(corpus, path)),
        // Вариантов нет — у статьи нет раздела синтаксиса; это пусто,
        // а не «не разобрано».
        variants: if value.variants.is_empty() {
            public::ArticleField::Empty
        } else {
            public::ArticleField::Parsed(
                value
                    .variants
                    .iter()
                    .map(|item| variant(corpus, &labels, item))
                    .collect(),
            )
        },
        value_types: field(&value.value_types, |list| types(corpus, &labels, list)),
        value_type_text: value.value_type_text.clone(),
        read_only: field(&value.read_only, |flag| *flag),
        availability: field(&value.availability, |places| {
            places.iter().map(context).collect()
        }),
        availability_text: value.availability_text.clone(),
        since_version: field(&value.since_version, Clone::clone),
        description: field(&value.description, Clone::clone),
        note: field(&value.note, Clone::clone),
        example: field(&value.example, Clone::clone),
        collection_element: field(&value.collection_element, |list| {
            types(corpus, &labels, list)
        }),
        member_lists: value
            .member_lists
            .iter()
            .map(|list| public::ArticleMemberList {
                title: list.title.clone(),
                members: list.members.iter().map(|item| link(corpus, item)).collect(),
            })
            .collect(),
        links: value.links.iter().map(|item| link(corpus, item)).collect(),
        other_sections: value.other_sections.clone(),
    }
}

#[cfg(test)]
#[path = "article_fields_tests.rs"]
mod tests;
