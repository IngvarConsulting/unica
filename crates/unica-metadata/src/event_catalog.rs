//! Immutable checked platform module and managed-form event catalogues.
//!
//! Каталог принадлежит линии платформы; сборка, с которой он снят, записана
//! внутри него как `source.installationVersion`. Линия без своего каталога
//! обслуживается ближайшей младшей описанной линией.
use crate::event::BindingFact;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::OnceLock;
use unica_source_contract::format_identity::PlatformLine;

/// One versioned possible-event record. The registry owns applicability and
/// exact expected shape; module projection only evaluates source evidence
/// against these records.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlatformEventSpec {
    pub event_id: String,
    pub handler_ru: String,
    pub handler_en: String,
    pub signature_ru: String,
    pub signature_en: String,
    pub method_kind: String,
    pub contexts: Vec<String>,
    pub source_page_id: String,
    #[serde(skip, default = "platform_binding")]
    pub binding: BindingFact,
}

const fn platform_binding() -> BindingFact {
    BindingFact::Platform
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlatformEventCatalogSource {
    pub installation_version: String,
    pub container: String,
    pub sha256: String,
    pub english_container: String,
    pub english_sha256: String,
    #[serde(default)]
    pub event_markup_page_count: usize,
    #[serde(default)]
    pub signature_event_leaf_count: usize,
    #[serde(default)]
    pub event_page_ids_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModuleEventCatalog {
    pub owner_kind: String,
    pub module_role: String,
    pub source_owner: Option<String>,
    pub source_path_prefix: Option<String>,
    pub base_contexts: Vec<String>,
    pub events: Vec<PlatformEventSpec>,
    pub exclusion_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FormPlatformEventCatalog {
    pub owner_kinds: Vec<String>,
    pub source_owner: Option<String>,
    #[serde(default)]
    pub inherited_source_owners: Vec<String>,
    #[serde(default)]
    pub event_id_overrides: BTreeMap<String, String>,
    pub base_contexts: Vec<String>,
    #[serde(default)]
    pub metadata_owner_kinds: Vec<String>,
    #[serde(default)]
    pub main_attribute_kinds: Vec<String>,
    #[serde(default)]
    pub main_attribute_type_prefixes: Vec<String>,
    #[serde(default)]
    pub dynamic_list_source: bool,
    pub events: Vec<PlatformEventSpec>,
    pub exclusion_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExcludedEventPage {
    pub page_id: String,
    pub title: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlatformEventCatalogFixture {
    pub profile: String,
    pub source: PlatformEventCatalogSource,
    pub module_catalogs: Vec<ModuleEventCatalog>,
    pub form_catalogs: Vec<FormPlatformEventCatalog>,
    #[serde(default, alias = "excludedUnmatchedPages")]
    pub excluded_structural_pages: Vec<ExcludedEventPage>,
    #[serde(default)]
    pub excluded_external_data_source_pages: Vec<ExcludedEventPage>,
    #[serde(default)]
    pub excluded_generic_template_pages: Vec<ExcludedEventPage>,
    #[serde(default)]
    pub excluded_out_of_profile_pages: Vec<ExcludedEventPage>,
}

/// Закреплённые каталоги по линиям платформы. Ключ — линия; сборка, с которой
/// каталог снят, лежит внутри него и сверяется с установленной отдельно
/// (`scripts/dev/check-platform-catalogs.py`).
const CHECKED_CATALOGS: &[(PlatformLine, &str)] = &[
    (
        PlatformLine::new(8, 3, 27),
        include_str!("platform-event-catalog-8.3.27.json"),
    ),
    (
        PlatformLine::new(8, 5, 1),
        include_str!("platform-event-catalog-8.5.1.json"),
    ),
    (
        PlatformLine::new(8, 5, 4),
        include_str!("platform-event-catalog-8.5.4.json"),
    ),
];

fn checked_catalogs() -> &'static [(PlatformLine, PlatformEventCatalogFixture)] {
    static PARSED: OnceLock<Vec<(PlatformLine, PlatformEventCatalogFixture)>> = OnceLock::new();
    PARSED.get_or_init(|| {
        CHECKED_CATALOGS
            .iter()
            .map(|(line, text)| {
                let fixture = serde_json::from_str(text).unwrap_or_else(|error| {
                    panic!("checked catalog of line {line} is invalid: {error}")
                });
                (*line, fixture)
            })
            .collect()
    })
}

/// Все закреплённые каталоги с их линиями, по возрастанию линии. Проверки
/// идут по этому списку, иначе новый каталог оказался бы непроверенным.
pub fn checked_platform_event_catalogs() -> &'static [(PlatformLine, PlatformEventCatalogFixture)] {
    checked_catalogs()
}

/// Каталог именно этой линии. Подстановки младшей линии нет: каталог есть
/// у каждой описанной линии, а линия без каталога — незакрытая работа, а не
/// повод обслужить набор чужими таблицами. Набор такой линии остаётся без
/// возможных событий, и это видно, а не маскируется чужим составом.
///
/// `None` — линии нет вовсе: образ формы, который не объявил формат выгрузки.
/// Тогда отвечает младший каталог: подставлять нечего, потому что нечего
/// и подменять. Полноту соответствия линий и каталогов держит тест
/// `every_described_line_has_a_checked_catalog_of_its_own`.
pub fn platform_event_catalog_fixture_for_line(
    line: Option<PlatformLine>,
) -> Option<&'static PlatformEventCatalogFixture> {
    let catalogs = checked_catalogs();
    let Some(line) = line else {
        return catalogs.first().map(|(_, fixture)| fixture);
    };
    catalogs
        .iter()
        .find(|(described, _)| *described == line)
        .map(|(_, fixture)| fixture)
}

#[cfg(test)]
mod tests {
    use super::{platform_event_catalog_fixture_for_line, CHECKED_CATALOGS};
    use unica_source_contract::format_identity::{PlatformLine, DESCRIBED_PLATFORM_LINES};

    fn form_event_ids(line: Option<PlatformLine>, owner: &str) -> Vec<&'static str> {
        platform_event_catalog_fixture_for_line(line)
            .expect("the line of this test has a checked catalog")
            .form_catalogs
            .iter()
            .filter(|catalog| catalog.owner_kinds.iter().any(|kind| kind == owner))
            .flat_map(|catalog| catalog.events.iter())
            .map(|event| event.event_id.as_str())
            .collect()
    }

    /// У каждой описанной линии есть свой закреплённый каталог. Новая
    /// описанная линия без каталога падает здесь, а не обслуживается тихо
    /// таблицами другой линии.
    /// rule: INV.PLATFORM.MODULE-EVENT-CATALOG
    #[test]
    fn every_described_line_has_a_checked_catalog_of_its_own() {
        for (line, id) in DESCRIBED_PLATFORM_LINES {
            let catalog = platform_event_catalog_fixture_for_line(Some(*line))
                .unwrap_or_else(|| panic!("line {id} has no checked event catalog"));
            assert_eq!(
                catalog.profile, *id,
                "line {id} was served the catalog of another line"
            );
        }
        assert_eq!(
            CHECKED_CATALOGS.len(),
            DESCRIBED_PLATFORM_LINES.len(),
            "a checked catalog without a described line is nobody's"
        );
    }

    /// Линия, которой не описано, каталога не получает: лучше без событий,
    /// чем с чужими.
    /// rule: INV.PLATFORM.MODULE-EVENT-CATALOG
    #[test]
    fn an_undescribed_line_is_not_served_by_another_lines_catalog() {
        for line in [PlatformLine::new(8, 3, 26), PlatformLine::new(8, 5, 2)] {
            assert!(
                platform_event_catalog_fixture_for_line(Some(line)).is_none(),
                "{line} is not described and must not borrow a catalog"
            );
        }
    }

    /// Выбор каталога идёт по линии набора, а не по одной закреплённой линии:
    /// у обычной группы формы событие появляется только с 8.5.1.
    /// rule: INV.PLATFORM.MODULE-EVENT-CATALOG
    #[test]
    fn the_line_of_the_set_decides_which_catalog_answers() {
        let old = PlatformLine::new(8, 3, 27);
        let new = PlatformLine::new(8, 5, 1);
        assert!(
            !form_event_ids(Some(old), "Element.Group").contains(&"Click"),
            "8.3.27 knows no event of a usual form group"
        );
        assert!(
            form_event_ids(Some(new), "Element.Group").contains(&"Click"),
            "8.5.1 added Click to a usual form group"
        );
        assert_eq!(
            form_event_ids(None, "Element.Group"),
            form_event_ids(Some(old), "Element.Group"),
            "an image without a declared format is answered by the oldest catalog"
        );
    }

    /// Менеджер очереди получает события только с той линии, которая их
    /// завела: вид «Очередь» существует с 8.5.4.
    /// rule: INV.PLATFORM.MODULE-EVENT-CATALOG
    #[test]
    fn the_queue_manager_has_its_events_only_from_the_line_that_added_them() {
        let events = |line: PlatformLine| {
            platform_event_catalog_fixture_for_line(Some(line))
                .expect("described line")
                .module_catalogs
                .iter()
                .filter(|catalog| catalog.owner_kind == "Queue")
                .flat_map(|catalog| catalog.events.iter())
                .map(|event| event.event_id.as_str())
                .collect::<Vec<_>>()
        };
        assert!(events(PlatformLine::new(8, 3, 27)).is_empty());
        assert!(events(PlatformLine::new(8, 5, 1)).is_empty());
        assert_eq!(
            events(PlatformLine::new(8, 5, 4)),
            ["MessageProcessing", "OnProcessingError"]
        );
    }
}
