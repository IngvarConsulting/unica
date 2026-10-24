//! Existing XDTO read/edit result facts and logical identity.
use serde::Serialize;
use unica_address::source_target::TargetKind;
#[derive(Clone, Debug, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum XdtoLocation {
    Addressed {
        source_set: String,
        metadata_path: String,
        target_kind: TargetKind,
    },
    Unaddressable {
        source_set: String,
        owner_metadata_path: String,
        node_key: String,
        body_byte_range: XdtoByteRange,
    },
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct XdtoByteRange {
    /// UTF-8 byte offset in the decoded XML body. A leading BOM is excluded.
    pub start: usize,
    /// Exclusive UTF-8 byte offset in the decoded XML body.
    pub end: usize,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct XdtoTypeCounts {
    pub total: usize,
    pub value_types: usize,
    pub object_types: usize,
    pub global_properties: usize,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct XdtoImportInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    pub location: XdtoLocation,
}
#[derive(Clone, Copy, Debug, Serialize)]
pub enum XdtoTypeKind {
    #[serde(rename = "valueType")]
    Value,
    #[serde(rename = "objectType")]
    Object,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct XdtoBoundsInfo {
    pub lower: String,
    pub upper: String,
    pub lower_explicit: bool,
    pub upper_explicit: bool,
    pub unbounded: bool,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct XdtoAnonymousTypeInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base: Option<XdtoQNameInfo>,
    #[serde(skip_serializing_if = "crate::facets::ValueFacets::is_empty")]
    pub facets: crate::facets::ValueFacets,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub discriminator: Option<String>,
    pub properties: Vec<XdtoPropertyInfo>,
    pub location: XdtoLocation,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct XdtoPropertyInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference: Option<XdtoQNameInfo>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub type_name: Option<XdtoQNameInfo>,
    pub bounds: XdtoBoundsInfo,
    pub type_defs: Vec<XdtoAnonymousTypeInfo>,
    pub location: XdtoLocation,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct XdtoQNameInfo {
    pub raw: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prefix: Option<String>,
    pub local: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct XdtoTypeInfo {
    #[serde(skip_serializing_if = "crate::facets::ValueFacets::is_empty")]
    pub facets: crate::facets::ValueFacets,
    pub kind: XdtoTypeKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base: Option<XdtoQNameInfo>,
    /// Address of the base when it is one unambiguous local type of the same kind.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_at: Option<String>,
    pub properties: Vec<XdtoPropertyInfo>,
    pub location: XdtoLocation,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct XdtoTypeSummary {
    pub kind: XdtoTypeKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base: Option<XdtoQNameInfo>,
    pub location: XdtoLocation,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct XdtoInfoData {
    pub source_set: String,
    pub metadata_path: String,
    pub location: XdtoLocation,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_namespace: Option<String>,
    pub imports: Vec<XdtoImportInfo>,
    pub counts: XdtoTypeCounts,
    pub global_properties: Vec<XdtoPropertyInfo>,
    pub types: Vec<XdtoTypeSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub type_detail: Option<XdtoTypeInfo>,
    pub findings: Vec<XdtoFinding>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum XdtoEditOperation {
    AddValueType,
    AddObjectType,
    AddProperty,
    RemoveType,
    RemoveProperty,
}
impl XdtoEditOperation {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "addValueType" => Ok(Self::AddValueType),
            "addObjectType" => Ok(Self::AddObjectType),
            "addProperty" => Ok(Self::AddProperty),
            "removeType" => Ok(Self::RemoveType),
            "removeProperty" => Ok(Self::RemoveProperty),
            _ => Err("unsupported_node: supported operations are addValueType, addObjectType, addProperty, removeType, removeProperty".to_string()),
        }
    }

    /// The writer uses kebab-case dispatch keys for the same operations
    /// that the typed payload names with camelCase tags.
    pub fn writer_key(self) -> &'static str {
        match self {
            Self::AddValueType => "add-value-type",
            Self::AddObjectType => "add-object-type",
            Self::AddProperty => "add-property",
            Self::RemoveType => "remove-type",
            Self::RemoveProperty => "remove-property",
        }
    }
}
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum XdtoFindingSeverity {
    Info,
    Error,
}
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum XdtoFindingState {
    PreExisting,
    Introduced,
}
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum XdtoFindingPhase {
    Before,
    After,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct XdtoFinding {
    pub code: String,
    pub severity: XdtoFindingSeverity,
    pub state: XdtoFindingState,
    pub phase: XdtoFindingPhase,
    pub message: String,
    pub location: XdtoLocation,
}
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum XdtoChangeKind {
    PackageTextEdit,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct XdtoProjectedChange {
    pub kind: XdtoChangeKind,
    pub location: XdtoLocation,
    pub body_byte_range: XdtoByteRange,
    pub removed_byte_count: usize,
    pub replacement_byte_count: usize,
}
/// The effect of one element of the `operations` array. Byte
/// ranges in `change` are relative to the document state the operation was
/// applied to — operations are sequential, so element `i` sees the text
/// produced by element `i - 1`.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct XdtoOperationEffect {
    pub operation_index: usize,
    pub op: XdtoEditOperation,
    pub no_op: bool,
    pub change: Option<XdtoProjectedChange>,
    pub findings: Vec<XdtoFinding>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct XdtoEditData {
    pub source_set: String,
    pub metadata_path: String,
    pub location: XdtoLocation,
    pub no_op: bool,
    pub effects: Vec<XdtoOperationEffect>,
}
/// Plan details section of object type additions in the apply answer.
pub const XDTO_OBJECTS: unica_source_contract::proposal::PlanDetailKey =
    unica_source_contract::proposal::PlanDetailKey::new("xdtoObjects");

/// One object type addition, described by the final staged package of the
/// plan: whether the type is still present, its base and its neighbours.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct XdtoObjectPlanDetail {
    pub at: String,
    pub name: String,
    pub present: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base: Option<XdtoQNameInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next: Option<String>,
}
pub trait XdtoIdentity {
    fn source_set(&self) -> &str;
    fn metadata_path(&self) -> &str;
}
