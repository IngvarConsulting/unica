//! Existing form read/edit/compile facts, independent of XML or filesystem implementations.
use crate::events::*;
use serde::Serialize;
use std::ops::Range;
use unica_execution_contract::mutation::MutationData;
use unica_metadata::support_state::ObjectSupportData as DomainObjectSupportData;
/// Typed answer of `unica.form.info` (ADR-0023). The report sliced a form into
/// numbered lines and hid deep pages behind `-Expand`; the data carries the
/// whole form at once, so a caller never has to ask for the rest.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FormInfoData {
    pub name: String,
    /// The form title when it declares one; `null` otherwise.
    pub title: Option<String>,
    /// The metadata object the form belongs to; `null` for a standalone form.
    pub object_context: Option<String>,
    pub is_extension: bool,
    /// `BaseForm` version for an extension form; `null` when the form declares
    /// no base form, empty string when it declares one without a version.
    pub base_form_version: Option<String>,
    pub support: DomainObjectSupportData,
    /// Canonical form-only evidence; legacy form.info retains its JSON shape.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub support_details: Option<FormSupportDetails>,
    pub properties: Vec<FormInfoProperty>,
    pub events: Vec<FormInfoEvent>,
    /// Items of the form's own command bar; empty when it has none or when
    /// `CommandBarLocation` is `None`.
    pub auto_command_bar: Vec<String>,
    /// Where that command bar sits: `Auto`, `Top`, `Bottom` or `None`.
    pub command_bar_location: String,
    pub elements: Vec<FormInfoElement>,
    pub attributes: Vec<FormInfoAttribute>,
    pub parameters: Vec<FormInfoParameter>,
    pub commands: Vec<FormInfoCommand>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conditional_appearance: Option<FormInfoConditionalAppearance>,
    /// Parsed semantic context retained for the hidden V13 event projector.
    /// V12 `form.info` keeps its established public JSON shape.
    #[serde(skip)]
    pub event_context: FormEventContext,
}

/// `Attributes/ConditionalAppearance` of a managed form: rules in XML order
/// with bounded typed facts and the limits that withheld the rest.
#[derive(Debug, Serialize)]
pub struct FormInfoConditionalAppearance {
    pub rules: Vec<FormInfoConditionalAppearanceRule>,
    pub limits: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct FormInfoConditionalAppearanceRule {
    pub name: String,
    #[serde(rename = "use", skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub presentation: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selection: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub appearance: Option<serde_json::Value>,
    pub limits: Vec<String>,
}

/// Own UUID rule is independent of configuration and owner restrictions.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FormSupportDetails {
    pub support_rule: Option<unica_metadata::support_state::ObjectSupportState>,
    pub support_rule_reason: Option<FormSupportRuleReason>,
    /// Own rule, owner and configuration together: a deny by any of them is
    /// `false`; this does not imply operation availability. The owner's own
    /// state is read from the owner itself.
    pub direct_edit_safe: Option<bool>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FormSupportRuleReason {
    MarkerAbsent,
    RuleAbsent,
    ConfigurationRemoved,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FormInfoProperty {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FormInfoEvent {
    pub name: String,
    pub handler: String,
    /// The `callType` attribute; `null` when the event declares none.
    pub call_type: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FormInfoElement {
    pub tag: String,
    /// Closed platform element kind retained for internal event projection.
    /// The V12 serialized `tag` remains the human-readable tree marker.
    #[serde(skip)]
    pub event_kind: Option<FormElementKind>,
    pub name: String,
    /// The data path or command the element is bound to; `null` when unbound.
    pub binding: Option<FormInfoBinding>,
    /// Direct XML facts may coexist on a common-command button. The legacy
    /// `binding` above keeps its precedence for existing event consumers.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command_name: Option<String>,
    /// Damage to one binding does not hide the other or the element itself.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub binding_limits: Vec<String>,
    /// The element title when it differs from its name; `null` otherwise.
    pub title: Option<String>,
    pub visible: bool,
    pub enabled: bool,
    pub read_only: bool,
    pub events: Vec<FormInfoEvent>,
    pub children: Vec<FormInfoElement>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FormInfoBinding {
    /// `dataPath`, `standardCommand`, `command` or `other`.
    pub kind: String,
    pub target: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FormInfoAttribute {
    pub name: String,
    #[serde(rename = "type")]
    pub type_name: Option<String>,
    pub is_main: bool,
    /// `MainTable` of a dynamic list; `null` for every other type.
    pub main_table: Option<String>,
    /// Columns of a value table or tree; empty for every other type.
    pub columns: Vec<FormInfoColumn>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FormInfoColumn {
    pub name: String,
    #[serde(rename = "type")]
    pub type_name: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FormInfoParameter {
    pub name: String,
    #[serde(rename = "type")]
    pub type_name: Option<String>,
    pub is_key: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FormInfoCommand {
    pub name: String,
    pub actions: Vec<FormInfoEvent>,
    /// The command title in its first language; `null` when it has none.
    pub title: Option<String>,
    /// The tooltip in its first language; `null` when the command has none.
    pub tooltip: Option<String>,
    /// The keyboard shortcut; `null` when the command declares none.
    pub shortcut: Option<String>,
    /// The picture reference (`StdPicture.X`, `CommonPicture.X`); `null`
    /// when the command has no picture.
    pub picture: Option<String>,
    /// The command's DefaultRepresentation; `null` when it keeps the default.
    pub representation: Option<String>,
}

/// Minimal, provider-neutral form evidence consumed by the hidden event
/// projector. It deliberately excludes support state, report properties and
/// every other V12 `form.info` concern.
#[derive(Debug, Clone)]
pub struct FormEventEvidence {
    pub context: FormEventContext,
    pub events: Vec<FormInfoEvent>,
    pub elements: Vec<FormInfoElement>,
    pub commands: Vec<FormInfoCommand>,
}

/// Typed answer of `unica.form.add` (ADR-0023): the form that was scaffolded,
/// where it was registered, and whether it became the object's default.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FormAddData {
    pub object_kind: String,
    pub object_name: String,
    pub form: String,
    /// The object descriptor the form was registered in.
    pub registered_in: String,
    /// The default-form property this form was assigned to; `null` when the
    /// call left the object's defaults alone.
    pub default_property: Option<String>,
    pub mutation: MutationData,
}

pub struct FormCompileObjectField {
    pub name: String,
    pub type_name: String,
}

pub struct FormCompileObjectTabularSection {
    pub name: String,
    pub synonym: String,
    pub columns: Vec<FormCompileObjectField>,
}

pub struct FormCompileObjectMeta {
    pub object_type: String,
    pub name: String,
    pub synonym: String,
    pub attributes: Vec<FormCompileObjectField>,
    pub tabular_sections: Vec<FormCompileObjectTabularSection>,
    pub code_length: i64,
    pub hierarchical: bool,
    pub hierarchy_type: String,
    pub owners: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FormEditData {
    pub changed: bool,
    pub removed: Vec<FormEditRemovedElement>,
    /// Elements added to the form layout.
    pub added_elements: Vec<FormEditAddedElement>,
    /// Attributes added to the form.
    pub added_attributes: Vec<FormEditAddedAttribute>,
    /// Commands added to the form.
    pub added_commands: Vec<FormEditAddedCommand>,
    /// Event handlers wired up, on the form itself or on an element.
    pub added_events: Vec<FormEditAddedEvent>,
    pub validation: FormEditValidation,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FormEditAddedElement {
    pub kind: String,
    pub name: String,
    /// The data path the element is bound to; `null` when it binds nothing.
    pub path: Option<String>,
    /// How a table renders its rows; `null` for anything but a table, or when
    /// the definition leaves it at the platform default.
    pub representation: Option<String>,
    /// Whether a table auto-inserts a new row; `null` when unset.
    pub auto_insert_new_row: Option<bool>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FormEditAddedAttribute {
    pub name: String,
    #[serde(rename = "type")]
    pub type_name: String,
    pub id: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FormEditAddedCommand {
    pub name: String,
    /// The command's action handler; `null` when it has none.
    pub action: Option<String>,
    pub id: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FormEditAddedEvent {
    /// The element the handler belongs to; `null` for a form-level event.
    pub element: Option<String>,
    pub name: String,
    pub handler: String,
    /// The `[client]`/`[server]` call type, when the definition sets one.
    pub call_type: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct FormEditRemovedElement {
    pub name: String,
    pub kind: String,
    pub reason: FormEditRemovalReason,
}

/// What one applied form definition added, for the caller's report.
pub struct FormEditApplied {
    pub added_elements: Vec<FormEditAddedElement>,
    pub added_attrs: Vec<FormEditAddedAttribute>,
    pub added_cmds: Vec<FormEditAddedCommand>,
    pub added_form_events: Vec<FormEditAddedEvent>,
    pub added_element_events: Vec<FormEditAddedEvent>,
    pub planned_removals: Vec<FormEditPlannedRemoval>,
    pub companion_count: usize,
    pub form_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormEditContainedNode {
    pub name: String,
    pub kind: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormEditPlannedRemoval {
    pub name: String,
    pub kind: String,
    pub contained: Vec<FormEditContainedNode>,
    pub range: Range<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormEditPlannedEvent {
    pub owner: FormEditEventOwner,
    pub name: String,
    pub handler: String,
    pub call_type: Option<String>,
}

#[derive(Debug)]
pub struct FormCompileStats {
    pub element_ids: usize,
    pub attributes: usize,
    pub commands: usize,
    pub parameters: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FormTypeEntry {
    pub kind: FormTypeNodeKind,
    pub wire_name: String,
    pub local_namespace: Option<(&'static str, &'static str)>,
    pub qualifier: Option<FormTypeQualifier>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormEditMode {
    Apply,
    Preview,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FormEditRemovalReason {
    Requested,
    Contained,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FormEditValidation {
    Passed,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum FormEditEventOwner {
    Form,
    Element(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Utf8Bom {
    Present,
    Absent,
}

pub enum FormEditInsertTarget {
    ExistingChildItems {
        range: std::ops::Range<usize>,
        child_indent: String,
    },
    ElementNeedsChildItems {
        range: std::ops::Range<usize>,
        tag: String,
        element_indent: String,
        child_items_indent: String,
        child_indent: String,
    },
    AfterElement {
        pos: usize,
        child_indent: String,
    },
    /// A form that has no root `ChildItems` yet. An empty form legitimately
    /// has none — the platform writes the section only once it holds
    /// something — so the editor creates it rather than refusing the form
    /// another writer produced (#341).
    RootNeedsChildItems {
        pos: usize,
        section_indent: String,
        child_indent: String,
    },
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum FormTypeNodeKind {
    Type,
    TypeSet,
    TypeId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FormTypeQualifier {
    Number {
        digits: u32,
        fraction: u32,
        nonnegative: bool,
    },
    String {
        length: u32,
        fixed: bool,
    },
    Date(&'static str),
}

impl FormEventEvidence {
    pub fn from_info(data: &FormInfoData) -> Self {
        Self {
            context: data.event_context.clone(),
            events: data.events.clone(),
            elements: data.elements.clone(),
            commands: data.commands.clone(),
        }
    }
}

impl FormEditMode {
    pub const fn is_preview(self) -> bool {
        matches!(self, Self::Preview)
    }
}

impl FormEditApplied {
    /// Names of the elements the definition removed, requested and contained.
    pub fn removed_element_names(&self) -> Vec<String> {
        self.planned_removals
            .iter()
            .flat_map(|removal| {
                std::iter::once(removal.name.clone())
                    .chain(removal.contained.iter().map(|node| node.name.clone()))
            })
            .collect()
    }
}

impl FormEditPlannedEvent {
    pub fn summary(&self) -> String {
        let call_type = self
            .call_type
            .as_deref()
            .map(|value| format!("[{value}]"))
            .unwrap_or_default();
        match &self.owner {
            FormEditEventOwner::Form => {
                format!("  + {}{call_type} -> {}", self.name, self.handler)
            }
            FormEditEventOwner::Element(element) => {
                format!("  + {element}.{}{call_type} -> {}", self.name, self.handler)
            }
        }
    }
}

impl FormEditInsertTarget {
    pub fn child_indent(&self) -> &str {
        match self {
            Self::ExistingChildItems { child_indent, .. }
            | Self::ElementNeedsChildItems { child_indent, .. }
            | Self::AfterElement { child_indent, .. }
            | Self::RootNeedsChildItems { child_indent, .. } => child_indent,
        }
    }
}

impl FormTypeNodeKind {
    pub fn tag(self) -> &'static str {
        match self {
            Self::Type => "Type",
            Self::TypeSet => "TypeSet",
            Self::TypeId => "TypeId",
        }
    }
}
