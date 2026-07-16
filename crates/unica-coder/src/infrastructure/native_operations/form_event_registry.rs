//! Registry and validation rules for managed-form event bindings.
//!
//! The registry deliberately keeps XML context discovery separate from form
//! mutation. Callers can validate a proposed binding before changing the
//! source document and reuse the same rules from `form.edit`, `form.compile`,
//! and `form.validate`.

use roxmltree::Node;
use std::fmt;

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

const NAMED_PERSISTENT_OBJECT_TYPES: &[&str] = &[
    "CatalogObject",
    "DocumentObject",
    "BusinessProcessObject",
    "TaskObject",
    "ExchangePlanObject",
    "ChartOfCharacteristicTypesObject",
];

const NAMED_PERSISTENT_RECORD_TYPES: &[&str] = &[
    "InformationRegisterRecordManager",
    "InformationRegisterRecordSet",
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
}

/// Form-level information needed by context-sensitive event rules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FormEventContext {
    pub(crate) definition: FormDefinitionKind,
    pub(crate) main_attribute: MainAttributeKind,
    pub(crate) main_attribute_type: Option<String>,
    pub(crate) main_attribute_provenance: MainAttributeProvenance,
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
        let form_main_attribute = direct_main_attribute(root);
        let base_main_attribute = base_form.and_then(direct_main_attribute);
        let (main_attribute_type, main_attribute_provenance) =
            if let Some(main_attribute) = form_main_attribute {
                (
                    main_attribute_type(main_attribute),
                    MainAttributeProvenance::DirectForm,
                )
            } else if let Some(main_attribute) = base_main_attribute {
                (
                    main_attribute_type(main_attribute),
                    MainAttributeProvenance::DirectBaseForm,
                )
            } else if definition == FormDefinitionKind::Extension {
                (None, MainAttributeProvenance::InheritedBaseFormUnavailable)
            } else {
                (None, MainAttributeProvenance::Missing)
            };
        let main_attribute = main_attribute_type
            .as_deref()
            .map(MainAttributeKind::from_type_name)
            .unwrap_or(MainAttributeKind::Unknown);

        Self {
            definition,
            main_attribute,
            main_attribute_type,
            main_attribute_provenance,
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

    if target == FormEventTarget::Form && OBJECT_RECORD_FORM_EVENTS.contains(&binding.name) {
        validate_object_event_context(context, target, binding.name)?;
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
mod tests {
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
            main_attribute,
            main_attribute_type: None,
            main_attribute_provenance: MainAttributeProvenance::Missing,
        }
    }

    fn extension_context(main_attribute: MainAttributeKind) -> FormEventContext {
        FormEventContext {
            definition: FormDefinitionKind::Extension,
            main_attribute,
            main_attribute_type: None,
            main_attribute_provenance: MainAttributeProvenance::InheritedBaseFormUnavailable,
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
        assert_eq!(
            MainAttributeKind::from_type_name("cfg:DocumentObject.Order"),
            MainAttributeKind::PersistentObject
        );
        assert_eq!(
            MainAttributeKind::from_type_name("cfg:ConstantsSet"),
            MainAttributeKind::PersistentObject
        );
        assert_eq!(
            MainAttributeKind::from_type_name("cfg:InformationRegisterRecordSet.Prices"),
            MainAttributeKind::PersistentRecord
        );
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
            "cfg:ChartOfAccountsObject.Main",
            "cfg:ChartOfCalculationTypesObject.Payroll",
            "cfg:AccumulationRegisterRecordSet.Stock",
            "cfg:AccountingRegisterRecordSet.Accounting",
            "cfg:CalculationRegisterRecordSet.Payroll",
            "cfg:ReportObject.Sales",
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
}
