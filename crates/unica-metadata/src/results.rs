use super::{
    MetaDiagnostic, MetaEventSource, MetaFillValue, MetaInfoDeclarations, MetaInfoDetails,
    MetaInfoPropertyValue, MetaObservedProperty, MetaPredefinedAccountType, MetadataType,
    ObservedMetadataType,
};
use crate::subsystem::SubsystemAddress;
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;
use unica_address::source_target::MetadataAddress;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MetaValidationStatus {
    Passed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetaValidationData {
    pub status: MetaValidationStatus,
    pub diagnostics: Vec<MetaDiagnostic>,
}

/// Who uses this object, read straight from the source tree.
///
/// Roles, event subscriptions and functional options are ordinary XML in the
/// configuration, so each list is exact and complete. They carry no index
/// metadata and no continuation: on a real vendor-class configuration the
/// largest of them is a few dozen entries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MetaUsageData {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub roles: Option<Vec<Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subscriptions: Option<Vec<Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub functional_options: Option<Vec<Value>>,
}

/// Predefined items of the object itself.
///
/// This is the object's own content, read from its `Ext/Predefined.xml`, so it
/// sits beside `collections` rather than among the things that reference the
/// object. It keeps counters because it is the one list that genuinely runs
/// long: a BSP identifier catalog reaches hundreds of entries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetaPredefinedItemsData {
    pub total: usize,
    pub returned: usize,
    pub truncated: bool,
    pub items: Vec<MetaPredefinedItemData>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetaPredefinedItemData {
    pub id: String,
    pub parent_id: Option<String>,
    pub name: String,
    pub code: String,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_folder: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<MetadataType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_type: Option<MetaPredefinedAccountType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub off_balance: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub order: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accounting_flags: Option<BTreeMap<String, bool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ext_dimension_types: Option<Vec<MetaPredefinedExtDimensionTypeData>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action_period_is_base: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetaPredefinedExtDimensionTypeData {
    pub name: String,
    pub turnover: bool,
    pub accounting_flags: BTreeMap<String, bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetaInfoPropertyData {
    pub key: String,
    pub value: MetaInfoPropertyValue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum MetadataMutationCapability {
    Editable,
    // The closed read model reserves this state for the first named platform
    // variant whose writer evidence is not yet available (ADR-0042).
    #[allow(dead_code)]
    ReadOnly,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetaElementData {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub column_references: Option<ColumnReferenceFacts>,
    pub name: String,
    #[serde(skip_serializing_if = "is_false")]
    pub incomplete: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub synonym: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub r#type: Option<ObservedMetadataType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fill_value: Option<MetaFillValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub addressing_dimension: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub properties: Vec<MetaObservedProperty>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub standard_attributes: Option<Option<Vec<super::MetaStandardAttribute>>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub property_limits: Vec<String>,
    pub attributes: Vec<MetaElementData>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnReferenceFacts {
    pub targets: Option<Vec<ColumnAttributeReference>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub limits: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ColumnAttributeReference {
    pub owner: MetadataAddress,
    pub attribute: String,
}

impl std::fmt::Display for ColumnAttributeReference {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.Attribute.{}", self.owner, self.attribute)
    }
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetaCollectionsData {
    pub attributes: Vec<MetaElementData>,
    pub tabular_sections: Vec<MetaElementData>,
    pub dimensions: Vec<MetaElementData>,
    pub resources: Vec<MetaElementData>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recalculations: Option<Option<Vec<MetaElementData>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accounting_flags: Option<Option<Vec<MetaElementData>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ext_dimension_accounting_flags: Option<Option<Vec<MetaElementData>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub addressing_attributes: Option<Option<Vec<MetaElementData>>>,
    pub enum_values: Vec<MetaElementData>,
    pub columns: Vec<MetaElementData>,
    pub forms: Vec<MetaElementData>,
    pub templates: Vec<MetaElementData>,
    pub commands: Vec<MetaElementData>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetaRelationTargetData {
    pub kind: String,
    pub value: String,
}

/// The XML `AutoRecord` value of an ExchangePlan content entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum MetaExchangeAutoRecord {
    Allow,
    Deny,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetaExchangeContentItem {
    pub target: MetadataAddress,
    pub auto_record: MetaExchangeAutoRecord,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MetaRelationsData {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<Option<Vec<MetaExchangeContentItem>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owners: Option<Option<Vec<MetaRelationTargetData>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub register_records: Option<Option<Vec<MetaRelationTargetData>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub based_on: Option<Option<Vec<MetaRelationTargetData>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_by_string: Option<Option<Vec<MetaRelationTargetData>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_lock_fields: Option<Option<Vec<MetaRelationTargetData>>>,
    pub source: Vec<MetaEventSource>,
    /// `v8:TypeId` members of an extension's Source: generated types of the
    /// extended configuration named by UUID. Read-only.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub source_type_ids: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MetaSupportStatus {
    Supported,
    Locked,
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetaInfoData {
    pub metadata_path: MetadataAddress,
    #[serde(flatten)]
    pub details: MetaInfoDetails,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub synonym: Option<String>,
    pub support: MetaSupportStatus,
    pub properties: Vec<MetaInfoPropertyData>,
    #[serde(flatten)]
    pub declarations: MetaInfoDeclarations,
    pub relations: MetaRelationsData,
    pub collections: MetaCollectionsData,
    /// Registered functional subsystems whose own `Content` contains this
    /// object. `None` means the topology was not proved, not an empty set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub functional_subsystems: Option<Vec<SubsystemAddress>>,
    /// Registered interface subsystems whose own `Content` contains this
    /// object. The role already includes every ancestor flag.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interface_subsystems: Option<Vec<SubsystemAddress>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub predefined_items: Option<MetaPredefinedItemsData>,
    pub usage: MetaUsageData,
    pub validation: MetaValidationData,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MetaPublicationAction {
    Create,
    Update,
    Remove,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MetaPublicationResource {
    Descriptor,
    Registration,
    Module,
    Form,
    Template,
    Command,
    Dependency,
    PredefinedData,
    Help,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetaPublicationPlanEntry {
    pub action: MetaPublicationAction,
    pub resource: MetaPublicationResource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata_path: Option<MetadataAddress>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetaMutationData {
    pub metadata_path: MetadataAddress,
    pub changed: bool,
    pub publication_plan: Vec<MetaPublicationPlanEntry>,
    /// Затронутые пути рабочего пространства — план в предпросмотре
    /// и фактическая квитанция в применении; конверт `changes`/`artifacts`
    /// строится отсюда же.
    pub changed_paths: Vec<MetaChangedPath>,
    pub effects: Vec<MetaMutationEffect>,
    pub validation: MetaValidationData,
    pub diagnostics: Vec<MetaDiagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetaChangedPath {
    pub action: MetaPublicationAction,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetaMutationEffect {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation_index: Option<u64>,
    pub operation: String,
    pub target: String,
    pub before: Option<Value>,
    pub after: Option<Value>,
}

/// Plan-detail section of `content.set`: the final composition of every
/// ExchangePlan the batch names, as published in `data.exchangeContent`.
pub const EXCHANGE_CONTENT: unica_source_contract::proposal::PlanDetailKey =
    unica_source_contract::proposal::PlanDetailKey::new("exchangeContent");

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExchangeContentPlanItem {
    pub at: String,
    pub auto_record: MetaExchangeAutoRecord,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExchangeContentPlanDetail {
    pub at: String,
    pub items: Vec<ExchangeContentPlanItem>,
}
