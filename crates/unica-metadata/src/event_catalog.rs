//! Immutable checked platform module and managed-form event catalogue.
use crate::event::BindingFact;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::OnceLock;

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

pub fn platform_event_catalog_fixture() -> &'static PlatformEventCatalogFixture {
    static FIXTURE: OnceLock<PlatformEventCatalogFixture> = OnceLock::new();
    FIXTURE.get_or_init(|| {
        serde_json::from_str(include_str!("platform-event-catalog-8.3.27.2074.json"))
            .expect("the checked 8.3.27.2074 platform event catalog must be valid")
    })
}
