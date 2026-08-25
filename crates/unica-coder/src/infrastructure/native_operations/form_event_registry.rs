//! Registry and validation rules for managed-form event bindings.
//!
//! The registry deliberately keeps XML context discovery separate from form
//! mutation. Callers can validate a proposed binding before changing the
//! source document and reuse the same rules from `form.edit`, `form.compile`,
//! and `form.validate`.

use roxmltree::Node;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;
use std::sync::OnceLock;

use crate::domain::address::NodeKind;
use crate::domain::module_projection::BindingFact;
use crate::domain::platform_profile::ModuleCapability;

use super::common::is_1c_identifier;

const FORM_LOGFORM_NS: &str = "http://v8.1c.ru/8.3/xcf/logform";
const FORM_V8_NS: &str = "http://v8.1c.ru/8.1/data/core";

const FORM_EVENTS: &[&str] = &[
    "OnCreateAtServer",
    "OnOpen",
    "BeforeClose",
    "OnClose",
    "NotificationProcessing",
    "ChoiceProcessing",
    "ExternalEvent",
    "OnReopen",
    "OnMainServerAvailabilityChange",
    "OnReadAtServer",
    "BeforeWrite",
    "NewWriteProcessing",
    "FillCheckProcessingAtServer",
    "BeforeWriteAtServer",
    "OnWriteAtServer",
    "AfterWriteAtServer",
    "AfterWrite",
    "BeforeLoadDataFromSettingsAtServer",
    "OnLoadDataFromSettingsAtServer",
    "OnSaveDataInSettingsAtServer",
    "BeforeLoadUserSettingsAtServer",
    "OnLoadUserSettingsAtServer",
    "OnSaveUserSettingsAtServer",
    "OnUpdateUserSettingSetAtServer",
    "BeforeLoadVariantAtServer",
    "OnLoadVariantAtServer",
    "OnSaveVariantAtServer",
    "OnChangeDisplaySettings",
    "URLProcessing",
    "URLListGetProcessing",
    "URLGetProcessing",
    "NavigationProcessing",
];

const OBJECT_RECORD_FORM_EVENTS: &[&str] = &[
    "OnReadAtServer",
    "BeforeWrite",
    "BeforeWriteAtServer",
    "OnWriteAtServer",
    "AfterWriteAtServer",
    "AfterWrite",
];

const INPUT_FIELD_EVENTS: &[&str] = &[
    "OnChange",
    "StartChoice",
    "Clearing",
    "ChoiceProcessing",
    "AutoComplete",
    "TextEditEnd",
    "Opening",
    "Creating",
    "EditTextChange",
    "Tuning",
    "StartListChoice",
    "MultipleValuesDelete",
];
const CHECK_BOX_FIELD_EVENTS: &[&str] = &["OnChange"];
const RADIO_BUTTON_FIELD_EVENTS: &[&str] = &["OnChange"];
const TRACK_BAR_FIELD_EVENTS: &[&str] = &["OnChange"];
const LABEL_DECORATION_EVENTS: &[&str] = &["Click", "URLProcessing"];
const LABEL_FIELD_EVENTS: &[&str] = &["URLProcessing", "Click", "OnChange"];
const TABLE_EVENTS: &[&str] = &[
    "Selection",
    "OnActivateRow",
    "BeforeAddRow",
    "BeforeDeleteRow",
    "OnStartEdit",
    "OnChange",
    "BeforeRowChange",
    "AfterDeleteRow",
    "OnEditEnd",
    "OnActivateCell",
    "OnGetDataAtServer",
    "Drag",
    "DragCheck",
    "ValueChoice",
    "ChoiceProcessing",
    "DragStart",
    "BeforeEditEnd",
    "BeforeExpand",
    "DragEnd",
    "OnUpdateUserSettingSetAtServer",
    "BeforeCollapse",
    "BeforeLoadUserSettingsAtServer",
    "OnActivateField",
    "RefreshRequestProcessing",
    "NewWriteProcessing",
    "OnLoadUserSettingsAtServer",
    "OnCurrentParentChange",
    "OnSaveUserSettingsAtServer",
    "URLGetProcessing",
];
const PAGES_EVENTS: &[&str] = &["OnCurrentPageChange"];
const PICTURE_DECORATION_EVENTS: &[&str] = &["Click", "Drag", "DragCheck"];
const PICTURE_FIELD_EVENTS: &[&str] = &["Click"];
const CALENDAR_FIELD_EVENTS: &[&str] = &["Selection", "OnChange", "OnPeriodOutput"];
const EXTENDED_TOOLTIP_EVENTS: &[&str] = &["URLProcessing", "Click"];
const DOCUMENT_CHANGE_EVENTS: &[&str] = &["OnChange"];
const GRAPHICAL_SCHEMA_FIELD_EVENTS: &[&str] = &["Selection", "OnActivate"];
const HTML_DOCUMENT_FIELD_EVENTS: &[&str] = &["OnClick", "DocumentComplete"];
const SPREADSHEET_DOCUMENT_FIELD_EVENTS: &[&str] = &[
    "DetailProcessing",
    "Selection",
    "OnActivate",
    "AdditionalDetailProcessing",
    "OnChange",
    "Drag",
    "URLProcessing",
    "BeforePrint",
    "BeforeWrite",
    "DragCheck",
    "OnChangeAreaContent",
];
const NO_EVENTS: &[&str] = &[];

/// One versioned possible-event record. The registry owns applicability and
/// exact expected shape; module projection only evaluates source evidence
/// against these records.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PlatformEventSpec {
    pub(crate) event_id: String,
    pub(crate) handler_ru: String,
    pub(crate) handler_en: String,
    pub(crate) signature_ru: String,
    pub(crate) signature_en: String,
    pub(crate) method_kind: String,
    pub(crate) contexts: Vec<String>,
    pub(crate) source_page_id: String,
    #[serde(skip, default = "platform_binding")]
    pub(crate) binding: BindingFact,
}

const fn platform_binding() -> BindingFact {
    BindingFact::Platform
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PlatformEventCatalogSource {
    pub(crate) installation_version: String,
    pub(crate) container: String,
    pub(crate) sha256: String,
    pub(crate) english_container: String,
    pub(crate) english_sha256: String,
    #[serde(default)]
    pub(crate) event_markup_page_count: usize,
    #[serde(default)]
    pub(crate) signature_event_leaf_count: usize,
    #[serde(default)]
    pub(crate) event_page_ids_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ModuleEventCatalog {
    pub(crate) owner_kind: String,
    pub(crate) module_role: String,
    pub(crate) source_owner: Option<String>,
    pub(crate) source_path_prefix: Option<String>,
    pub(crate) base_contexts: Vec<String>,
    pub(crate) events: Vec<PlatformEventSpec>,
    pub(crate) exclusion_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FormPlatformEventCatalog {
    pub(crate) owner_kinds: Vec<String>,
    pub(crate) source_owner: Option<String>,
    #[serde(default)]
    pub(crate) inherited_source_owners: Vec<String>,
    #[serde(default)]
    pub(crate) event_id_overrides: BTreeMap<String, String>,
    pub(crate) base_contexts: Vec<String>,
    #[serde(default)]
    pub(crate) metadata_owner_kinds: Vec<String>,
    #[serde(default)]
    pub(crate) main_attribute_kinds: Vec<String>,
    #[serde(default)]
    pub(crate) main_attribute_type_prefixes: Vec<String>,
    #[serde(default)]
    pub(crate) dynamic_list_source: bool,
    pub(crate) events: Vec<PlatformEventSpec>,
    pub(crate) exclusion_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExcludedEventPage {
    pub(crate) page_id: String,
    pub(crate) title: String,
    pub(crate) reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PlatformEventCatalogFixture {
    pub(crate) profile: String,
    pub(crate) source: PlatformEventCatalogSource,
    pub(crate) module_catalogs: Vec<ModuleEventCatalog>,
    pub(crate) form_catalogs: Vec<FormPlatformEventCatalog>,
    #[serde(default, alias = "excludedUnmatchedPages")]
    pub(crate) excluded_structural_pages: Vec<ExcludedEventPage>,
    #[serde(default)]
    pub(crate) excluded_external_data_source_pages: Vec<ExcludedEventPage>,
    #[serde(default)]
    pub(crate) excluded_generic_template_pages: Vec<ExcludedEventPage>,
    #[serde(default)]
    pub(crate) excluded_out_of_profile_pages: Vec<ExcludedEventPage>,
}

pub(crate) fn platform_event_catalog_fixture() -> &'static PlatformEventCatalogFixture {
    static FIXTURE: OnceLock<PlatformEventCatalogFixture> = OnceLock::new();
    FIXTURE.get_or_init(|| {
        serde_json::from_str(include_str!("../platform-event-catalog-8.3.27.2074.json"))
            .expect("the checked 8.3.27.2074 platform event catalog must be valid")
    })
}

/// Possible module events for the exact 8.3.27 capability. Service handlers
/// and form events intentionally remain on their declarative logical owners.
pub(crate) fn module_event_catalog_8_3_27(
    capability: ModuleCapability,
) -> &'static [PlatformEventSpec] {
    let owner_kind = capability.owner_kind().as_str();
    let role = capability.role().as_str();
    platform_event_catalog_fixture()
        .module_catalogs
        .iter()
        .find(|catalog| {
            catalog.module_role == role
                && (catalog.owner_kind == owner_kind || catalog.owner_kind == "*")
        })
        .map_or(&[], |catalog| catalog.events.as_slice())
}

const NAMED_PERSISTENT_OBJECT_TYPES: &[&str] = &[
    "CatalogObject",
    "DocumentObject",
    "BusinessProcessObject",
    "TaskObject",
    "ExchangePlanObject",
    "ChartOfAccountsObject",
    "ChartOfCharacteristicTypesObject",
    "ChartOfCalculationTypesObject",
];

const NAMED_PERSISTENT_RECORD_TYPES: &[&str] = &[
    "InformationRegisterRecordManager",
    "InformationRegisterRecordSet",
    "AccumulationRegisterRecordSet",
    "AccountingRegisterRecordSet",
    "CalculationRegisterRecordSet",
];

/// Distinguishes a configuration form from an extension form without a
/// call-site boolean whose meaning could be ambiguous.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FormDefinitionKind {
    Regular,
    Extension,
}

/// Relevant class of the form's direct main attribute.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MainAttributeKind {
    PersistentObject,
    PersistentRecord,
    DynamicList,
    Other,
    Unknown,
}

/// Records where the effective main-attribute context came from.  The
/// distinction matters for borrowed extension forms: an absent inherited
/// context can only be reported as unverified, while a malformed direct
/// override is a real validation error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MainAttributeProvenance {
    DirectForm,
    DirectBaseForm,
    InheritedBaseFormUnavailable,
    Missing,
}

impl MainAttributeKind {
    pub(crate) fn from_type_name(type_name: &str) -> Self {
        let type_name = type_name.trim();
        if type_name.is_empty() {
            return Self::Unknown;
        }

        let unqualified = type_name.strip_prefix("cfg:").unwrap_or(type_name);
        if unqualified == "ConstantsSet" {
            Self::PersistentObject
        } else if unqualified == "DynamicList" {
            Self::DynamicList
        } else {
            let mut parts = unqualified.split('.');
            let family = parts.next().unwrap_or_default();
            let object_name = parts.next().unwrap_or_default();
            let is_exact_named_type = is_1c_identifier(object_name) && parts.next().is_none();
            if is_exact_named_type && NAMED_PERSISTENT_OBJECT_TYPES.contains(&family) {
                Self::PersistentObject
            } else if is_exact_named_type && NAMED_PERSISTENT_RECORD_TYPES.contains(&family) {
                Self::PersistentRecord
            } else {
                Self::Other
            }
        }
    }

    const fn catalog_key(self) -> &'static str {
        match self {
            Self::PersistentObject => "PersistentObject",
            Self::PersistentRecord => "PersistentRecord",
            Self::DynamicList => "DynamicList",
            Self::Other => "Other",
            Self::Unknown => "Unknown",
        }
    }
}

/// Form-level information needed by context-sensitive event rules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FormEventContext {
    pub(crate) definition: FormDefinitionKind,
    /// Whether Part 1 contains any direct form node besides `BaseForm`.
    /// A literal BaseForm-only projection proves no directly owned insertion
    /// surface and must therefore fail closed for Available events.
    pub(crate) direct_part_writable: bool,
    pub(crate) main_attribute: MainAttributeKind,
    pub(crate) main_attribute_type: Option<String>,
    pub(crate) main_attribute_provenance: MainAttributeProvenance,
    pub(crate) main_attribute_name: Option<String>,
    pub(crate) metadata_owner: Option<NodeKind>,
}

impl FormEventContext {
    /// Reads only direct logform children. A root `MainAttribute` wins; an
    /// extension's direct `BaseForm` is a fallback. Arbitrary descendants are
    /// intentionally ignored so nested elements cannot change form context.
    pub(crate) fn from_root(root: Node<'_, '_>) -> Self {
        let base_form = direct_logform_child(root, "BaseForm");
        let definition = if base_form.is_some() {
            FormDefinitionKind::Extension
        } else {
            FormDefinitionKind::Regular
        };
        let direct_part_writable = definition == FormDefinitionKind::Regular
            || root.children().any(|child| {
                child.is_element()
                    && child.tag_name().namespace() == Some(FORM_LOGFORM_NS)
                    && child.tag_name().name() != "BaseForm"
            });
        let form_main_attribute = direct_main_attribute(root);
        let base_main_attribute = base_form.and_then(direct_main_attribute);
        let (main_attribute_type, main_attribute_name, main_attribute_provenance) =
            if let Some(main_attribute) = form_main_attribute {
                (
                    main_attribute_type(main_attribute),
                    main_attribute.attribute("name").map(str::to_string),
                    MainAttributeProvenance::DirectForm,
                )
            } else if let Some(main_attribute) = base_main_attribute {
                (
                    main_attribute_type(main_attribute),
                    main_attribute.attribute("name").map(str::to_string),
                    MainAttributeProvenance::DirectBaseForm,
                )
            } else if definition == FormDefinitionKind::Extension {
                (
                    None,
                    None,
                    MainAttributeProvenance::InheritedBaseFormUnavailable,
                )
            } else {
                (None, None, MainAttributeProvenance::Missing)
            };
        let main_attribute = main_attribute_type
            .as_deref()
            .map(MainAttributeKind::from_type_name)
            .unwrap_or(MainAttributeKind::Unknown);

        Self {
            definition,
            direct_part_writable,
            main_attribute,
            main_attribute_type,
            main_attribute_provenance,
            main_attribute_name,
            metadata_owner: None,
        }
    }
}

pub(crate) fn context_from_root(root: Node<'_, '_>) -> FormEventContext {
    FormEventContext::from_root(root)
}

/// Element categories used by the compact form DSL and their XML equivalents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum FormElementKind {
    InputField,
    CheckBoxField,
    RadioButtonField,
    TrackBarField,
    LabelDecoration,
    LabelField,
    Table,
    Pages,
    Page,
    Button,
    PictureField,
    CalendarField,
    PictureDecoration,
    ExtendedTooltip,
    FormattedDocumentField,
    TextDocumentField,
    GraphicalSchemaField,
    HtmlDocumentField,
    SpreadsheetDocumentField,
    CommandBar,
    Group,
}

impl FormElementKind {
    #[cfg(test)]
    const ALL: [Self; 21] = [
        Self::InputField,
        Self::CheckBoxField,
        Self::RadioButtonField,
        Self::TrackBarField,
        Self::LabelDecoration,
        Self::LabelField,
        Self::Table,
        Self::Pages,
        Self::Page,
        Self::Button,
        Self::PictureField,
        Self::CalendarField,
        Self::PictureDecoration,
        Self::ExtendedTooltip,
        Self::FormattedDocumentField,
        Self::TextDocumentField,
        Self::GraphicalSchemaField,
        Self::HtmlDocumentField,
        Self::SpreadsheetDocumentField,
        Self::CommandBar,
        Self::Group,
    ];

    const fn catalog_key(self) -> &'static str {
        match self {
            Self::InputField => "InputField",
            Self::CheckBoxField => "CheckBoxField",
            Self::RadioButtonField => "RadioButtonField",
            Self::TrackBarField => "TrackBarField",
            Self::LabelDecoration => "LabelDecoration",
            Self::LabelField => "LabelField",
            Self::Table => "Table",
            Self::Pages => "Pages",
            Self::Page => "Page",
            Self::Button => "Button",
            Self::PictureField => "PictureField",
            Self::CalendarField => "CalendarField",
            Self::PictureDecoration => "PictureDecoration",
            Self::ExtendedTooltip => "ExtendedTooltip",
            Self::FormattedDocumentField => "FormattedDocumentField",
            Self::TextDocumentField => "TextDocumentField",
            Self::GraphicalSchemaField => "GraphicalSchemaField",
            Self::HtmlDocumentField => "HtmlDocumentField",
            Self::SpreadsheetDocumentField => "SpreadsheetDocumentField",
            Self::CommandBar => "CommandBar",
            Self::Group => "Group",
        }
    }

    pub(crate) fn from_xml_tag(tag: &str) -> Option<Self> {
        match tag {
            "InputField" => Some(Self::InputField),
            "CheckBoxField" => Some(Self::CheckBoxField),
            "RadioButtonField" => Some(Self::RadioButtonField),
            "TrackBarField" => Some(Self::TrackBarField),
            "LabelDecoration" => Some(Self::LabelDecoration),
            "LabelField" => Some(Self::LabelField),
            "Table" => Some(Self::Table),
            "Pages" => Some(Self::Pages),
            "Page" => Some(Self::Page),
            "Button" => Some(Self::Button),
            "PictureField" => Some(Self::PictureField),
            "CalendarField" => Some(Self::CalendarField),
            "PictureDecoration" => Some(Self::PictureDecoration),
            "ExtendedTooltip" => Some(Self::ExtendedTooltip),
            "FormattedDocumentField" => Some(Self::FormattedDocumentField),
            "TextDocumentField" => Some(Self::TextDocumentField),
            "GraphicalSchemaField" => Some(Self::GraphicalSchemaField),
            "HTMLDocumentField" => Some(Self::HtmlDocumentField),
            "SpreadSheetDocumentField" => Some(Self::SpreadsheetDocumentField),
            "CommandBar" | "AutoCommandBar" => Some(Self::CommandBar),
            "UsualGroup" => Some(Self::Group),
            _ => None,
        }
    }

    #[cfg(test)]
    pub(crate) fn from_dsl_key(key: &str) -> Option<Self> {
        match key {
            "input" => Some(Self::InputField),
            "check" => Some(Self::CheckBoxField),
            "radio" => Some(Self::RadioButtonField),
            "trackBar" => Some(Self::TrackBarField),
            "label" => Some(Self::LabelDecoration),
            "labelField" => Some(Self::LabelField),
            "table" => Some(Self::Table),
            "pages" => Some(Self::Pages),
            "page" => Some(Self::Page),
            "button" => Some(Self::Button),
            "picField" => Some(Self::PictureField),
            "calendar" => Some(Self::CalendarField),
            "picture" => Some(Self::PictureDecoration),
            "extendedTooltip" => Some(Self::ExtendedTooltip),
            "formattedDoc" => Some(Self::FormattedDocumentField),
            "textDoc" => Some(Self::TextDocumentField),
            "graphicalSchema" => Some(Self::GraphicalSchemaField),
            "html" => Some(Self::HtmlDocumentField),
            "spreadsheet" => Some(Self::SpreadsheetDocumentField),
            "cmdBar" => Some(Self::CommandBar),
            "group" => Some(Self::Group),
            _ => None,
        }
    }

    pub(crate) const fn dsl_key(self) -> &'static str {
        match self {
            Self::InputField => "input",
            Self::CheckBoxField => "check",
            Self::RadioButtonField => "radio",
            Self::TrackBarField => "trackBar",
            Self::LabelDecoration => "label",
            Self::LabelField => "labelField",
            Self::Table => "table",
            Self::Pages => "pages",
            Self::Page => "page",
            Self::Button => "button",
            Self::PictureField => "picField",
            Self::CalendarField => "calendar",
            Self::PictureDecoration => "picture",
            Self::ExtendedTooltip => "extendedTooltip",
            Self::FormattedDocumentField => "formattedDoc",
            Self::TextDocumentField => "textDoc",
            Self::GraphicalSchemaField => "graphicalSchema",
            Self::HtmlDocumentField => "html",
            Self::SpreadsheetDocumentField => "spreadsheet",
            Self::CommandBar => "cmdBar",
            Self::Group => "group",
        }
    }

    pub(crate) const fn allowed_events(self) -> &'static [&'static str] {
        match self {
            Self::InputField => INPUT_FIELD_EVENTS,
            Self::CheckBoxField => CHECK_BOX_FIELD_EVENTS,
            Self::RadioButtonField => RADIO_BUTTON_FIELD_EVENTS,
            Self::TrackBarField => TRACK_BAR_FIELD_EVENTS,
            Self::LabelDecoration => LABEL_DECORATION_EVENTS,
            Self::LabelField => LABEL_FIELD_EVENTS,
            Self::Table => TABLE_EVENTS,
            Self::Pages => PAGES_EVENTS,
            Self::Page | Self::Button | Self::CommandBar | Self::Group => NO_EVENTS,
            Self::PictureField => PICTURE_FIELD_EVENTS,
            Self::CalendarField => CALENDAR_FIELD_EVENTS,
            Self::PictureDecoration => PICTURE_DECORATION_EVENTS,
            Self::ExtendedTooltip => EXTENDED_TOOLTIP_EVENTS,
            Self::FormattedDocumentField | Self::TextDocumentField => DOCUMENT_CHANGE_EVENTS,
            Self::GraphicalSchemaField => GRAPHICAL_SCHEMA_FIELD_EVENTS,
            Self::HtmlDocumentField => HTML_DOCUMENT_FIELD_EVENTS,
            Self::SpreadsheetDocumentField => SPREADSHEET_DOCUMENT_FIELD_EVENTS,
        }
    }
}

impl fmt::Display for FormElementKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.dsl_key())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FormEventTarget {
    Form,
    Element(FormElementKind),
}

impl FormEventTarget {
    pub(crate) const fn allowed_events(self) -> &'static [&'static str] {
        match self {
            Self::Form => FORM_EVENTS,
            Self::Element(kind) => kind.allowed_events(),
        }
    }
}

impl fmt::Display for FormEventTarget {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Form => formatter.write_str("form"),
            Self::Element(kind) => write!(formatter, "element type '{kind}'"),
        }
    }
}

/// Logical form-event owner. `Table` and nested `Column` are distinct even
/// though both reuse the existing element event matrices.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum FormEventOwnerKind {
    Form,
    Element(FormElementKind),
    Table,
    Column(FormElementKind),
    Command,
}

impl FormEventOwnerKind {
    fn catalog_key(self) -> String {
        match self {
            Self::Form => "Form".to_string(),
            Self::Element(kind) => format!("Element.{}", kind.catalog_key()),
            Self::Table => "Table".to_string(),
            Self::Column(kind) => format!("Column.{}", kind.catalog_key()),
            Self::Command => "Command".to_string(),
        }
    }
}

/// Form possible events use the v0.12 owner taxonomy and the checked vendor
/// 8.3.27 catalog. The v0.12 validation matrices remain unchanged.
pub(crate) fn form_event_catalog_8_3_27(
    context: &FormEventContext,
    owner: FormEventOwnerKind,
    data_path: Option<&str>,
) -> Vec<&'static PlatformEventSpec> {
    if owner == FormEventOwnerKind::Command {
        return vec![form_command_execute_event_8_3_27()];
    }
    let key = owner.catalog_key();
    let normalized_type = context
        .main_attribute_type
        .as_deref()
        .map(|value| value.strip_prefix("cfg:").unwrap_or(value));
    let is_dynamic_list_source = owner == FormEventOwnerKind::Table
        && context.main_attribute == MainAttributeKind::DynamicList
        && context.main_attribute_name.as_deref().is_some_and(|name| {
            data_path
                .map(str::trim)
                .map(|path| path.trim_start_matches('~'))
                .and_then(|path| path.split('.').next())
                .is_some_and(|root| root == name)
        });
    let mut selected = platform_event_catalog_fixture()
        .form_catalogs
        .iter()
        .filter(|catalog| catalog.owner_kinds.contains(&key))
        .filter(|catalog| {
            (catalog.metadata_owner_kinds.is_empty()
                || context.metadata_owner.is_some_and(|owner| {
                    catalog
                        .metadata_owner_kinds
                        .iter()
                        .any(|expected| expected == owner.as_str())
                }))
                && (catalog.main_attribute_kinds.is_empty()
                    || catalog
                        .main_attribute_kinds
                        .iter()
                        .any(|expected| expected == context.main_attribute.catalog_key()))
                && (catalog.main_attribute_type_prefixes.is_empty()
                    || normalized_type.is_some_and(|actual| {
                        catalog
                            .main_attribute_type_prefixes
                            .iter()
                            .any(|prefix| actual.starts_with(prefix))
                    }))
                && (!catalog.dynamic_list_source || is_dynamic_list_source)
        })
        .flat_map(|catalog| catalog.events.iter())
        .collect::<Vec<_>>();
    // More specific catalogs follow base catalogs in the checked fixture.
    // Keep the last exact event ID so owner-specific signatures replace a
    // generic object/record signature rather than producing two Event nodes.
    let mut by_id = BTreeMap::new();
    for event in selected.drain(..) {
        by_id.insert(event.event_id.as_str(), event);
    }
    by_id.into_values().collect()
}

/// A managed-form command's direct `Action` is a form property binding, not
/// the metadata Command-module `CommandProcessing` event. Keep this record
/// outside the checked vendor module catalog so that catalog stays byte-exact.
fn form_command_execute_event_8_3_27() -> &'static PlatformEventSpec {
    static EVENT: OnceLock<PlatformEventSpec> = OnceLock::new();
    EVENT.get_or_init(|| PlatformEventSpec {
        event_id: "Execute".to_string(),
        handler_ru: "ОбработкаКоманды".to_string(),
        handler_en: "CommandProcessing".to_string(),
        signature_ru: "Процедура ОбработкаКоманды(Команда)".to_string(),
        signature_en: "Procedure CommandProcessing(Command)".to_string(),
        method_kind: "procedure".to_string(),
        contexts: [
            "thinClient",
            "webClient",
            "thickClientManaged",
            "mobileClient",
            "mobileAppClient",
        ]
        .into_iter()
        .map(str::to_string)
        .collect(),
        source_page_id: "platform-format:managed-form-command-action:8.3.27".to_string(),
        binding: BindingFact::Property,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FormCallType {
    Before,
    After,
    Override,
}

impl FormCallType {
    pub(crate) fn from_xml(value: &str) -> Option<Self> {
        match value {
            "Before" => Some(Self::Before),
            "After" => Some(Self::After),
            "Override" => Some(Self::Override),
            _ => None,
        }
    }

    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Before => "Before",
            Self::After => "After",
            Self::Override => "Override",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FormEventBinding<'a> {
    pub(crate) name: &'a str,
    pub(crate) handler: &'a str,
    pub(crate) call_type: Option<&'a str>,
}

impl<'a> FormEventBinding<'a> {
    pub(crate) const fn new(name: &'a str, handler: &'a str) -> Self {
        Self {
            name,
            handler,
            call_type: None,
        }
    }

    #[must_use]
    pub(crate) const fn with_call_type(mut self, call_type: &'a str) -> Self {
        self.call_type = Some(call_type);
        self
    }
}

/// Stable machine-readable event diagnostic codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum FormEventDiagnosticCode {
    EventNotAllowed,
    ContextUnknown,
    EmptyHandler,
    Duplicate,
    BindingConflict,
    TargetNotFound,
    InvalidCallType,
    CallTypeRequired,
    CallTypeNotAllowed,
}

impl FormEventDiagnosticCode {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::EventNotAllowed => "FORM_EVENT_NOT_ALLOWED",
            Self::ContextUnknown => "FORM_EVENT_CONTEXT_UNKNOWN",
            Self::EmptyHandler => "FORM_EVENT_EMPTY_HANDLER",
            Self::Duplicate => "FORM_EVENT_DUPLICATE",
            Self::BindingConflict => "FORM_EVENT_BINDING_CONFLICT",
            Self::TargetNotFound => "FORM_EVENT_TARGET_NOT_FOUND",
            Self::InvalidCallType => "FORM_EVENT_INVALID_CALL_TYPE",
            Self::CallTypeRequired => "FORM_EVENT_CALL_TYPE_REQUIRED",
            Self::CallTypeNotAllowed => "FORM_EVENT_CALL_TYPE_NOT_ALLOWED",
        }
    }
}

impl fmt::Display for FormEventDiagnosticCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FormEventDiagnostic {
    pub(crate) code: FormEventDiagnosticCode,
    pub(crate) target: String,
    pub(crate) event: String,
    pub(crate) detail: String,
}

impl FormEventDiagnostic {
    pub(crate) fn new(
        code: FormEventDiagnosticCode,
        target: impl Into<String>,
        event: impl Into<String>,
    ) -> Self {
        Self {
            code,
            target: target.into(),
            event: event.into(),
            detail: String::new(),
        }
    }

    #[must_use]
    pub(crate) fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = detail.into();
        self
    }
}

impl fmt::Display for FormEventDiagnostic {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.detail.is_empty() {
            write!(
                formatter,
                "[{}] event '{}' on {}",
                self.code, self.event, self.target
            )
        } else {
            write!(
                formatter,
                "[{}] event '{}' on {}: {}",
                self.code, self.event, self.target, self.detail
            )
        }
    }
}

impl std::error::Error for FormEventDiagnostic {}

pub(crate) fn validate_event(
    context: &FormEventContext,
    target: FormEventTarget,
    binding: &FormEventBinding<'_>,
) -> Result<(), FormEventDiagnostic> {
    let target_text = target.to_string();

    if binding.handler.trim().is_empty() {
        return Err(FormEventDiagnostic::new(
            FormEventDiagnosticCode::EmptyHandler,
            target_text,
            binding.name,
        )
        .with_detail("handler must not be empty"));
    }

    if !target.allowed_events().contains(&binding.name) {
        return Err(FormEventDiagnostic::new(
            FormEventDiagnosticCode::EventNotAllowed,
            target_text,
            binding.name,
        )
        .with_detail("event is not present in the target event matrix"));
    }

    validate_event_call_type(context, target, binding)?;

    if target == FormEventTarget::Form && OBJECT_RECORD_FORM_EVENTS.contains(&binding.name) {
        validate_object_event_context(context, target, binding.name)?;
    }

    Ok(())
}

/// Validates the extension-only XML binding fact independently from the
/// legacy v0.12 event-name matrix. Hidden v0.13 projection uses the vendor
/// catalog for applicability, while sharing this exact context rule.
pub(crate) fn validate_event_call_type(
    context: &FormEventContext,
    target: FormEventTarget,
    binding: &FormEventBinding<'_>,
) -> Result<(), FormEventDiagnostic> {
    if context.definition == FormDefinitionKind::Extension && binding.call_type.is_none() {
        return Err(FormEventDiagnostic::new(
            FormEventDiagnosticCode::CallTypeRequired,
            target.to_string(),
            binding.name,
        )
        .with_detail("borrowed form bindings require Before, After, or Override callType"));
    }
    if let Some(call_type) = binding.call_type {
        let parsed = FormCallType::from_xml(call_type).ok_or_else(|| {
            FormEventDiagnostic::new(
                FormEventDiagnosticCode::InvalidCallType,
                target.to_string(),
                binding.name,
            )
            .with_detail(format!(
                "callType '{call_type}' is invalid; expected Before, After, or Override"
            ))
        })?;

        match context.definition {
            FormDefinitionKind::Regular => {
                return Err(FormEventDiagnostic::new(
                    FormEventDiagnosticCode::CallTypeNotAllowed,
                    target.to_string(),
                    binding.name,
                )
                .with_detail(format!(
                    "callType '{}' is allowed only in extension forms",
                    parsed.as_str()
                )));
            }
            FormDefinitionKind::Extension => {}
        }
    }
    Ok(())
}

fn validate_object_event_context(
    context: &FormEventContext,
    target: FormEventTarget,
    event: &str,
) -> Result<(), FormEventDiagnostic> {
    match context.main_attribute {
        MainAttributeKind::PersistentObject | MainAttributeKind::PersistentRecord => Ok(()),
        MainAttributeKind::Unknown => {
            let detail = match context.main_attribute_provenance {
                MainAttributeProvenance::DirectForm => {
                    "direct Form MainAttribute has no readable type"
                }
                MainAttributeProvenance::DirectBaseForm => {
                    "direct BaseForm MainAttribute has no readable type"
                }
                MainAttributeProvenance::InheritedBaseFormUnavailable => {
                    "borrowed BaseForm main-attribute context is unavailable"
                }
                MainAttributeProvenance::Missing => {
                    "direct MainAttribute type was not found on Form"
                }
            };
            Err(FormEventDiagnostic::new(
                FormEventDiagnosticCode::ContextUnknown,
                target.to_string(),
                event,
            )
            .with_detail(detail))
        }
        MainAttributeKind::DynamicList | MainAttributeKind::Other => {
            let found = context.main_attribute_type.as_deref().unwrap_or("unknown");
            Err(FormEventDiagnostic::new(
                FormEventDiagnosticCode::EventNotAllowed,
                target.to_string(),
                event,
            )
            .with_detail(format!(
                "object/record form event requires a supported persistent main attribute; found '{found}'"
            )))
        }
    }
}

fn direct_logform_child<'a, 'input>(
    node: Node<'a, 'input>,
    local_name: &str,
) -> Option<Node<'a, 'input>> {
    node.children().find(|child| {
        child.is_element()
            && child.tag_name().name() == local_name
            && child.tag_name().namespace() == Some(FORM_LOGFORM_NS)
    })
}

fn direct_main_attribute<'a, 'input>(container: Node<'a, 'input>) -> Option<Node<'a, 'input>> {
    let attributes = direct_logform_child(container, "Attributes")?;
    attributes.children().find(|attribute| {
        attribute.is_element()
            && attribute.tag_name().name() == "Attribute"
            && attribute.tag_name().namespace() == Some(FORM_LOGFORM_NS)
            && direct_logform_child(*attribute, "MainAttribute")
                .and_then(|flag| flag.text())
                .is_some_and(|flag| flag.trim() == "true")
    })
}

fn main_attribute_type(main_attribute: Node<'_, '_>) -> Option<String> {
    let type_node = direct_logform_child(main_attribute, "Type")?;

    let v8_type_nodes = type_node
        .descendants()
        .skip(1)
        .filter(|node| {
            node.is_element()
                && node.tag_name().name() == "Type"
                && node.tag_name().namespace() == Some(FORM_V8_NS)
        })
        .collect::<Vec<_>>();
    if !v8_type_nodes.is_empty() {
        let v8_types = v8_type_nodes
            .into_iter()
            .map(trimmed_text)
            .collect::<Option<Vec<_>>>()?;
        return Some(v8_types.join("|"));
    }

    type_node
        .children()
        .filter(|node| node.is_text())
        .find_map(trimmed_text)
}

fn trimmed_text(node: Node<'_, '_>) -> Option<String> {
    node.text()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(ToOwned::to_owned)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use roxmltree::Document;

    const FORM_PREFIX: &str = r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform"
        xmlns:v8="http://v8.1c.ru/8.1/data/core">"#;

    fn context(xml: &str) -> FormEventContext {
        let document = Document::parse(xml).unwrap();
        context_from_root(document.root_element())
    }

    fn regular_context(main_attribute: MainAttributeKind) -> FormEventContext {
        FormEventContext {
            definition: FormDefinitionKind::Regular,
            direct_part_writable: true,
            main_attribute,
            main_attribute_type: None,
            main_attribute_provenance: MainAttributeProvenance::Missing,
            main_attribute_name: None,
            metadata_owner: None,
        }
    }

    fn extension_context(main_attribute: MainAttributeKind) -> FormEventContext {
        FormEventContext {
            definition: FormDefinitionKind::Extension,
            direct_part_writable: true,
            main_attribute,
            main_attribute_type: None,
            main_attribute_provenance: MainAttributeProvenance::InheritedBaseFormUnavailable,
            main_attribute_name: None,
            metadata_owner: None,
        }
    }

    fn form_event_catalog_8_3_27(owner: FormEventOwnerKind) -> Vec<&'static PlatformEventSpec> {
        super::form_event_catalog_8_3_27(&regular_context(MainAttributeKind::Other), owner, None)
    }

    fn projection_context(
        definition: FormDefinitionKind,
        main_attribute: MainAttributeKind,
        main_attribute_type: Option<&str>,
        metadata_owner: NodeKind,
    ) -> FormEventContext {
        FormEventContext {
            definition,
            direct_part_writable: true,
            main_attribute,
            main_attribute_type: main_attribute_type.map(str::to_string),
            main_attribute_provenance: MainAttributeProvenance::DirectForm,
            main_attribute_name: Some("Object".to_string()),
            metadata_owner: Some(metadata_owner),
        }
    }

    #[test]
    fn discovers_root_persistent_object_main_attribute() {
        let xml = format!(
            r#"{FORM_PREFIX}
                <Attributes>
                    <Attribute name="Object" id="1">
                        <Type><v8:Type>cfg:BusinessProcessObject.Task</v8:Type></Type>
                        <MainAttribute>true</MainAttribute>
                    </Attribute>
                </Attributes>
            </Form>"#
        );

        let actual = context(&xml);

        assert_eq!(actual.definition, FormDefinitionKind::Regular);
        assert_eq!(actual.main_attribute, MainAttributeKind::PersistentObject);
        assert_eq!(
            actual.main_attribute_provenance,
            MainAttributeProvenance::DirectForm
        );
        assert_eq!(
            actual.main_attribute_type.as_deref(),
            Some("cfg:BusinessProcessObject.Task")
        );
    }

    #[test]
    fn root_main_attribute_has_priority_over_base_form() {
        let xml = format!(
            r#"{FORM_PREFIX}
                <Attributes>
                    <Attribute name="List" id="1">
                        <Type><v8:Type>cfg:DynamicList</v8:Type></Type>
                        <MainAttribute>true</MainAttribute>
                    </Attribute>
                </Attributes>
                <BaseForm>
                    <Attributes>
                        <Attribute name="Object" id="1">
                            <Type><v8:Type>cfg:CatalogObject.Products</v8:Type></Type>
                            <MainAttribute>true</MainAttribute>
                        </Attribute>
                    </Attributes>
                </BaseForm>
            </Form>"#
        );

        let actual = context(&xml);

        assert_eq!(actual.definition, FormDefinitionKind::Extension);
        assert_eq!(actual.main_attribute, MainAttributeKind::DynamicList);
        assert_eq!(
            actual.main_attribute_provenance,
            MainAttributeProvenance::DirectForm
        );
        assert_eq!(
            actual.main_attribute_type.as_deref(),
            Some("cfg:DynamicList")
        );
    }

    #[test]
    fn falls_back_to_direct_base_form_main_attribute() {
        let xml = format!(
            r#"{FORM_PREFIX}
                <BaseForm>
                    <Attributes>
                        <Attribute name="Record" id="1">
                            <Type>
                                <v8:Type>cfg:InformationRegisterRecordManager.Prices</v8:Type>
                            </Type>
                            <MainAttribute>true</MainAttribute>
                        </Attribute>
                    </Attributes>
                </BaseForm>
            </Form>"#
        );

        let actual = context(&xml);

        assert_eq!(actual.definition, FormDefinitionKind::Extension);
        assert_eq!(actual.main_attribute, MainAttributeKind::PersistentRecord);
        assert_eq!(
            actual.main_attribute_provenance,
            MainAttributeProvenance::DirectBaseForm
        );
    }

    #[test]
    fn malformed_direct_main_attribute_overrides_valid_base_form_context() {
        let xml = format!(
            r#"{FORM_PREFIX}
                <Attributes>
                    <Attribute name="Object" id="1">
                        <Type/>
                        <MainAttribute>true</MainAttribute>
                    </Attribute>
                </Attributes>
                <BaseForm>
                    <Attributes>
                        <Attribute name="BaseObject" id="1">
                            <Type><v8:Type>cfg:CatalogObject.Products</v8:Type></Type>
                            <MainAttribute>true</MainAttribute>
                        </Attribute>
                    </Attributes>
                </BaseForm>
            </Form>"#
        );

        let actual = context(&xml);

        assert_eq!(actual.definition, FormDefinitionKind::Extension);
        assert_eq!(actual.main_attribute, MainAttributeKind::Unknown);
        assert_eq!(actual.main_attribute_type, None);
        assert_eq!(
            actual.main_attribute_provenance,
            MainAttributeProvenance::DirectForm
        );
    }

    #[test]
    fn distinguishes_unavailable_borrowed_context_from_malformed_base_context() {
        let unavailable = context(&format!(r#"{FORM_PREFIX}<BaseForm/></Form>"#));
        assert_eq!(unavailable.main_attribute, MainAttributeKind::Unknown);
        assert_eq!(
            unavailable.main_attribute_provenance,
            MainAttributeProvenance::InheritedBaseFormUnavailable
        );

        let malformed = context(&format!(
            r#"{FORM_PREFIX}
                <BaseForm>
                    <Attributes>
                        <Attribute name="BaseObject" id="1">
                            <Type/>
                            <MainAttribute>true</MainAttribute>
                        </Attribute>
                    </Attributes>
                </BaseForm>
            </Form>"#
        ));
        assert_eq!(malformed.main_attribute, MainAttributeKind::Unknown);
        assert_eq!(
            malformed.main_attribute_provenance,
            MainAttributeProvenance::DirectBaseForm
        );
    }

    #[test]
    fn multiple_v8_types_are_not_classified_as_a_persistent_main_type() {
        let xml = format!(
            r#"{FORM_PREFIX}
                <Attributes>
                    <Attribute name="Object" id="1">
                        <Type>
                            <v8:Type>cfg:CatalogObject.Products</v8:Type>
                            <v8:Type>xs:string</v8:Type>
                        </Type>
                        <MainAttribute>true</MainAttribute>
                    </Attribute>
                </Attributes>
            </Form>"#
        );

        let actual = context(&xml);

        assert_eq!(actual.main_attribute, MainAttributeKind::Other);
        assert_eq!(
            actual.main_attribute_type.as_deref(),
            Some("cfg:CatalogObject.Products|xs:string")
        );
        assert_eq!(
            actual.main_attribute_provenance,
            MainAttributeProvenance::DirectForm
        );

        let mixed_empty = context(&format!(
            r#"{FORM_PREFIX}
                <Attributes>
                    <Attribute name="Object" id="1">
                        <Type>
                            <v8:Type>cfg:CatalogObject.Products</v8:Type>
                            <v8:Type/>
                        </Type>
                        <MainAttribute>true</MainAttribute>
                    </Attribute>
                </Attributes>
            </Form>"#
        ));
        assert_eq!(mixed_empty.main_attribute, MainAttributeKind::Unknown);
        assert_eq!(mixed_empty.main_attribute_type, None);
        assert_eq!(
            mixed_empty.main_attribute_provenance,
            MainAttributeProvenance::DirectForm
        );
    }

    #[test]
    fn ignores_wrong_namespace_and_nested_main_attribute_traps() {
        let xml = format!(
            r#"{FORM_PREFIX}
                <Attributes xmlns="urn:not-logform">
                    <Attribute>
                        <Type><v8:Type>cfg:CatalogObject.Trap</v8:Type></Type>
                        <MainAttribute>true</MainAttribute>
                    </Attribute>
                </Attributes>
                <ChildItems>
                    <InputField name="Trap" id="1">
                        <Attributes>
                            <Attribute>
                                <Type><v8:Type>cfg:CatalogObject.NestedTrap</v8:Type></Type>
                                <MainAttribute>true</MainAttribute>
                            </Attribute>
                        </Attributes>
                    </InputField>
                </ChildItems>
            </Form>"#
        );

        let actual = context(&xml);

        assert_eq!(actual.main_attribute, MainAttributeKind::Unknown);
        assert_eq!(actual.main_attribute_type, None);
        assert_eq!(
            actual.main_attribute_provenance,
            MainAttributeProvenance::Missing
        );
    }

    #[test]
    fn classifies_known_main_attribute_families() {
        for persistent_object in [
            "cfg:CatalogObject.Goods",
            "cfg:DocumentObject.Order",
            "cfg:BusinessProcessObject.Approval",
            "cfg:TaskObject.Review",
            "cfg:ExchangePlanObject.Sync",
            "cfg:ChartOfAccountsObject.Main",
            "cfg:ChartOfCharacteristicTypesObject.Properties",
            "cfg:ChartOfCalculationTypesObject.Payroll",
        ] {
            assert_eq!(
                MainAttributeKind::from_type_name(persistent_object),
                MainAttributeKind::PersistentObject,
                "{persistent_object} must support persistent object form events"
            );
        }
        assert_eq!(
            MainAttributeKind::from_type_name("cfg:ConstantsSet"),
            MainAttributeKind::PersistentObject
        );
        for persistent_record in [
            "cfg:InformationRegisterRecordManager.Prices",
            "cfg:InformationRegisterRecordSet.Prices",
            "cfg:AccumulationRegisterRecordSet.Stock",
            "cfg:AccountingRegisterRecordSet.Accounting",
            "cfg:CalculationRegisterRecordSet.Payroll",
        ] {
            assert_eq!(
                MainAttributeKind::from_type_name(persistent_record),
                MainAttributeKind::PersistentRecord,
                "{persistent_record} must support persistent record form events"
            );
        }
        assert_eq!(
            MainAttributeKind::from_type_name("cfg:DynamicList"),
            MainAttributeKind::DynamicList
        );
        assert_eq!(
            MainAttributeKind::from_type_name("cfg:DataProcessorObject.Import"),
            MainAttributeKind::Other
        );
        for malformed in [
            "cfg:ConstantsSet.ApplicationSettings",
            "cfg:CatalogObject",
            "cfg:CatalogObject.Goods|string",
            "cfg:CatalogObject.Goods+string",
            "cfg:CatalogObject.Goods.Extra",
            "cfg:CatalogObject.Goods Name",
            "cfg:CatalogObject.Goods,Other",
            "cfg:CatalogObject.Goods/Other",
            "cfg:CatalogObject.Goods#x",
            "cfg:CatalogObject.123",
        ] {
            assert_eq!(
                MainAttributeKind::from_type_name(malformed),
                MainAttributeKind::Other,
                "{malformed} must not enter the persistent event whitelist"
            );
        }
        for unsupported in [
            "cfg:ReportObject.Sales",
            "cfg:DataProcessorObject.Import",
            "cfg:ExternalReportObject.Sales",
            "cfg:ExternalDataProcessorObject.Import",
        ] {
            assert_eq!(
                MainAttributeKind::from_type_name(unsupported),
                MainAttributeKind::Other,
                "{unsupported} must not enter the persistent event whitelist"
            );
        }
        assert_eq!(
            MainAttributeKind::from_type_name("  "),
            MainAttributeKind::Unknown
        );
    }

    #[test]
    fn on_read_accepts_persistent_object_and_record_contexts() {
        let binding = FormEventBinding::new("OnReadAtServer", "ObjectOnReadAtServer");

        assert!(validate_event(
            &regular_context(MainAttributeKind::PersistentObject),
            FormEventTarget::Form,
            &binding,
        )
        .is_ok());
        assert!(validate_event(
            &regular_context(MainAttributeKind::PersistentRecord),
            FormEventTarget::Form,
            &binding,
        )
        .is_ok());
    }

    #[test]
    fn on_read_rejects_known_nonpersistent_context() {
        let mut context = regular_context(MainAttributeKind::DynamicList);
        context.main_attribute_type = Some("cfg:DynamicList".to_string());
        let error = validate_event(
            &context,
            FormEventTarget::Form,
            &FormEventBinding::new("OnReadAtServer", "ListOnReadAtServer"),
        )
        .unwrap_err();

        assert_eq!(error.code, FormEventDiagnosticCode::EventNotAllowed);
        assert!(error.detail.contains("cfg:DynamicList"));
    }

    #[test]
    fn on_read_reports_unknown_context_separately() {
        let error = validate_event(
            &regular_context(MainAttributeKind::Unknown),
            FormEventTarget::Form,
            &FormEventBinding::new("OnReadAtServer", "ObjectOnReadAtServer"),
        )
        .unwrap_err();

        assert_eq!(error.code, FormEventDiagnosticCode::ContextUnknown);
    }

    #[test]
    fn all_object_record_events_are_context_gated() {
        for event in OBJECT_RECORD_FORM_EVENTS {
            assert!(validate_event(
                &regular_context(MainAttributeKind::PersistentObject),
                FormEventTarget::Form,
                &FormEventBinding::new(event, "ObjectEventHandler"),
            )
            .is_ok());

            let unknown_error = validate_event(
                &regular_context(MainAttributeKind::Unknown),
                FormEventTarget::Form,
                &FormEventBinding::new(event, "ObjectEventHandler"),
            )
            .unwrap_err();
            assert_eq!(
                unknown_error.code,
                FormEventDiagnosticCode::ContextUnknown,
                "{event} must report unknown context"
            );

            let unsupported_error = validate_event(
                &regular_context(MainAttributeKind::Other),
                FormEventTarget::Form,
                &FormEventBinding::new(event, "ObjectEventHandler"),
            )
            .unwrap_err();
            assert_eq!(
                unsupported_error.code,
                FormEventDiagnosticCode::EventNotAllowed,
                "{event} must reject a known unsupported context"
            );
        }
    }

    #[test]
    fn generic_write_processing_events_are_not_main_context_gated() {
        for event in ["NewWriteProcessing", "FillCheckProcessingAtServer"] {
            for context in [
                regular_context(MainAttributeKind::DynamicList),
                regular_context(MainAttributeKind::Other),
                regular_context(MainAttributeKind::Unknown),
            ] {
                assert!(
                    validate_event(
                        &context,
                        FormEventTarget::Form,
                        &FormEventBinding::new(event, "GenericWriteHandler"),
                    )
                    .is_ok(),
                    "{event} must be valid without a persistent object/record main attribute"
                );
            }
        }
    }

    #[test]
    fn validates_root_event_union() {
        let context = regular_context(MainAttributeKind::Other);

        assert!(validate_event(
            &context,
            FormEventTarget::Form,
            &FormEventBinding::new("OnCreateAtServer", "FormOnCreateAtServer"),
        )
        .is_ok());
        assert!(validate_event(
            &context,
            FormEventTarget::Form,
            &FormEventBinding::new(
                "OnMainServerAvailabilityChange",
                "FormOnMainServerAvailabilityChange",
            ),
        )
        .is_ok());
        assert!(validate_event(
            &context,
            FormEventTarget::Form,
            &FormEventBinding::new("URLListGetProcessing", "FormURLListGetProcessing"),
        )
        .is_ok());
        let error = validate_event(
            &context,
            FormEventTarget::Form,
            &FormEventBinding::new("Opening", "FormOpening"),
        )
        .unwrap_err();
        assert_eq!(error.code, FormEventDiagnosticCode::EventNotAllowed);
    }

    #[test]
    fn opening_is_allowed_only_for_input_field() {
        let context = regular_context(MainAttributeKind::Other);
        let binding = FormEventBinding::new("Opening", "FieldOpening");

        assert!(validate_event(
            &context,
            FormEventTarget::Element(FormElementKind::InputField),
            &binding,
        )
        .is_ok());
        let error = validate_event(
            &context,
            FormEventTarget::Element(FormElementKind::LabelField),
            &binding,
        )
        .unwrap_err();
        assert_eq!(error.code, FormEventDiagnosticCode::EventNotAllowed);
    }

    #[test]
    fn current_page_change_is_allowed_only_for_pages() {
        let context = regular_context(MainAttributeKind::Other);
        let binding = FormEventBinding::new("OnCurrentPageChange", "PagesOnChange");

        assert!(validate_event(
            &context,
            FormEventTarget::Element(FormElementKind::Pages),
            &binding,
        )
        .is_ok());
        let error = validate_event(
            &context,
            FormEventTarget::Element(FormElementKind::Page),
            &binding,
        )
        .unwrap_err();
        assert_eq!(error.code, FormEventDiagnosticCode::EventNotAllowed);
    }

    #[test]
    fn maps_xml_tags_and_dsl_keys_to_the_same_matrix() {
        let cases = [
            ("InputField", "input", FormElementKind::InputField),
            ("CheckBoxField", "check", FormElementKind::CheckBoxField),
            ("TrackBarField", "trackBar", FormElementKind::TrackBarField),
            ("Table", "table", FormElementKind::Table),
            ("Pages", "pages", FormElementKind::Pages),
            (
                "PictureDecoration",
                "picture",
                FormElementKind::PictureDecoration,
            ),
            (
                "ExtendedTooltip",
                "extendedTooltip",
                FormElementKind::ExtendedTooltip,
            ),
            (
                "FormattedDocumentField",
                "formattedDoc",
                FormElementKind::FormattedDocumentField,
            ),
            (
                "SpreadSheetDocumentField",
                "spreadsheet",
                FormElementKind::SpreadsheetDocumentField,
            ),
            ("UsualGroup", "group", FormElementKind::Group),
        ];

        for (xml_tag, dsl_key, expected) in cases {
            assert_eq!(FormElementKind::from_xml_tag(xml_tag), Some(expected));
            assert_eq!(FormElementKind::from_dsl_key(dsl_key), Some(expected));
        }
        assert_eq!(FormElementKind::from_xml_tag("Popup"), None);
        assert_eq!(FormElementKind::from_xml_tag("UnknownElement"), None);
        assert_eq!(FormElementKind::from_dsl_key("unknown"), None);
    }

    #[test]
    fn uses_platform_audited_element_event_matrix() {
        assert_eq!(FormElementKind::Button.allowed_events(), NO_EVENTS);
        assert_eq!(
            FormElementKind::LabelField.allowed_events(),
            &["URLProcessing", "Click", "OnChange"]
        );
        assert_eq!(FormElementKind::PictureField.allowed_events(), &["Click"]);
        assert_eq!(
            FormElementKind::CalendarField.allowed_events(),
            &["Selection", "OnChange", "OnPeriodOutput"]
        );
        assert_eq!(
            FormElementKind::HtmlDocumentField.allowed_events(),
            &["OnClick", "DocumentComplete"]
        );
        assert_eq!(
            FormElementKind::SpreadsheetDocumentField.allowed_events(),
            &[
                "DetailProcessing",
                "Selection",
                "OnActivate",
                "AdditionalDetailProcessing",
                "OnChange",
                "Drag",
                "URLProcessing",
                "BeforePrint",
                "BeforeWrite",
                "DragCheck",
                "OnChangeAreaContent",
            ]
        );

        assert!(!FormElementKind::InputField
            .allowed_events()
            .contains(&"Click"));
        assert!(!FormElementKind::Table.allowed_events().contains(&"Drop"));

        let button_error = validate_event(
            &regular_context(MainAttributeKind::Other),
            FormEventTarget::Element(FormElementKind::Button),
            &FormEventBinding::new("Click", "ButtonClick"),
        )
        .unwrap_err();
        assert_eq!(button_error.code, FormEventDiagnosticCode::EventNotAllowed);
    }

    #[test]
    fn form_compile_skill_documents_registered_events_for_each_documented_element() {
        const SKILL: &str =
            include_str!("../../../../../plugins/unica/skills/form-compile/SKILL.md");
        const START: &str = "<!-- form-event-registry:start -->";
        const END: &str = "<!-- form-event-registry:end -->";

        let section = SKILL
            .split_once(START)
            .and_then(|(_, tail)| tail.split_once(END).map(|(section, _)| section))
            .expect("form-compile event table must be delimited for parity checks");
        let cases = [
            ("input", FormElementKind::InputField),
            ("check", FormElementKind::CheckBoxField),
            ("labelField", FormElementKind::LabelField),
            ("table", FormElementKind::Table),
            ("pages", FormElementKind::Pages),
            ("page", FormElementKind::Page),
            ("button", FormElementKind::Button),
            ("cmdBar", FormElementKind::CommandBar),
            ("autoCmdBar", FormElementKind::CommandBar),
            ("group", FormElementKind::Group),
        ];

        let documented_keys = section
            .lines()
            .filter_map(|line| {
                line.strip_prefix("| `")
                    .and_then(|line| line.split_once("` | "))
                    .map(|(key, _)| key)
            })
            .collect::<Vec<_>>();
        assert_eq!(
            documented_keys,
            cases.iter().map(|(key, _)| *key).collect::<Vec<_>>(),
            "the documented event table must cover exactly the documented form DSL elements"
        );

        for (key, kind) in cases {
            let expected = if kind.allowed_events().is_empty() {
                "—".to_string()
            } else {
                kind.allowed_events()
                    .iter()
                    .map(|event| format!("`{event}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            let prefix = format!("| `{key}` | ");
            let documented = section
                .lines()
                .find_map(|line| line.strip_prefix(&prefix))
                .and_then(|value| value.strip_suffix(" |"))
                .expect("documented form element event row must be present");

            assert_eq!(documented, expected, "event list for `{key}`");
        }
    }

    #[test]
    fn rejects_empty_handler() {
        let error = validate_event(
            &regular_context(MainAttributeKind::Other),
            FormEventTarget::Element(FormElementKind::LabelDecoration),
            &FormEventBinding::new("Click", "  \n"),
        )
        .unwrap_err();

        assert_eq!(error.code, FormEventDiagnosticCode::EmptyHandler);
    }

    #[test]
    fn rejects_call_type_in_regular_form() {
        let error = validate_event(
            &regular_context(MainAttributeKind::Other),
            FormEventTarget::Form,
            &FormEventBinding::new("OnOpen", "FormOnOpen").with_call_type("After"),
        )
        .unwrap_err();

        assert_eq!(error.code, FormEventDiagnosticCode::CallTypeNotAllowed);
    }

    #[test]
    fn accepts_all_call_types_in_extension_form() {
        let context = extension_context(MainAttributeKind::Other);

        for call_type in ["Before", "After", "Override"] {
            assert!(validate_event(
                &context,
                FormEventTarget::Form,
                &FormEventBinding::new("OnOpen", "FormOnOpen").with_call_type(call_type),
            )
            .is_ok());
        }
    }

    #[test]
    fn extension_form_binding_requires_an_explicit_call_type() {
        let error = validate_event_call_type(
            &extension_context(MainAttributeKind::Other),
            FormEventTarget::Form,
            &FormEventBinding::new("OnOpen", "FormOnOpen"),
        )
        .unwrap_err();

        assert_eq!(error.code.as_str(), "FORM_EVENT_CALL_TYPE_REQUIRED");
    }

    #[test]
    fn rejects_invalid_call_type_in_extension_form() {
        for invalid in ["Instead", "after", ""] {
            let error = validate_event(
                &extension_context(MainAttributeKind::Other),
                FormEventTarget::Form,
                &FormEventBinding::new("OnOpen", "FormOnOpen").with_call_type(invalid),
            )
            .unwrap_err();

            assert_eq!(
                error.code,
                FormEventDiagnosticCode::InvalidCallType,
                "'{invalid}' must be rejected"
            );
        }

        let error = validate_event(
            &extension_context(MainAttributeKind::Unknown),
            FormEventTarget::Form,
            &FormEventBinding::new("OnReadAtServer", "FormOnReadAtServer").with_call_type("after"),
        )
        .unwrap_err();
        assert_eq!(error.code, FormEventDiagnosticCode::InvalidCallType);
    }

    #[test]
    fn diagnostic_codes_and_display_are_stable() {
        let codes = [
            (
                FormEventDiagnosticCode::EventNotAllowed,
                "FORM_EVENT_NOT_ALLOWED",
            ),
            (
                FormEventDiagnosticCode::ContextUnknown,
                "FORM_EVENT_CONTEXT_UNKNOWN",
            ),
            (
                FormEventDiagnosticCode::EmptyHandler,
                "FORM_EVENT_EMPTY_HANDLER",
            ),
            (FormEventDiagnosticCode::Duplicate, "FORM_EVENT_DUPLICATE"),
            (
                FormEventDiagnosticCode::BindingConflict,
                "FORM_EVENT_BINDING_CONFLICT",
            ),
            (
                FormEventDiagnosticCode::TargetNotFound,
                "FORM_EVENT_TARGET_NOT_FOUND",
            ),
            (
                FormEventDiagnosticCode::InvalidCallType,
                "FORM_EVENT_INVALID_CALL_TYPE",
            ),
            (
                FormEventDiagnosticCode::CallTypeRequired,
                "FORM_EVENT_CALL_TYPE_REQUIRED",
            ),
            (
                FormEventDiagnosticCode::CallTypeNotAllowed,
                "FORM_EVENT_CALL_TYPE_NOT_ALLOWED",
            ),
        ];

        for (code, expected) in codes {
            assert_eq!(code.as_str(), expected);
        }

        let diagnostic =
            FormEventDiagnostic::new(FormEventDiagnosticCode::EventNotAllowed, "form", "Opening")
                .with_detail("event is not present in the target event matrix");
        assert_eq!(
            diagnostic.to_string(),
            "[FORM_EVENT_NOT_ALLOWED] event 'Opening' on form: event is not present in the target event matrix"
        );
    }

    #[test]
    fn module_event_applicability_covers_every_approved_role_family() {
        use crate::domain::address::QualifiedAddress;
        use crate::domain::platform_profile::PlatformProfile;

        let cases: [(&str, &[&str]); 16] = [
            (
                "main:Document.Заказ.Module.Object",
                &[
                    "BeforeDelete",
                    "BeforeWrite",
                    "FillCheckProcessing",
                    "Filling",
                    "GenerateFromDataHistoryVersionProcessing",
                    "OnCopy",
                    "OnSetNewNumber",
                    "OnWrite",
                    "Posting",
                    "UndoPosting",
                ],
            ),
            (
                "main:Document.Заказ.Module.Manager",
                &[
                    "AfterWriteDataHistoryVersionsProcessing",
                    "ChoiceDataGetProcessing",
                    "FormGetProcessing",
                    "PresentationFieldsGetProcessing",
                    "PresentationGetProcessing",
                ],
            ),
            (
                "main:InformationRegister.Цены.Module.RecordSet",
                &[
                    "BeforeWrite",
                    "FillCheckProcessing",
                    "Filling",
                    "GenerateFromDataHistoryVersionProcessing",
                    "OnWrite",
                ],
            ),
            (
                "main:Constant.ОсновнаяВалюта.Module.ValueManager",
                &[
                    "BeforeWrite",
                    "FillCheckProcessing",
                    "GenerateFromDataHistoryVersionProcessing",
                    "OnWrite",
                ],
            ),
            ("main:CommonModule.ЗаказыСервер", &[]),
            ("main:Document.Заказ.Form.ФормаДокумента.Module.Form", &[]),
            (
                "main:Document.Заказ.Command.ПровестиИЗакрыть.Module.Command",
                &["CommandProcessing"],
            ),
            (
                "main:Module.ManagedApplication",
                &[
                    "AddInDetachmentOnErrorProcessing",
                    "AfterExchangeDataWithMainServer",
                    "BeforeExit",
                    "BeforeStart",
                    "CollaborationSystemExternalUserInvitationProcessing",
                    "CollaborationSystemMessageTemplateChoiceProcessing",
                    "CollaborationSystemUsersAutoComplete",
                    "CollaborationSystemUsersChoiceFormGetProcessing",
                    "ErrorDisplayProcessing",
                    "ExternEventProcessing",
                    "IncomingShareRequestCommandGenerateProcessing",
                    "NavigationByURLProcessing",
                    "OnChangeDisplaySettings",
                    "OnClientApplicationResume",
                    "OnClientApplicationSuspend",
                    "OnCollaborationSystemMessageActionChoice",
                    "OnCollaborationSystemMessageButtonPanelButtonClick",
                    "OnExit",
                    "OnGlobalSearch",
                    "OnGlobalSearchResultActionChoice",
                    "OnGlobalSearchResultChoice",
                    "OnMainServerAvailabilityChange",
                    "OnPasteFromClipboard",
                    "OnStart",
                ],
            ),
            (
                "main:Module.OrdinaryApplication",
                &[
                    "BeforeExit",
                    "BeforeStart",
                    "ExternEventProcessing",
                    "OnChangeDisplaySettings",
                    "OnExit",
                    "OnStart",
                ],
            ),
            ("main:Module.Session", &["SessionParametersSetting"]),
            ("main:Module.ExternalConnection", &["OnExit", "OnStart"]),
            ("main:HTTPService.API.Module.HTTPService", &[]),
            ("main:WebService.Обмен.Module.WebService", &[]),
            (
                "main:IntegrationService.Шина.Module.IntegrationService",
                &[],
            ),
            (
                "main:Bot.Помощник.Module.Bot",
                &[
                    "CollaborationSystemMessageButtonPanelButtonClickProcessing",
                    "CollaborationSystemMessageProcessing",
                    "OnAddToCollaborationSystemConversation",
                    "OnCreateCollaborationSystemConversation",
                    "OnDeleteFromCollaborationSystemConversation",
                ],
            ),
            (
                "main:WebSocketClient.Телефония.Module.WebSocketClient",
                &[
                    "BeforeConnect",
                    "OnCloseConnection",
                    "OnError",
                    "OnMessage",
                    "OnOpenConnection",
                ],
            ),
        ];
        let profile = PlatformProfile::v8_3_27();
        for (raw, expected) in cases {
            let at = QualifiedAddress::parse(raw).unwrap();
            let capability = profile.module_capability(&at).unwrap();
            let actual = module_event_catalog_8_3_27(capability)
                .iter()
                .map(|event| event.event_id.as_str())
                .collect::<Vec<_>>();
            assert_eq!(actual, expected, "{raw}");
        }
    }

    #[test]
    fn module_catalog_covers_the_task12_direct_owner_role_matrix_exactly() {
        use crate::domain::address::NodeKind;
        use crate::domain::platform_profile::{ModuleRole, PlatformProfile};

        let direct_roles = [
            ModuleRole::Object,
            ModuleRole::Manager,
            ModuleRole::RecordSet,
            ModuleRole::ValueManager,
        ];
        let profile = PlatformProfile::v8_3_27();
        let expected = NodeKind::metadata_kinds()
            .iter()
            .flat_map(|owner| direct_roles.iter().map(move |role| (*owner, *role)))
            .filter(|(owner, role)| profile.supports_direct_module_role(*owner, *role))
            .map(|(owner, role)| (owner.as_str(), role.as_str()))
            .collect::<std::collections::BTreeSet<_>>();
        let actual = platform_event_catalog_fixture()
            .module_catalogs
            .iter()
            .filter(|catalog| {
                direct_roles
                    .iter()
                    .any(|role| role.as_str() == catalog.module_role)
            })
            .map(|catalog| (catalog.owner_kind.as_str(), catalog.module_role.as_str()))
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(actual, expected);
    }

    #[test]
    fn form_event_applicability_preserves_every_logical_owner_family() {
        let owners = std::iter::once(FormEventOwnerKind::Form)
            .chain(std::iter::once(FormEventOwnerKind::Table))
            .chain(std::iter::once(FormEventOwnerKind::Command))
            .chain(
                FormElementKind::ALL
                    .into_iter()
                    .filter(|kind| *kind != FormElementKind::Table)
                    .map(FormEventOwnerKind::Element),
            )
            .chain(
                FormElementKind::ALL
                    .into_iter()
                    .filter(|kind| *kind != FormElementKind::Table)
                    .map(FormEventOwnerKind::Column),
            )
            .collect::<Vec<_>>();
        let mut observed_keys = std::collections::BTreeSet::new();
        for owner in owners {
            let key = owner.catalog_key();
            let expected = platform_event_catalog_fixture()
                .form_catalogs
                .iter()
                .find(|catalog| catalog.owner_kinds.contains(&key))
                .unwrap_or_else(|| panic!("missing checked catalog for {key}"));
            assert_eq!(
                form_event_catalog_8_3_27(owner)
                    .into_iter()
                    .map(|event| event.event_id.as_str())
                    .collect::<Vec<_>>(),
                expected
                    .events
                    .iter()
                    .map(|event| event.event_id.as_str())
                    .collect::<Vec<_>>(),
                "{key}"
            );
            observed_keys.insert(key);
        }
        let expected_keys = platform_event_catalog_fixture()
            .form_catalogs
            .iter()
            .flat_map(|catalog| catalog.owner_kinds.iter().cloned())
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(observed_keys, expected_keys);
    }

    #[test]
    fn form_applicability_variants_have_exact_closed_event_additions_and_counts() {
        let base_context = projection_context(
            FormDefinitionKind::Regular,
            MainAttributeKind::Other,
            None,
            NodeKind::CommonForm,
        );
        let base = super::form_event_catalog_8_3_27(&base_context, FormEventOwnerKind::Form, None)
            .into_iter()
            .map(|event| event.event_id.as_str())
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(base.len(), 26);

        let cases = [
            (
                "catalog-object",
                projection_context(
                    FormDefinitionKind::Regular,
                    MainAttributeKind::PersistentObject,
                    Some("cfg:CatalogObject.Items"),
                    NodeKind::Catalog,
                ),
                &[
                    "AfterWrite",
                    "AfterWriteAtServer",
                    "BeforeWrite",
                    "BeforeWriteAtServer",
                    "OnReadAtServer",
                    "OnWriteAtServer",
                    "ValueChoice",
                ][..],
            ),
            (
                "document-object",
                projection_context(
                    FormDefinitionKind::Regular,
                    MainAttributeKind::PersistentObject,
                    Some("cfg:DocumentObject.Order"),
                    NodeKind::Document,
                ),
                &[
                    "AfterWrite",
                    "AfterWriteAtServer",
                    "BeforeWrite",
                    "BeforeWriteAtServer",
                    "OnReadAtServer",
                    "OnWriteAtServer",
                    "ValueChoice",
                ][..],
            ),
            (
                "business-process-object",
                projection_context(
                    FormDefinitionKind::Regular,
                    MainAttributeKind::PersistentObject,
                    Some("cfg:BusinessProcessObject.Route"),
                    NodeKind::BusinessProcess,
                ),
                &[
                    "AfterWrite",
                    "AfterWriteAtServer",
                    "BeforeStart",
                    "BeforeWrite",
                    "BeforeWriteAtServer",
                    "OnReadAtServer",
                    "OnWriteAtServer",
                    "ValueChoice",
                ][..],
            ),
            (
                "task-object",
                projection_context(
                    FormDefinitionKind::Regular,
                    MainAttributeKind::PersistentObject,
                    Some("cfg:TaskObject.Task"),
                    NodeKind::Task,
                ),
                &[
                    "AfterWrite",
                    "AfterWriteAtServer",
                    "BeforeExecute",
                    "BeforeWrite",
                    "BeforeWriteAtServer",
                    "OnReadAtServer",
                    "OnWriteAtServer",
                    "ValueChoice",
                ][..],
            ),
            (
                "characteristic-object",
                projection_context(
                    FormDefinitionKind::Regular,
                    MainAttributeKind::PersistentObject,
                    Some("cfg:ChartOfCharacteristicTypesObject.Kinds"),
                    NodeKind::ChartOfCharacteristicTypes,
                ),
                &[
                    "AfterWrite",
                    "AfterWriteAtServer",
                    "BeforeWrite",
                    "BeforeWriteAtServer",
                    "OnReadAtServer",
                    "OnWriteAtServer",
                    "ValueChoice",
                ][..],
            ),
            (
                "constant-set",
                projection_context(
                    FormDefinitionKind::Regular,
                    MainAttributeKind::PersistentObject,
                    Some("cfg:ConstantsSet"),
                    NodeKind::Constant,
                ),
                &[
                    "AfterWrite",
                    "AfterWriteAtServer",
                    "BeforeWrite",
                    "BeforeWriteAtServer",
                    "OnReadAtServer",
                    "OnWriteAtServer",
                ][..],
            ),
            (
                "information-register-record",
                projection_context(
                    FormDefinitionKind::Regular,
                    MainAttributeKind::PersistentRecord,
                    Some("cfg:InformationRegisterRecordManager.Prices"),
                    NodeKind::InformationRegister,
                ),
                &[
                    "AfterWrite",
                    "AfterWriteAtServer",
                    "BeforeWrite",
                    "BeforeWriteAtServer",
                    "OnReadAtServer",
                    "OnWriteAtServer",
                ][..],
            ),
            (
                "record-set",
                projection_context(
                    FormDefinitionKind::Regular,
                    MainAttributeKind::PersistentRecord,
                    Some("cfg:AccumulationRegisterRecordSet.Stock"),
                    NodeKind::AccumulationRegister,
                ),
                &[
                    "AfterWrite",
                    "AfterWriteAtServer",
                    "BeforeWrite",
                    "BeforeWriteAtServer",
                    "OnReadAtServer",
                    "OnWriteAtServer",
                ][..],
            ),
            (
                "report",
                projection_context(
                    FormDefinitionKind::Regular,
                    MainAttributeKind::Other,
                    Some("cfg:ReportObject.Sales"),
                    NodeKind::Report,
                ),
                &[
                    "BeforeLoadUserSettingsAtServer",
                    "BeforeLoadVariantAtServer",
                    "OnLoadUserSettingsAtServer",
                    "OnLoadVariantAtServer",
                    "OnSaveUserSettingsAtServer",
                    "OnSaveVariantAtServer",
                    "OnUpdateUserSettingSetAtServer",
                ][..],
            ),
        ];
        for (case, context, expected_additions) in cases {
            let actual = super::form_event_catalog_8_3_27(&context, FormEventOwnerKind::Form, None)
                .into_iter()
                .map(|event| event.event_id.as_str())
                .collect::<std::collections::BTreeSet<_>>();
            let additions = actual.difference(&base).copied().collect::<Vec<_>>();
            assert_eq!(additions, expected_additions, "{case}");
            assert_eq!(
                actual.len(),
                base.len() + expected_additions.len(),
                "{case}"
            );
        }

        let extension_document = projection_context(
            FormDefinitionKind::Extension,
            MainAttributeKind::PersistentObject,
            Some("cfg:DocumentObject.Order"),
            NodeKind::Document,
        );
        let extension_ids =
            super::form_event_catalog_8_3_27(&extension_document, FormEventOwnerKind::Form, None)
                .into_iter()
                .map(|event| event.event_id.as_str())
                .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(extension_ids.len(), 33);

        let mut dynamic = projection_context(
            FormDefinitionKind::Regular,
            MainAttributeKind::DynamicList,
            Some("cfg:DynamicList"),
            NodeKind::Catalog,
        );
        dynamic.main_attribute_name = Some("List".to_string());
        let ordinary_table = super::form_event_catalog_8_3_27(
            &dynamic,
            FormEventOwnerKind::Table,
            Some("OtherRows"),
        );
        let dynamic_table =
            super::form_event_catalog_8_3_27(&dynamic, FormEventOwnerKind::Table, Some("List"));
        let inherited_dynamic_table =
            super::form_event_catalog_8_3_27(&dynamic, FormEventOwnerKind::Table, Some("~List"));
        assert_eq!(ordinary_table.len(), 23);
        assert_eq!(dynamic_table.len(), 30);
        assert_eq!(inherited_dynamic_table.len(), 30);
        let ordinary_ids = ordinary_table
            .into_iter()
            .map(|event| event.event_id.as_str())
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            dynamic_table
                .into_iter()
                .map(|event| event.event_id.as_str())
                .filter(|event| !ordinary_ids.contains(event))
                .collect::<Vec<_>>(),
            [
                "BeforeLoadUserSettingsAtServer",
                "OnGetDataAtServer",
                "OnLoadUserSettingsAtServer",
                "OnSaveUserSettingsAtServer",
                "OnUpdateUserSettingSetAtServer",
                "URLGetProcessing",
                "URLListGetProcessing",
            ]
        );
    }

    #[test]
    fn event_catalog_entries_have_exact_bilingual_shape_context_and_provenance() {
        let fixture = platform_event_catalog_fixture();
        for events in fixture
            .module_catalogs
            .iter()
            .map(|catalog| catalog.events.as_slice())
            .chain(
                fixture
                    .form_catalogs
                    .iter()
                    .map(|catalog| catalog.events.as_slice()),
            )
        {
            for event in events {
                let expected_head = if event.method_kind == "function" {
                    ("Функция ", "Function ")
                } else {
                    ("Процедура ", "Procedure ")
                };
                assert!(event.signature_ru.starts_with(expected_head.0));
                assert!(event.signature_en.starts_with(expected_head.1));
                assert!(event.signature_ru.contains(&event.handler_ru));
                assert!(event.signature_en.contains(&event.handler_en));
                assert!(!event.contexts.is_empty());
                assert!(event
                    .source_page_id
                    .starts_with("platform-syntax-help:syntax-context:"));
            }
        }
    }

    #[test]
    fn every_catalog_has_unique_semantic_event_ids_not_generic_storage_names() {
        let fixture = platform_event_catalog_fixture();
        for (catalog, events) in fixture
            .module_catalogs
            .iter()
            .map(|catalog| {
                (
                    format!("{} × {}", catalog.owner_kind, catalog.module_role),
                    catalog.events.as_slice(),
                )
            })
            .chain(fixture.form_catalogs.iter().map(|catalog| {
                (
                    format!("{:?}", catalog.owner_kinds),
                    catalog.events.as_slice(),
                )
            }))
        {
            let ids = events
                .iter()
                .map(|event| event.event_id.as_str())
                .collect::<Vec<_>>();
            let unique = ids
                .iter()
                .copied()
                .collect::<std::collections::BTreeSet<_>>();
            assert_eq!(unique.len(), ids.len(), "{catalog}: {ids:?}");
            assert!(
                !ids.contains(&"event"),
                "{catalog}: storage filename is not a semantic event id"
            );
        }
    }

    #[test]
    fn form_catalog_execution_contexts_distinguish_client_and_server_callbacks() {
        let form = form_event_catalog_8_3_27(FormEventOwnerKind::Form);
        let on_open = form
            .iter()
            .find(|event| event.event_id == "OnOpen")
            .unwrap();
        assert_eq!(
            on_open.contexts,
            [
                "thinClient",
                "webClient",
                "thickClientManaged",
                "mobileClient",
                "mobileAppClient",
            ]
        );
        let create_at_server = form
            .iter()
            .find(|event| event.event_id == "OnCreateAtServer")
            .unwrap();
        assert_eq!(create_at_server.contexts, ["server"]);
        let field =
            form_event_catalog_8_3_27(FormEventOwnerKind::Element(FormElementKind::InputField));
        assert_eq!(
            field
                .iter()
                .find(|event| event.event_id == "OnChange")
                .unwrap()
                .contexts,
            [
                "thinClient",
                "webClient",
                "thickClientManaged",
                "mobileClient",
                "mobileAppClient",
            ]
        );
    }

    #[test]
    fn checked_event_catalog_is_a_closed_immutable_8_3_27_set() {
        use sha2::{Digest, Sha256};

        let bytes = include_bytes!("../platform-event-catalog-8.3.27.2074.json");
        let digest = Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        assert_eq!(
            digest,
            "4f3b22f55276e7146f77db717e56684f950df11cadccbce2df66da39aa33bf74"
        );
        let fixture = platform_event_catalog_fixture();
        assert_eq!(fixture.profile, "8.3.27");
        assert_eq!(fixture.source.installation_version, "8.3.27.2074");
        assert_eq!(fixture.module_catalogs.len(), 48);
        assert_eq!(fixture.form_catalogs.len(), 31);
        assert_eq!(
            fixture
                .module_catalogs
                .iter()
                .map(|catalog| catalog.events.len())
                .sum::<usize>(),
            205
        );
        assert_eq!(
            fixture
                .form_catalogs
                .iter()
                .map(|catalog| catalog.events.len())
                .sum::<usize>(),
            199
        );
        assert_eq!(fixture.excluded_structural_pages.len(), 4);
        assert_eq!(fixture.excluded_external_data_source_pages.len(), 13);
        assert_eq!(fixture.excluded_generic_template_pages.len(), 5);
        assert_eq!(fixture.excluded_out_of_profile_pages.len(), 269);
        assert_eq!(fixture.source.english_container, "shcntx_root.hbk");
        assert_eq!(
            fixture.source.english_sha256,
            "0113af559ef2001dedbbd9e6bbfcf8bc02e3241ca715f95c8a34c4f24c962421"
        );
    }

    #[test]
    fn checked_event_fixture_is_non_skipping_closed_partition_evidence() {
        let fixture = platform_event_catalog_fixture();
        let mut memberships = std::collections::BTreeMap::<&str, Vec<&str>>::new();
        let mut selected_records = std::collections::BTreeMap::<&str, usize>::new();
        for event in fixture
            .module_catalogs
            .iter()
            .flat_map(|catalog| catalog.events.iter())
            .chain(
                fixture
                    .form_catalogs
                    .iter()
                    .flat_map(|catalog| catalog.events.iter()),
            )
        {
            *selected_records
                .entry(event.source_page_id.as_str())
                .or_default() += 1;
            memberships
                .entry(event.source_page_id.as_str())
                .or_default()
                .push("selected");
        }
        for (category, pages) in [
            ("structural", fixture.excluded_structural_pages.as_slice()),
            (
                "externalDataSourceDeferred",
                fixture.excluded_external_data_source_pages.as_slice(),
            ),
            (
                "genericTemplate",
                fixture.excluded_generic_template_pages.as_slice(),
            ),
            (
                "reviewedOutOfProfile",
                fixture.excluded_out_of_profile_pages.as_slice(),
            ),
        ] {
            for page in pages {
                memberships
                    .entry(page.page_id.as_str())
                    .or_default()
                    .push(category);
            }
        }
        let duplicate_categories = memberships
            .iter()
            .filter(|(_, categories)| {
                categories
                    .iter()
                    .copied()
                    .collect::<std::collections::BTreeSet<_>>()
                    .len()
                    != 1
            })
            .collect::<Vec<_>>();
        assert!(
            duplicate_categories.is_empty(),
            "page IDs must belong to exactly one partition category: {duplicate_categories:#?}"
        );
        assert_eq!(memberships.len(), fixture.source.event_markup_page_count);
        assert_eq!(fixture.source.event_markup_page_count, 693);
        assert_eq!(fixture.source.signature_event_leaf_count, 689);
        assert_eq!(fixture.excluded_structural_pages.len(), 4);
        assert_eq!(fixture.excluded_external_data_source_pages.len(), 13);
        assert_eq!(fixture.excluded_generic_template_pages.len(), 5);
        assert_eq!(fixture.excluded_out_of_profile_pages.len(), 269);
        assert_eq!(
            fixture.source.event_page_ids_sha256,
            "19ad908ce8af0ba191dc642545d70ec163d402f129c34cd9f3c2d68318a09a31"
        );
        assert_eq!(selected_records.len(), 402, "unique selected vendor pages");
        assert_eq!(
            selected_records.values().sum::<usize>(),
            404,
            "selected projection records"
        );
        let repeated = selected_records
            .into_iter()
            .filter(|(_, count)| *count > 1)
            .collect::<std::collections::BTreeMap<_, _>>();
        assert_eq!(
            repeated,
            std::collections::BTreeMap::from([
                (
                    "platform-syntax-help:syntax-context:objects/catalog1649/Command module/events/CommandProcessing563.html",
                    2,
                ),
                (
                    "platform-syntax-help:syntax-context:objects/catalog1649/catalog1676/FormField/events/OnChange326.html",
                    2,
                ),
            ])
        );
        let approved_reasons = [
            "out-of-profile/ordinary-form: ordinary-form runtime owners are outside the approved managed-form projection",
            "out-of-profile/managed-form-element: the element kind is absent from the approved v0.13 form owner taxonomy",
            "out-of-profile/external-data-source-module: ExternalDataSource has no approved v0.13 address/profile owner",
            "out-of-profile/non-module-owner: the platform event owner is not an approved module or managed-form owner",
            "out-of-profile/settings-composer: no approved source-derived form applicability fact identifies this owner",
        ];
        assert!(fixture
            .excluded_out_of_profile_pages
            .iter()
            .all(|page| approved_reasons.contains(&page.reason.as_str())));
        let reason_counts = fixture.excluded_out_of_profile_pages.iter().fold(
            std::collections::BTreeMap::<&str, usize>::new(),
            |mut counts, page| {
                *counts.entry(page.reason.as_str()).or_default() += 1;
                counts
            },
        );
        assert_eq!(
            reason_counts,
            std::collections::BTreeMap::from([
                (approved_reasons[0], 189),
                (approved_reasons[1], 40),
                (approved_reasons[2], 19),
                (approved_reasons[3], 20),
                (approved_reasons[4], 1),
            ])
        );
    }

    #[test]
    pub(crate) fn platform_8_3_27_module_event_catalog_is_role_specific() {
        module_event_applicability_covers_every_approved_role_family();
        module_catalog_covers_the_task12_direct_owner_role_matrix_exactly();
        form_event_applicability_preserves_every_logical_owner_family();
        form_applicability_variants_have_exact_closed_event_additions_and_counts();
        event_catalog_entries_have_exact_bilingual_shape_context_and_provenance();
        every_catalog_has_unique_semantic_event_ids_not_generic_storage_names();
        form_catalog_execution_contexts_distinguish_client_and_server_callbacks();
        checked_event_catalog_is_a_closed_immutable_8_3_27_set();
        checked_event_fixture_is_non_skipping_closed_partition_evidence();
    }
}
