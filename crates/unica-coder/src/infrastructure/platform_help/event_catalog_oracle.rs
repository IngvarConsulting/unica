//! Deterministic provenance oracle for the compact 8.3.27 event catalog.
//!
//! The runtime projection reads only the checked fixture. This test enumerates
//! every `/events/` page of the installed vendor Syntax Assistant and proves
//! that the compact fixture is an exact derivation for the approved v0.13
//! logical owner/module profile. Vendor HBK content is never copied here.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::infrastructure::native_operations::form_event_registry::{
    ExcludedEventPage, PlatformEventCatalogFixture, PlatformEventSpec,
};
use crate::infrastructure::platform_help::corpus::{read_corpus, CorpusPage};

const FIXTURE: &str = include_str!("../platform-event-catalog-8.3.27.2074.json");
const FIXTURE_PATH: &str = "src/infrastructure/platform-event-catalog-8.3.27.2074.json";

fn installed_root() -> Option<PathBuf> {
    std::env::var("UNICA_PLATFORM_HELP_DIR")
        .ok()
        .map(PathBuf::from)
}

fn event_owner(page: &CorpusPage) -> Option<&str> {
    let (_, english) = page.title.rsplit_once(" (")?;
    let english = english.strip_suffix(')')?;
    english.rsplit_once('.').map(|(owner, _)| owner)
}

fn event_id(page: &CorpusPage) -> Result<String, String> {
    if let Some((_, english)) = page.title.rsplit_once(" (") {
        if let Some(english) = english.strip_suffix(')') {
            if let Some((_, event)) = english.rsplit_once('.') {
                if !event.is_empty() {
                    return Ok(event.to_string());
                }
            }
        }
    }
    let path = &page.path;
    let file = path
        .rsplit('/')
        .next()
        .ok_or_else(|| format!("event page has no file name: {path}"))?;
    let stem = file
        .strip_suffix(".html")
        .or_else(|| file.strip_suffix(".htm"))
        .unwrap_or(file);
    let id = stem.trim_end_matches(|ch: char| ch.is_ascii_digit());
    if id.is_empty() {
        Err(format!("event page has no stable event id: {path}"))
    } else {
        Ok(id.to_string())
    }
}

fn declaration(raw: &str) -> String {
    raw.split("<br>")
        .next()
        .unwrap_or(raw)
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
        .trim()
        .to_string()
}

fn declaration_kind(signature: &str) -> Result<&'static str, String> {
    let head = signature.split_whitespace().next().unwrap_or_default();
    match head.to_lowercase().as_str() {
        "процедура" | "procedure" => Ok("procedure"),
        "функция" | "function" => Ok("function"),
        _ => Err(format!("event template has no method kind: {signature}")),
    }
}

fn declaration_name(signature: &str) -> Result<String, String> {
    let after_kind = signature
        .split_once(char::is_whitespace)
        .map(|(_, tail)| tail.trim_start())
        .ok_or_else(|| format!("event template has no handler: {signature}"))?;
    let name = after_kind
        .split_once('(')
        .map(|(name, _)| name.trim())
        .unwrap_or(after_kind);
    if name.is_empty() {
        Err(format!("event template has an empty handler: {signature}"))
    } else {
        Ok(name.to_string())
    }
}

fn availability_contexts(page: &CorpusPage, base: &[String], handler_ru: &str) -> Vec<String> {
    if page.text.contains("Примечание: Вызывается на сервере.")
        || handler_ru.to_lowercase().contains("насервере")
    {
        return base
            .iter()
            .filter(|context| context.as_str() == "server")
            .cloned()
            .collect();
    }
    let availability = page
        .text
        .rsplit_once("Доступность:")
        .map(|(_, tail)| tail.split('.').next().unwrap_or(tail))
        .unwrap_or_default()
        .to_lowercase();
    let mut candidates = BTreeSet::new();
    for (needle, contexts) in [
        ("тонкий клиент", &["thinClient"][..]),
        ("веб-клиент", &["webClient"][..]),
        ("мобильный клиент", &["mobileClient"][..]),
        ("сервер", &["server"][..]),
        (
            "толстый клиент",
            &["thickClientManaged", "thickClientOrdinary"][..],
        ),
        ("внешнее соединение", &["externalConnection"][..]),
        ("мобильное приложение (клиент)", &["mobileAppClient"][..]),
        ("мобильное приложение (сервер)", &["mobileAppServer"][..]),
        (
            "мобильный автономный сервер",
            &["mobileStandaloneServer"][..],
        ),
    ] {
        if availability.contains(needle) {
            candidates.extend(contexts.iter().copied());
        }
    }
    base.iter()
        .filter(|context| candidates.contains(context.as_str()))
        .cloned()
        .collect()
}

fn derive_event(
    page: &CorpusPage,
    base_contexts: &[String],
    form_callback: bool,
) -> Result<PlatformEventSpec, String> {
    let signature = page
        .signature
        .as_ref()
        .ok_or_else(|| format!("event leaf has no bilingual template: {}", page.path))?;
    let signature_ru = declaration(
        signature
            .ru
            .as_deref()
            .ok_or_else(|| format!("event leaf has no Russian template: {}", page.path))?,
    );
    let signature_en = declaration(
        signature
            .en
            .as_deref()
            .ok_or_else(|| format!("event leaf has no English template: {}", page.path))?,
    );
    let method_kind = declaration_kind(&signature_ru)?.to_string();
    if declaration_kind(&signature_en)? != method_kind {
        return Err(format!(
            "bilingual method kinds disagree on {}: {signature_ru} / {signature_en}",
            page.path
        ));
    }
    let handler_ru = declaration_name(&signature_ru)?;
    let handler_en = declaration_name(&signature_en)?;
    let mut contexts = availability_contexts(page, base_contexts, &handler_ru);
    if form_callback
        && !handler_ru.to_lowercase().contains("насервере")
        && !page.text.contains("Примечание: Вызывается на сервере.")
    {
        contexts.retain(|context| {
            matches!(
                context.as_str(),
                "thinClient"
                    | "webClient"
                    | "thickClientManaged"
                    | "mobileClient"
                    | "mobileAppClient"
            )
        });
    }
    if contexts.is_empty() {
        return Err(format!(
            "event leaf has no proven effective context for its catalog: {} | {}",
            page.path, page.title
        ));
    }
    Ok(PlatformEventSpec {
        event_id: event_id(page)?,
        handler_ru,
        handler_en,
        signature_ru,
        signature_en,
        method_kind,
        contexts,
        source_page_id: format!("platform-syntax-help:syntax-context:{}", page.path),
        binding: crate::domain::module_projection::BindingFact::Platform,
    })
}

fn selected_pages<'a>(
    pages: &'a [CorpusPage],
    source_owner: &str,
    source_path_prefix: Option<&str>,
) -> Vec<&'a CorpusPage> {
    let mut selected = pages
        .iter()
        .filter(|page| page.path.contains("/events/") && page.signature.is_some())
        .filter(|page| {
            source_path_prefix
                .map(|prefix| page.path.starts_with(prefix))
                .unwrap_or_else(|| event_owner(page) == Some(source_owner))
        })
        .collect::<Vec<_>>();
    selected.sort_by_key(|page| event_id(page).unwrap_or_default());
    selected
}

fn unmatched_exclusions(pages: &[CorpusPage]) -> Vec<ExcludedEventPage> {
    let mut excluded = pages
        .iter()
        .filter(|page| page.path.contains("/events/") && event_owner(page).is_none())
        .map(|page| {
            let reason = if page.signature.is_none() {
                "structural event-catalog index; not an event leaf"
            } else {
                "external-data-source managed-form owner is outside the approved v0.13 address/profile matrix"
            };
            ExcludedEventPage {
                page_id: format!("platform-syntax-help:syntax-context:{}", page.path),
                title: page.title.clone(),
                reason: reason.to_string(),
            }
        })
        .collect::<Vec<_>>();
    excluded.sort_by(|left, right| left.page_id.cmp(&right.page_id));
    excluded
}

fn generic_template_exclusions(pages: &[CorpusPage]) -> Vec<ExcludedEventPage> {
    let owners = [
        "HTTP-service module",
        "Web service module",
        "Integration service module",
        "Extension for controls located in a form",
    ];
    let mut excluded = pages
        .iter()
        .filter(|page| page.path.contains("/events/") && page.signature.is_some())
        .filter(|page| event_owner(page).is_some_and(|owner| owners.contains(&owner)))
        .map(|page| ExcludedEventPage {
            page_id: format!("platform-syntax-help:syntax-context:{}", page.path),
            title: page.title.clone(),
            reason: if event_owner(page) == Some("Extension for controls located in a form") {
                "generic <Event name> callback template; no closed named event ID is proven"
                    .to_string()
            } else {
                "generic declarative handler template; binding remains on its exact declarative owner"
                    .to_string()
            },
        })
        .collect::<Vec<_>>();
    excluded.sort_by(|left, right| left.page_id.cmp(&right.page_id));
    excluded
}

fn derive_fixture(root: &Path) -> Result<PlatformEventCatalogFixture, String> {
    let mut fixture: PlatformEventCatalogFixture =
        serde_json::from_str(FIXTURE).map_err(|error| error.to_string())?;
    let container = root.join(&fixture.source.container);
    let bytes = std::fs::read(&container).map_err(|error| error.to_string())?;
    let digest = Sha256::digest(&bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    if digest != fixture.source.sha256 {
        return Err(format!(
            "Syntax Assistant digest differs for {}: expected {}, actual {digest}",
            container.display(),
            fixture.source.sha256
        ));
    }
    let pages = read_corpus(&bytes).map_err(|error| format!("{error:?}"))?;
    for catalog in &mut fixture.module_catalogs {
        let Some(source_owner) = catalog.source_owner.as_deref() else {
            catalog.events.clear();
            continue;
        };
        let selected = selected_pages(&pages, source_owner, catalog.source_path_prefix.as_deref());
        if selected.is_empty() {
            return Err(format!(
                "module catalog {} × {} resolved no vendor event pages for {source_owner}",
                catalog.owner_kind, catalog.module_role
            ));
        }
        catalog.events = selected
            .into_iter()
            .map(|page| derive_event(page, &catalog.base_contexts, false))
            .collect::<Result<Vec<_>, _>>()?;
    }
    for catalog in &mut fixture.form_catalogs {
        let Some(source_owner) = catalog.source_owner.as_deref() else {
            catalog.events.clear();
            continue;
        };
        let mut selected = selected_pages(&pages, source_owner, None);
        for inherited in &catalog.inherited_source_owners {
            selected.extend(selected_pages(&pages, inherited, None));
        }
        selected.sort_by_key(|page| event_id(page).unwrap_or_default());
        if selected.is_empty() {
            return Err(format!(
                "form catalog {:?} resolved no vendor event pages for {source_owner}",
                catalog.owner_kinds
            ));
        }
        catalog.events = selected
            .into_iter()
            .map(|page| derive_event(page, &catalog.base_contexts, true))
            .collect::<Result<Vec<_>, _>>()?;
        for event in &mut catalog.events {
            if let Some(canonical) = catalog.event_id_overrides.get(&event.event_id) {
                event.event_id.clone_from(canonical);
            }
        }
    }
    fixture.excluded_unmatched_pages = unmatched_exclusions(&pages);
    fixture.excluded_generic_template_pages = generic_template_exclusions(&pages);
    Ok(fixture)
}

#[test]
fn checked_platform_event_catalog_matches_complete_8_3_27_vendor_oracle() {
    let Some(root) = installed_root() else {
        eprintln!("UNICA_PLATFORM_HELP_DIR is not set; platform event oracle skipped");
        return;
    };
    let derived = derive_fixture(&root).expect("derive complete event fixture from vendor corpus");
    let checked: PlatformEventCatalogFixture =
        serde_json::from_str(FIXTURE).expect("checked event fixture");
    assert_eq!(
        derived, checked,
        "checked platform event catalog differs from the complete vendor derivation"
    );
}

#[test]
#[ignore = "writes the mechanically derived compact fixture when explicitly requested"]
fn regenerate_checked_platform_event_catalog() {
    assert_eq!(
        std::env::var("UNICA_UPDATE_PLATFORM_EVENT_CATALOG").as_deref(),
        Ok("1"),
        "set UNICA_UPDATE_PLATFORM_EVENT_CATALOG=1 explicitly"
    );
    let root = installed_root().expect("UNICA_PLATFORM_HELP_DIR");
    let derived = derive_fixture(&root).expect("derive complete event fixture from vendor corpus");
    let text = format!(
        "{}\n",
        serde_json::to_string_pretty(&derived).expect("serialize derived event fixture")
    );
    std::fs::write(
        Path::new(env!("CARGO_MANIFEST_DIR")).join(FIXTURE_PATH),
        text,
    )
    .expect("write derived event fixture");
}
