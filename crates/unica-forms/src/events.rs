//! Registry and validation rules for managed-form event bindings.
//!
//! The registry deliberately keeps XML context discovery separate from form
//! mutation. Callers can validate a proposed binding before changing the
//! source document and reuse the same rules from `form.edit`, `form.compile`,
//! and `form.validate`.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::OnceLock;

use unica_address::address::NodeKind;
use unica_metadata::event::BindingFact;
use unica_metadata::event::FormCallType;

use unica_metadata::identifier::is_1c_identifier;

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

use unica_metadata::event_catalog::{platform_event_catalog_fixture, PlatformEventSpec};

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
pub enum FormDefinitionKind {
    Regular,
    Extension,
}

/// Relevant class of the form's direct main attribute.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MainAttributeKind {
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
pub enum MainAttributeProvenance {
    DirectForm,
    DirectBaseForm,
    InheritedBaseFormUnavailable,
    Missing,
}

impl MainAttributeKind {
    pub fn from_type_name(type_name: &str) -> Self {
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
pub struct FormEventContext {
    pub definition: FormDefinitionKind,
    /// Whether Part 1 contains any direct form node besides `BaseForm`.
    /// A literal BaseForm-only projection proves no directly owned insertion
    /// surface and must therefore fail closed for Available events.
    pub direct_part_writable: bool,
    pub main_attribute: MainAttributeKind,
    pub main_attribute_type: Option<String>,
    pub main_attribute_provenance: MainAttributeProvenance,
    pub main_attribute_name: Option<String>,
    pub metadata_owner: Option<NodeKind>,
}

/// Element categories used by the compact form DSL and their XML equivalents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FormElementKind {
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
    #[cfg(any(test, feature = "test-support"))]
    pub const ALL: [Self; 21] = [
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

    pub fn from_xml_tag(tag: &str) -> Option<Self> {
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

    #[cfg(any(test, feature = "test-support"))]
    pub fn from_dsl_key(key: &str) -> Option<Self> {
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

    pub const fn dsl_key(self) -> &'static str {
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

    pub const fn allowed_events(self) -> &'static [&'static str] {
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
pub enum FormEventTarget {
    Form,
    Element(FormElementKind),
}

impl FormEventTarget {
    pub const fn allowed_events(self) -> &'static [&'static str] {
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
pub enum FormEventOwnerKind {
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
pub fn form_event_catalog_8_3_27(
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

/// Table events whose handler does not receive the item first, although the
/// help is silent about it. Evidence from the 8.3.27 УТ dump: all 110
/// `ПриПолученииДанныхНаСервере` handlers take `(ИмяЭлемента, Настройки,
/// Строки)` and all 5 `ОбработкаЗаписиНового` table handlers take
/// `(НовыйОбъект, Источник, СтандартнаяОбработка)`. Every other item event,
/// including the server `ПриОбновленииСоставаПользовательскихНастроекНаСервере`
/// of a table (13 of 18 handlers declare `Элемент`), receives the item.
const TABLE_EVENTS_WITHOUT_ITEM: &[&str] = &["OnGetDataAtServer", "NewWriteProcessing"];

/// Parameter lists the platform passes that the help does not document.
/// Evidence from the 8.3.27 УТ dump: 103 `Нажатие` handlers of label
/// decorations declare `(Элемент, СтандартнаяОбработка)`.
const UNDOCUMENTED_FORMS: &[(&str, &str, &[&str])] = &[(
    "Element.LabelDecoration",
    "Click",
    &["Элемент", "СтандартнаяОбработка"],
)];

/// Known parameter lists of one form or item event under one owner, from the
/// checked 8.3.27 catalog. The first list follows
/// [`form_event_handler_parameters_8_3_27`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormEventParameterForms {
    pub event_id: String,
    /// Russian handler name of the event, as the help names it.
    pub handler_ru: String,
    /// The first entry is the help signature for this form; a later entry
    /// is an undocumented list proven by the dump.
    pub forms: Vec<Vec<String>>,
}

/// Every catalogued event of the owner with its known parameter lists. The
/// events are selected for this form exactly as the event catalog selects
/// them: by form kind, main attribute and a table's dynamic-list source.
pub fn form_owner_event_parameters_8_3_27(
    context: &FormEventContext,
    owner: FormEventOwnerKind,
    data_path: Option<&str>,
) -> Vec<FormEventParameterForms> {
    let key = owner.catalog_key();
    let undocumented_key = match owner {
        FormEventOwnerKind::Column(kind) => format!("Element.{}", kind.catalog_key()),
        _ => key,
    };
    form_event_catalog_8_3_27(context, owner, data_path)
        .into_iter()
        .filter_map(|spec| {
            let mut forms = vec![form_event_handler_parameters_8_3_27(owner, spec)?];
            forms.extend(
                UNDOCUMENTED_FORMS
                    .iter()
                    .filter(|(owner, event_id, _)| {
                        *owner == undocumented_key && *event_id == spec.event_id
                    })
                    .map(|(_, _, parameters)| {
                        parameters.iter().map(|name| (*name).to_string()).collect()
                    }),
            );
            Some(FormEventParameterForms {
                event_id: spec.event_id.clone(),
                handler_ru: spec.handler_ru.clone(),
                forms,
            })
        })
        .collect()
}

/// Parameters of the current handler signature of a form, item, table or
/// column event: the help list, with `Элемент` first for an item, table or
/// column event. The help omits `Элемент`, which the platform passes first to
/// those handlers; it is not added where the help already names it or
/// [`TABLE_EVENTS_WITHOUT_ITEM`] says otherwise. A command action keeps its
/// own `(Команда)`. `None` means the catalog signature has no parameter list.
pub fn form_event_handler_parameters_8_3_27(
    owner: FormEventOwnerKind,
    spec: &PlatformEventSpec,
) -> Option<Vec<String>> {
    let mut parameters = signature_parameters(&spec.signature_ru)?;
    let item_owner = matches!(
        owner,
        FormEventOwnerKind::Element(_) | FormEventOwnerKind::Table | FormEventOwnerKind::Column(_)
    );
    let explicit_item = parameters
        .first()
        .is_some_and(|first| first.to_lowercase() == "элемент");
    let without_item = owner == FormEventOwnerKind::Table
        && TABLE_EVENTS_WITHOUT_ITEM.contains(&spec.event_id.as_str());
    if item_owner && !explicit_item && !without_item {
        parameters.insert(0, "Элемент".to_string());
    }
    Some(parameters)
}

/// The Russian catalog signature of the event with the parameters of
/// [`form_event_handler_parameters_8_3_27`]. A signature without a parameter
/// list is returned as the catalog has it.
pub fn form_event_handler_signature_8_3_27(
    owner: FormEventOwnerKind,
    spec: &PlatformEventSpec,
) -> String {
    let Some(parameters) = form_event_handler_parameters_8_3_27(owner, spec) else {
        return spec.signature_ru.clone();
    };
    let Some((head, _)) = spec.signature_ru.split_once('(') else {
        return spec.signature_ru.clone();
    };
    let tail = spec
        .signature_ru
        .rsplit_once(')')
        .map_or("", |(_, tail)| tail);
    format!("{head}({}){tail}", parameters.join(", "))
}

/// A handler signature that cannot belong to the bound event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HandlerSignatureFinding {
    /// The parameter names fit another event of the owner strictly better.
    ForeignEvent {
        bound_ru: String,
        expected: Vec<String>,
        other_ru: String,
        other: Vec<String>,
    },
    /// More required parameters than any known list of the event.
    ExtraRequired {
        bound_ru: String,
        required: usize,
        expected: Vec<String>,
    },
}

/// Compares a handler signature with the bound event among the owner's
/// events. A shorter signature is never a finding: the platform still calls
/// handlers written for earlier versions and matches their parameters by
/// meaning.
///
/// The name score of a list is the number of handler parameter names it
/// contains, case-insensitively. Another event is a candidate when its score
/// is at least 2 and strictly greater than the bound event's score. The
/// candidate must be the only one with the highest score; among equal scores
/// the only one with a list of exactly the handler's parameter count wins.
/// The handler's parameter count must also differ from every known list of
/// the bound event: with the same count the platform passes exactly the bound
/// event's arguments, and different names are only names.
pub fn foreign_handler_signature_8_3_27(
    owner_events: &[FormEventParameterForms],
    event: &str,
    parameters: &[(&str, bool)],
) -> Option<HandlerSignatureFinding> {
    let bound = owner_events
        .iter()
        .find(|candidate| candidate.event_id == event)?;
    let names = parameters
        .iter()
        .map(|parameter| parameter.0.to_lowercase())
        .collect::<Vec<_>>();
    let score = |forms: &[Vec<String>]| {
        forms
            .iter()
            .map(|form| {
                names
                    .iter()
                    .filter(|name| form.iter().any(|known| known.to_lowercase() == **name))
                    .count()
            })
            .max()
            .unwrap_or(0)
    };
    let fits_count =
        |forms: &[Vec<String>]| forms.iter().any(|form| form.len() == parameters.len());
    let bound_score = score(&bound.forms);
    let scored = owner_events
        .iter()
        .filter(|other| other.event_id != event)
        .map(|other| (other, score(&other.forms)))
        .collect::<Vec<_>>();
    let top = scored.iter().map(|(_, score)| *score).max().unwrap_or(0);
    let leaders = scored
        .iter()
        .filter(|(_, score)| *score == top)
        .map(|(other, _)| *other)
        .collect::<Vec<_>>();
    let leader = match leaders.as_slice() {
        [only] => Some(*only),
        _ => {
            let mut fitting = leaders.iter().filter(|other| fits_count(&other.forms));
            match (fitting.next(), fitting.next()) {
                (Some(only), None) => Some(*only),
                _ => None,
            }
        }
    };
    if let Some(other) = leader {
        if !fits_count(&bound.forms) && top >= 2 && top > bound_score {
            return Some(HandlerSignatureFinding::ForeignEvent {
                bound_ru: bound.handler_ru.clone(),
                expected: bound.forms[0].clone(),
                other_ru: other.handler_ru.clone(),
                other: other.forms[0].clone(),
            });
        }
    }
    let required = parameters.iter().filter(|parameter| !parameter.1).count();
    let maximum = bound.forms.iter().map(Vec::len).max().unwrap_or(0);
    (required > maximum).then(|| HandlerSignatureFinding::ExtraRequired {
        bound_ru: bound.handler_ru.clone(),
        required,
        expected: bound.forms[0].clone(),
    })
}

fn signature_parameters(signature: &str) -> Option<Vec<String>> {
    let (_, tail) = signature.split_once('(')?;
    let (list, _) = tail.rsplit_once(')')?;
    Some(
        list.split(',')
            .map(str::trim)
            .filter(|parameter| !parameter.is_empty())
            .map(str::to_string)
            .collect(),
    )
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
pub struct FormEventBinding<'a> {
    pub name: &'a str,
    pub handler: &'a str,
    pub call_type: Option<&'a str>,
    /// Путь данных таблицы-владельца: по нему каталог выбирает события
    /// источника динамического списка, как при публикации узлов `Event`.
    pub data_path: Option<&'a str>,
}

impl<'a> FormEventBinding<'a> {
    pub const fn new(name: &'a str, handler: &'a str) -> Self {
        Self {
            name,
            handler,
            call_type: None,
            data_path: None,
        }
    }

    #[must_use]
    pub const fn with_data_path(mut self, data_path: Option<&'a str>) -> Self {
        self.data_path = data_path;
        self
    }

    #[must_use]
    pub const fn with_call_type(mut self, call_type: &'a str) -> Self {
        self.call_type = Some(call_type);
        self
    }
}

/// Stable machine-readable event diagnostic codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FormEventDiagnosticCode {
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
    pub const fn as_str(self) -> &'static str {
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
pub struct FormEventDiagnostic {
    pub code: FormEventDiagnosticCode,
    pub target: String,
    pub event: String,
    pub detail: String,
}

impl FormEventDiagnostic {
    pub fn new(
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
    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
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

/// Событие есть в каталоге 8.3.27 для владельца этого вида и контекста формы.
fn catalogued_event(
    context: &FormEventContext,
    target: FormEventTarget,
    binding: &FormEventBinding<'_>,
) -> bool {
    let owner = match target {
        FormEventTarget::Form => FormEventOwnerKind::Form,
        FormEventTarget::Element(FormElementKind::Table) => FormEventOwnerKind::Table,
        FormEventTarget::Element(kind) => FormEventOwnerKind::Element(kind),
    };
    form_event_catalog_8_3_27(context, owner, binding.data_path)
        .iter()
        .any(|spec| spec.event_id == binding.name)
}

pub fn validate_event(
    context: &FormEventContext,
    target: FormEventTarget,
    binding: &FormEventBinding<'_>,
) -> Result<(), FormEventDiagnostic> {
    let target_text = target.to_string();

    validate_property_event_binding(context, target, binding)?;

    // Событие допустимо, если его знает матрица цели или каталог событий
    // 8.3.27, из которого `view` строит узлы `Event`: привязка на адресе
    // события исполнима везде, где этот узел опубликован (#1355).
    if !target.allowed_events().contains(&binding.name)
        && !catalogued_event(context, target, binding)
    {
        return Err(FormEventDiagnostic::new(
            FormEventDiagnosticCode::EventNotAllowed,
            target_text,
            binding.name,
        )
        .with_detail("event is not present in the target event matrix"));
    }

    if target == FormEventTarget::Form && OBJECT_RECORD_FORM_EVENTS.contains(&binding.name) {
        validate_object_event_context(context, target, binding.name)?;
    }

    Ok(())
}

/// Shared shape validation for a property-bound event. V12 event validation
/// adds its exact target matrix after this seam; hidden V13 projection uses
/// its versioned registry for applicability and reuses the same handler and
/// extension call-type rules here.
pub fn validate_property_event_binding(
    context: &FormEventContext,
    target: FormEventTarget,
    binding: &FormEventBinding<'_>,
) -> Result<(), FormEventDiagnostic> {
    if binding.handler.trim().is_empty() {
        return Err(FormEventDiagnostic::new(
            FormEventDiagnosticCode::EmptyHandler,
            target.to_string(),
            binding.name,
        )
        .with_detail("handler must not be empty"));
    }
    validate_event_call_type(context, target, binding)
}

/// Validates the extension-only XML binding fact independently from the
/// legacy v0.12 event-name matrix. Hidden v0.13 projection uses the vendor
/// catalog for applicability, while sharing this exact context rule.
pub fn validate_event_call_type(
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

/// Existing event-matrix observations exposed only to the preserved cross-owner oracles.
#[cfg(any(test, feature = "test-support"))]
pub mod test_support {
    pub fn form_event_owner_catalog_key(owner: super::FormEventOwnerKind) -> String {
        owner.catalog_key()
    }
    pub const OBJECT_RECORD_FORM_EVENTS: &[&str] = super::OBJECT_RECORD_FORM_EVENTS;
    pub const NO_EVENTS: &[&str] = super::NO_EVENTS;
}
