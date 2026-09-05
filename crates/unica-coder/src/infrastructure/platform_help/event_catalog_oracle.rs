//! Deterministic provenance oracle for the compact 8.3.27 event catalog.
//!
//! The runtime projection reads only the checked fixture. This test enumerates
//! every `/events/` page of the installed vendor Syntax Assistant and proves
//! that the compact fixture is an exact derivation for the approved v0.13
//! logical owner/module profile. Vendor HBK content is never copied here.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::infrastructure::native_operations::form_event_registry::{
    ExcludedEventPage, FormPlatformEventCatalog, PlatformEventCatalogFixture, PlatformEventSpec,
};
use crate::infrastructure::platform_help::corpus::{read_corpus, CorpusPage};
use crate::infrastructure::platform_help::installation::discover;

const FIXTURE: &str = include_str!("../platform-event-catalog-8.3.27.2074.json");
const FIXTURE_PATH: &str = "src/infrastructure/platform-event-catalog-8.3.27.2074.json";

fn installed_root() -> Option<PathBuf> {
    std::env::var("UNICA_PLATFORM_HELP_DIR")
        .ok()
        .map(PathBuf::from)
}

fn source_page_path(source_page_id: &str) -> &str {
    source_page_id
        .strip_prefix("platform-syntax-help:syntax-context:")
        .unwrap_or(source_page_id)
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

fn exact_english_template(
    page: &CorpusPage,
    english_page: Option<&CorpusPage>,
    raw: &str,
) -> Result<String, String> {
    let parsed = declaration(raw);
    if parsed != "," {
        return Ok(parsed);
    }
    // These two pinned 8.3.27.2074 `.st` records contain a literal comma in
    // the English slot. The adjacent English pages still name the handler and
    // parameter semantics; freeze the exact bilingual declaration here so a
    // generic fallback can never turn another damaged page into invented API.
    let (expected_ru, english_syntax, canonical) = match page.path.as_str() {
        "objects/catalog1649/catalog1679/catalog2421/Form table extension for dynamic list/events/URLGetProcessing713.html" => (
            "Процедура ОбработкаПолученияНавигационнойСсылки(Элемент, Ключ, Представление, НавигационнаяСсылка, СтандартнаяОбработка)<br>    <?><br>КонецПроцедуры",
            "Syntax: URLGetProcessing(",
            "Procedure URLGetProcessing(Item, Key, Presentation, URL, StandardProcessing)",
        ),
        "objects/catalog1649/catalog1679/catalog2421/Form table extension for dynamic list/events/URLListGetProcessing712.html" => (
            "Процедура ОбработкаПолученияСпискаНавигационныхСсылок(Элемент, СписокНавигационыхСсылок, КлючПоУмолчанию)<br>    <?><br>КонецПроцедуры",
            "Syntax: URLListGetProcessing(",
            "Procedure URLListGetProcessing(Item, URLList, DefaultKey)",
        ),
        _ => return Ok(parsed),
    };
    let raw_ru = page
        .signature
        .as_ref()
        .and_then(|signature| signature.ru.as_deref());
    let english_page = english_page
        .ok_or_else(|| format!("missing adjacent English evidence page for {}", page.path))?;
    if raw_ru != Some(expected_ru)
        || english_page
            .signature
            .as_ref()
            .and_then(|signature| signature.en.as_deref())
            != Some(",")
        || !english_page.text.contains(english_syntax)
    {
        return Err(format!(
            "pinned bilingual exception evidence changed for {}",
            page.path
        ));
    }
    Ok(canonical.to_string())
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
    english_page: Option<&CorpusPage>,
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
    let signature_en = exact_english_template(
        page,
        english_page,
        signature
            .en
            .as_deref()
            .ok_or_else(|| format!("event leaf has no English template: {}", page.path))?,
    )?;
    let method_kind = declaration_kind(&signature_ru)
        .map_err(|error| format!("{} on {} from {:?}", error, page.path, signature.ru))?
        .to_string();
    if declaration_kind(&signature_en)
        .map_err(|error| format!("{} on {} from {:?}", error, page.path, signature.en))?
        != method_kind
    {
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

const APPROVED_EVENT_OWNERS: &[&str] = &[
    "AccountingRegisterManager.&lt;Accounting register name&gt;",
    "AccountingRegisterRecordSet.&lt;Accounting register name&gt;",
    "AccumulationRegisterManager.&lt;Accumulation register name&gt;",
    "AccumulationRegisterRecordSet.&lt;Accumulation register name&gt;",
    "Bot module",
    "BusinessProcessManager.&lt;Business process name&gt;",
    "BusinessProcessObject.&lt;Business process name&gt;",
    "CalculationRegisterManager.&lt;Calculation register name&gt;",
    "CalculationRegisterRecordSet.&lt;Calculation register name&gt;",
    "CatalogManager.&lt;Catalog name&gt;",
    "CatalogObject.&lt;Catalog name&gt;",
    "ChartOfAccountsManager.&lt;Chart of accounts name&gt;",
    "ChartOfAccountsObject.&lt;Chart of accounts name&gt;",
    "ChartOfCalculationTypesManager.&lt;Chart of calculation types name&gt;",
    "ChartOfCalculationTypesObject.&lt;Chart of calculation types name&gt;",
    "ChartOfCharacteristicTypesManager.&lt;Chart of characteristic types name&gt;",
    "ChartOfCharacteristicTypesObject.&lt;Chart of characteristic types name&gt;",
    "CheckBox",
    "ClientApplicationForm",
    "Client application form extension for business processes",
    "Client application form extension for catalogs",
    "Client application form extension for chart of characteristics types",
    "Client application form extension for constants",
    "Client application form extension for documents",
    "Client application form extension for information register records",
    "Client application form extension for objects",
    "Client application form extension for reports",
    "Command module",
    "ConstantManager.&lt;Constant Name&gt;",
    "ConstantValueManager.&lt;Constant name&gt;",
    "DataProcessorManager.&lt;Data processor name&gt;",
    "DataProcessorObject.&lt;Data processor name&gt;",
    "DocumentJournalManager.&lt;Document journal name&gt;",
    "DocumentManager.&lt;Document name&gt;",
    "DocumentObject.&lt;Document name&gt;",
    "EnumManager.&lt;Enumeration name&gt;",
    "ExchangePlanManager.&lt;Exchange plan name&gt;",
    "ExchangePlanObject.&lt;Exchange plan name&gt;",
    "ExternalDataProcessor",
    "ExternalReport",
    "FilterCriterionManager.&lt;Criterion name&gt;",
    "Form decoration extension for a label",
    "Form decoration extension for a picture",
    "Form extension for a HTML document field",
    "Form field extension for a calendar field",
    "Form field extension for a formatted document",
    "Form field extension for a graphical schema field",
    "Form field extension for a label field",
    "Form field extension for a picture field",
    "Form field extension for a spreadsheet document field",
    "Form field extension for a text box",
    "Form field extension for a text document",
    "Form group extension for pages",
    "Form table extension for dynamic list",
    "FormField",
    "FormTable",
    "Global context",
    "InformationRegisterManager.&lt;Information register name&gt;",
    "InformationRegisterRecordSet.&lt;Information register name&gt;",
    "Managed form extension for record set",
    "Managed form extension for tasks",
    "RadioButton",
    "ReportManager.&lt;Report name&gt;",
    "ReportObject.&lt;Report name&gt;",
    "SettingsStorageManager.&lt;Storage name&gt;",
    "TaskManager.&lt;Task name&gt;",
    "TaskObject.&lt;Task name&gt;",
    "TrackBar",
    "WebSocket client module",
];

fn is_approved_event_page(page: &CorpusPage) -> bool {
    page.path.contains("/events/")
        && page.signature.is_some()
        && event_owner(page).is_some_and(|owner| APPROVED_EVENT_OWNERS.contains(&owner))
}

fn excluded_page(page: &CorpusPage, reason: &str) -> ExcludedEventPage {
    ExcludedEventPage {
        page_id: format!("platform-syntax-help:syntax-context:{}", page.path),
        title: page.title.clone(),
        reason: reason.to_string(),
    }
}

fn structural_exclusions(pages: &[CorpusPage]) -> Vec<ExcludedEventPage> {
    let mut excluded = pages
        .iter()
        .filter(|page| page.path.contains("/events/") && page.signature.is_none())
        .map(|page| excluded_page(page, "structural event-catalog index; not an event leaf"))
        .collect::<Vec<_>>();
    excluded.sort_by(|left, right| left.page_id.cmp(&right.page_id));
    excluded
}

fn external_data_source_exclusions(pages: &[CorpusPage]) -> Vec<ExcludedEventPage> {
    let prefix = "objects/catalog1649/catalog1890/Client application form extension for external data source table";
    let mut excluded = pages
        .iter()
        .filter(|page| page.path.contains("/events/") && page.path.starts_with(prefix))
        .map(|page| excluded_page(page, "external-data-source managed-form owner is outside the approved v0.13 address/profile matrix"))
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

const ORDINARY_FORM_OWNERS: &[&str] = &[
    "Accounting register record set form extension",
    "Accumulation register record set form extension",
    "Business process list table box extension",
    "Business process object form extension",
    "Button",
    "Calculation register record set form extension",
    "Calculation type form extension",
    "Calculation types list table box extension",
    "CalendarBox",
    "Catalog item form extension",
    "Catalog list table box extension",
    "Characteristic type item form extension",
    "Characteristic types list table box extension",
    "Chart",
    "Chart of accounts item form extension",
    "Chart of accounts list table box extension",
    "ComboBox",
    "Constant form extension",
    "Data processor form extension",
    "Dendrogram",
    "Document form extension",
    "Document journal table box extension",
    "Document list table box extension",
    "Extension for controls located in a spreadsheet document field",
    "Extension for controls located in a table box",
    "Form",
    "GanttChart",
    "GeographicalSchemaField",
    "GraphicalSchemaField",
    "HTMLDocumentField",
    "Information register record form extension",
    "Information register record set form extension",
    "Label",
    "ListBox",
    "Node form extension",
    "Node list table box extension",
    "Panel",
    "PictureBox",
    "PivotChart",
    "Report form extension",
    "SpreadsheetDocumentField",
    "TableBox",
    "Task form extension",
    "Task list table box extension",
    "TextBox",
    "Value tree table box extension",
];

const UNSUPPORTED_MANAGED_FORM_ELEMENT_OWNERS: &[&str] = &[
    "Form extension for a PDF document field",
    "Form field extension for a Gantt chart field",
    "Form field extension for a chart field",
    "Form field extension for a dendrogram field",
    "Form field extension for a geographical schema field",
    "Form field extension for a period field",
    "Form field extension for a planner",
];

const EXTERNAL_DATA_SOURCE_MODULE_OWNERS: &[&str] = &[
    "ExternalDataSourceCubeDimensionTableManager.&lt;External source name&gt;.&lt;Cube name&gt;.&lt;Dimension table name&gt;",
    "ExternalDataSourceCubeManager.&lt;External source name&gt;.&lt;Cube name&gt;",
    "ExternalDataSourceTableManager.&lt;External source name&gt;.&lt;External data source table name&gt;",
    "ExternalDataSourceTableObject.&lt;External source name&gt;.&lt;External data source table name&gt;",
    "ExternalDataSourceTableRecordSet.&lt;External source name&gt;.&lt;External data source table name&gt;",
];

const NON_MODULE_EVENT_OWNERS: &[&str] = &[
    "BusinessProcessRoutePointRef.&lt;Business process name&gt;",
    "RecalculationRecordSet.&lt;Recalculation name&gt;",
    "SequenceRecordSet.&lt;Sequence name&gt;",
];

fn reviewed_out_of_profile_reason(page: &CorpusPage) -> Option<&'static str> {
    let owner = event_owner(page)?;
    if ORDINARY_FORM_OWNERS.contains(&owner) {
        Some("out-of-profile/ordinary-form: ordinary-form runtime owners are outside the approved managed-form projection")
    } else if UNSUPPORTED_MANAGED_FORM_ELEMENT_OWNERS.contains(&owner) {
        Some("out-of-profile/managed-form-element: the element kind is absent from the approved v0.13 form owner taxonomy")
    } else if EXTERNAL_DATA_SOURCE_MODULE_OWNERS.contains(&owner) {
        Some("out-of-profile/external-data-source-module: ExternalDataSource has no approved v0.13 address/profile owner")
    } else if NON_MODULE_EVENT_OWNERS.contains(&owner) {
        Some("out-of-profile/non-module-owner: the platform event owner is not an approved module or managed-form owner")
    } else if owner == "Client application form extension for settings composer" {
        Some("out-of-profile/settings-composer: no approved source-derived form applicability fact identifies this owner")
    } else {
        None
    }
}

fn out_of_profile_exclusions(pages: &[CorpusPage]) -> Result<Vec<ExcludedEventPage>, String> {
    let structural = structural_exclusions(pages)
        .into_iter()
        .map(|page| source_page_path(&page.page_id).to_string())
        .collect::<BTreeSet<_>>();
    let external = external_data_source_exclusions(pages)
        .into_iter()
        .map(|page| source_page_path(&page.page_id).to_string())
        .collect::<BTreeSet<_>>();
    let generic = generic_template_exclusions(pages)
        .into_iter()
        .map(|page| source_page_path(&page.page_id).to_string())
        .collect::<BTreeSet<_>>();
    let mut excluded = Vec::new();
    for page in pages
        .iter()
        .filter(|page| page.path.contains("/events/"))
        .filter(|page| !is_approved_event_page(page))
        .filter(|page| !structural.contains(&page.path))
        .filter(|page| !external.contains(&page.path))
        .filter(|page| !generic.contains(&page.path))
    {
        let reason = reviewed_out_of_profile_reason(page).ok_or_else(|| {
            format!(
                "unreviewed out-of-profile event owner on {} | {}",
                page.path, page.title
            )
        })?;
        excluded.push(excluded_page(page, reason));
    }
    excluded.sort_by(|left, right| left.page_id.cmp(&right.page_id));
    Ok(excluded)
}

fn supplemental_form_catalog(
    source_owner: &str,
    owner_kind: &str,
    metadata_owner_kinds: &[&str],
    main_attribute_kinds: &[&str],
    main_attribute_type_prefixes: &[&str],
    dynamic_list_source: bool,
) -> FormPlatformEventCatalog {
    FormPlatformEventCatalog {
        owner_kinds: vec![owner_kind.to_string()],
        source_owner: Some(source_owner.to_string()),
        inherited_source_owners: Vec::new(),
        event_id_overrides: BTreeMap::new(),
        base_contexts: vec![
            "thinClient".to_string(),
            "webClient".to_string(),
            "thickClientManaged".to_string(),
            "mobileClient".to_string(),
            "mobileAppClient".to_string(),
            "server".to_string(),
        ],
        metadata_owner_kinds: metadata_owner_kinds
            .iter()
            .map(|value| (*value).to_string())
            .collect(),
        main_attribute_kinds: main_attribute_kinds
            .iter()
            .map(|value| (*value).to_string())
            .collect(),
        main_attribute_type_prefixes: main_attribute_type_prefixes
            .iter()
            .map(|value| (*value).to_string())
            .collect(),
        dynamic_list_source,
        events: Vec::new(),
        exclusion_reason: None,
    }
}

fn ensure_supplemental_form_catalogs(fixture: &mut PlatformEventCatalogFixture) {
    let catalogs = [
        supplemental_form_catalog(
            "Client application form extension for objects",
            "Form",
            &[
                "Catalog",
                "Document",
                "ExchangePlan",
                "ChartOfAccounts",
                "ChartOfCharacteristicTypes",
                "ChartOfCalculationTypes",
                "BusinessProcess",
                "Task",
                "DataProcessor",
                "ExternalDataProcessor",
                "ExternalReport",
            ],
            &["PersistentObject"],
            &[],
            false,
        ),
        supplemental_form_catalog(
            "Managed form extension for record set",
            "Form",
            &[],
            &["PersistentRecord"],
            &[
                "InformationRegisterRecordSet",
                "AccumulationRegisterRecordSet",
                "AccountingRegisterRecordSet",
                "CalculationRegisterRecordSet",
            ],
            false,
        ),
        supplemental_form_catalog(
            "Client application form extension for catalogs",
            "Form",
            &["Catalog"],
            &["PersistentObject"],
            &[],
            false,
        ),
        supplemental_form_catalog(
            "Client application form extension for documents",
            "Form",
            &["Document"],
            &["PersistentObject"],
            &[],
            false,
        ),
        supplemental_form_catalog(
            "Client application form extension for business processes",
            "Form",
            &["BusinessProcess"],
            &["PersistentObject"],
            &[],
            false,
        ),
        supplemental_form_catalog(
            "Managed form extension for tasks",
            "Form",
            &["Task"],
            &["PersistentObject"],
            &[],
            false,
        ),
        supplemental_form_catalog(
            "Client application form extension for chart of characteristics types",
            "Form",
            &["ChartOfCharacteristicTypes"],
            &["PersistentObject"],
            &[],
            false,
        ),
        supplemental_form_catalog(
            "Client application form extension for constants",
            "Form",
            &["Constant"],
            &["PersistentObject"],
            &["ConstantsSet"],
            false,
        ),
        supplemental_form_catalog(
            "Client application form extension for information register records",
            "Form",
            &["InformationRegister"],
            &["PersistentRecord"],
            &["InformationRegisterRecordManager"],
            false,
        ),
        supplemental_form_catalog(
            "Client application form extension for reports",
            "Form",
            &["Report", "ExternalReport"],
            &[],
            &[],
            false,
        ),
        supplemental_form_catalog(
            "Form table extension for dynamic list",
            "Table",
            &[],
            &["DynamicList"],
            &[],
            true,
        ),
    ];
    for catalog in catalogs {
        let existing = fixture.form_catalogs.iter().position(|existing| {
            existing.source_owner == catalog.source_owner
                && existing.owner_kinds == catalog.owner_kinds
        });
        if let Some(index) = existing {
            fixture.form_catalogs[index] = catalog;
        } else {
            fixture.form_catalogs.push(catalog);
        }
    }
}

fn derive_fixture(root: &Path) -> Result<PlatformEventCatalogFixture, String> {
    let mut fixture: PlatformEventCatalogFixture =
        serde_json::from_str(FIXTURE).map_err(|error| error.to_string())?;
    ensure_supplemental_form_catalogs(&mut fixture);
    let corpora = discover(root, "ru").map_err(|error| format!("{error:?}"))?;
    if corpora.version != fixture.source.installation_version {
        return Err(format!(
            "Syntax Assistant installation version differs: expected {}, actual {}",
            fixture.source.installation_version, corpora.version
        ));
    }
    let container = corpora
        .syntax_context
        .containers
        .into_iter()
        .find(|path| {
            path.file_name().and_then(|name| name.to_str())
                == Some(fixture.source.container.as_str())
        })
        .ok_or_else(|| {
            format!(
                "installed {} has no selected {} container",
                corpora.version, fixture.source.container
            )
        })?;
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
    let english_corpora = discover(root, "root").map_err(|error| format!("{error:?}"))?;
    let english_container = english_corpora
        .syntax_context
        .containers
        .into_iter()
        .find(|path| {
            path.file_name().and_then(|name| name.to_str())
                == Some(fixture.source.english_container.as_str())
        })
        .ok_or_else(|| {
            format!(
                "installed {} has no selected {} container",
                english_corpora.version, fixture.source.english_container
            )
        })?;
    let english_bytes = std::fs::read(&english_container).map_err(|error| error.to_string())?;
    let english_digest = Sha256::digest(&english_bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    if english_digest != fixture.source.english_sha256 {
        return Err(format!(
            "English Syntax Assistant digest differs for {}: expected {}, actual {english_digest}",
            english_container.display(),
            fixture.source.english_sha256
        ));
    }
    let english_pages = read_corpus(&english_bytes).map_err(|error| format!("{error:?}"))?;
    let english_by_path = english_pages
        .iter()
        .map(|page| (page.path.as_str(), page))
        .collect::<BTreeMap<_, _>>();
    let event_pages = pages
        .iter()
        .filter(|page| page.path.contains("/events/"))
        .collect::<Vec<_>>();
    fixture.source.event_markup_page_count = event_pages.len();
    fixture.source.signature_event_leaf_count = event_pages
        .iter()
        .filter(|page| page.signature.is_some())
        .count();
    let mut event_page_ids = event_pages
        .iter()
        .map(|page| page.path.as_str())
        .collect::<Vec<_>>();
    event_page_ids.sort_unstable();
    fixture.source.event_page_ids_sha256 = Sha256::digest(event_page_ids.join("\n").as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
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
            .map(|page| {
                derive_event(
                    page,
                    english_by_path.get(page.path.as_str()).copied(),
                    &catalog.base_contexts,
                    false,
                )
            })
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
            .map(|page| {
                derive_event(
                    page,
                    english_by_path.get(page.path.as_str()).copied(),
                    &catalog.base_contexts,
                    true,
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        for event in &mut catalog.events {
            if let Some(canonical) = catalog.event_id_overrides.get(&event.event_id) {
                event.event_id.clone_from(canonical);
            }
        }
    }
    fixture.excluded_structural_pages = structural_exclusions(&pages);
    fixture.excluded_external_data_source_pages = external_data_source_exclusions(&pages);
    fixture.excluded_generic_template_pages = generic_template_exclusions(&pages);
    fixture.excluded_out_of_profile_pages = out_of_profile_exclusions(&pages)?;

    let projected = fixture
        .module_catalogs
        .iter()
        .flat_map(|catalog| catalog.events.iter())
        .chain(
            fixture
                .form_catalogs
                .iter()
                .flat_map(|catalog| catalog.events.iter()),
        )
        .map(|event| source_page_path(&event.source_page_id))
        .collect::<BTreeSet<_>>();
    let approved = event_pages
        .iter()
        .filter(|page| is_approved_event_page(page))
        .map(|page| page.path.as_str())
        .collect::<BTreeSet<_>>();
    if projected != approved {
        let missing = approved.difference(&projected).copied().collect::<Vec<_>>();
        let unexpected = projected.difference(&approved).copied().collect::<Vec<_>>();
        return Err(format!(
            "approved source partition differs from derived catalog: missing={missing:#?}, unexpected={unexpected:#?}"
        ));
    }
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
fn vendor_oracle_rejects_a_matching_hbk_from_the_wrong_installation_version() {
    let Some(root) = installed_root() else {
        eprintln!("UNICA_PLATFORM_HELP_DIR is not set; platform version oracle skipped");
        return;
    };
    let checked: PlatformEventCatalogFixture =
        serde_json::from_str(FIXTURE).expect("checked event fixture");
    let temp = tempfile::tempdir().expect("temporary installation parent");
    let wrong_root = temp.path().join("8.3.27.9999");
    std::fs::create_dir(&wrong_root).expect("wrong-version installation root");
    std::fs::copy(
        root.join(&checked.source.container),
        wrong_root.join(&checked.source.container),
    )
    .expect("copy pinned syntax assistant under a deliberately wrong version");

    let error = derive_fixture(&wrong_root)
        .expect_err("the installation root version is source identity, not fixture metadata");
    assert!(
        error.contains("8.3.27.9999") && error.contains("8.3.27.2074"),
        "version mismatch must name actual and expected versions: {error}"
    );
}

#[test]
fn an_unknown_vendor_owner_cannot_enter_the_reviewed_out_of_profile_category() {
    let page = CorpusPage {
        path: "objects/future/events/Changed1.html".to_string(),
        title: "Изменилось (FutureOwner.Changed)".to_string(),
        text: String::new(),
        signature: None,
    };
    assert_eq!(reviewed_out_of_profile_reason(&page), None);
}

#[test]
fn bilingual_template_exceptions_are_exact_and_source_guarded() {
    let Some(root) = installed_root() else {
        eprintln!("UNICA_PLATFORM_HELP_DIR is not set; bilingual exception oracle skipped");
        return;
    };
    let checked: PlatformEventCatalogFixture =
        serde_json::from_str(FIXTURE).expect("checked event fixture");
    let ru_bytes = std::fs::read(root.join(&checked.source.container)).unwrap();
    let en_bytes = std::fs::read(root.join(&checked.source.english_container)).unwrap();
    let ru_pages = read_corpus(&ru_bytes).unwrap();
    let en_pages = read_corpus(&en_bytes).unwrap();
    let mut broken = ru_pages
        .iter()
        .filter(|page| is_approved_event_page(page))
        .filter_map(|page| {
            let signature = page.signature.as_ref()?;
            let raw = signature.en.as_deref()?;
            declaration_kind(&declaration(raw)).is_err().then_some((
                page.path.as_str(),
                signature.ru.as_deref(),
                raw,
            ))
        })
        .collect::<Vec<_>>();
    broken.sort_by_key(|(path, _, _)| *path);
    assert_eq!(
        broken,
        [
            (
                "objects/catalog1649/catalog1679/catalog2421/Form table extension for dynamic list/events/URLGetProcessing713.html",
                Some("Процедура ОбработкаПолученияНавигационнойСсылки(Элемент, Ключ, Представление, НавигационнаяСсылка, СтандартнаяОбработка)<br>    <?><br>КонецПроцедуры"),
                ",",
            ),
            (
                "objects/catalog1649/catalog1679/catalog2421/Form table extension for dynamic list/events/URLListGetProcessing712.html",
                Some("Процедура ОбработкаПолученияСпискаНавигационныхСсылок(Элемент, СписокНавигационыхСсылок, КлючПоУмолчанию)<br>    <?><br>КонецПроцедуры"),
                ",",
            ),
        ]
    );
    let expected = [
        (
            broken[0].0,
            "Syntax: URLGetProcessing(",
            "Procedure URLGetProcessing(Item, Key, Presentation, URL, StandardProcessing)",
        ),
        (
            broken[1].0,
            "Syntax: URLListGetProcessing(",
            "Procedure URLListGetProcessing(Item, URLList, DefaultKey)",
        ),
    ];
    for (page_id, evidence, canonical) in expected {
        let english_page = en_pages.iter().find(|page| page.path == page_id).unwrap();
        assert!(english_page.text.contains(evidence), "{page_id}");
        let checked_event = checked
            .form_catalogs
            .iter()
            .flat_map(|catalog| catalog.events.iter())
            .find(|event| source_page_path(&event.source_page_id) == page_id)
            .unwrap();
        assert_eq!(checked_event.signature_en, canonical, "{page_id}");
    }
}

#[test]
fn every_vendor_event_markup_page_belongs_to_one_closed_partition_category() {
    let Some(root) = installed_root() else {
        eprintln!("UNICA_PLATFORM_HELP_DIR is not set; platform event partition oracle skipped");
        return;
    };
    let checked: PlatformEventCatalogFixture =
        serde_json::from_str(FIXTURE).expect("checked event fixture");
    let bytes = std::fs::read(root.join(&checked.source.container)).expect("syntax assistant");
    let pages = read_corpus(&bytes).expect("read complete syntax assistant");
    let event_pages = pages
        .iter()
        .filter(|page| page.path.contains("/events/"))
        .collect::<Vec<_>>();
    assert_eq!(event_pages.len(), 693, "pinned HBK event markup census");

    let selected = checked
        .module_catalogs
        .iter()
        .flat_map(|catalog| catalog.events.iter())
        .chain(
            checked
                .form_catalogs
                .iter()
                .flat_map(|catalog| catalog.events.iter()),
        )
        .map(|event| source_page_path(&event.source_page_id))
        .collect::<BTreeSet<_>>();
    let structural_or_deferred = checked
        .excluded_structural_pages
        .iter()
        .chain(checked.excluded_external_data_source_pages.iter())
        .chain(checked.excluded_generic_template_pages.iter())
        .chain(checked.excluded_out_of_profile_pages.iter())
        .map(|page| source_page_path(&page.page_id))
        .collect::<BTreeSet<_>>();
    let duplicate_categories = selected
        .intersection(&structural_or_deferred)
        .copied()
        .collect::<Vec<_>>();
    assert!(
        duplicate_categories.is_empty(),
        "page IDs classified twice: {duplicate_categories:#?}"
    );

    let classified = selected
        .union(&structural_or_deferred)
        .copied()
        .collect::<BTreeSet<_>>();
    let mut unclassified_by_owner = BTreeMap::<String, Vec<String>>::new();
    for page in event_pages
        .iter()
        .filter(|page| !classified.contains(page.path.as_str()))
    {
        unclassified_by_owner
            .entry(event_owner(page).unwrap_or("<ownerless>").to_string())
            .or_default()
            .push(format!("{} | {}", page.path, page.title));
    }
    assert!(
        unclassified_by_owner.is_empty(),
        "{} of {} event markup pages are unclassified across {} owners",
        unclassified_by_owner.values().map(Vec::len).sum::<usize>(),
        event_pages.len(),
        unclassified_by_owner.len()
    );
    assert_eq!(classified.len(), event_pages.len());
}

// Исследование, не тест: собирается только с фичей `research`, запуск —
// scripts/research/platform-event-catalog.sh. `#[ignore]` остаётся стражем от
// случайного прогона с включённой фичей.
#[cfg(feature = "research")]
#[test]
#[ignore = "research: writes the mechanically derived compact fixture; run scripts/research/platform-event-catalog.sh"]
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
