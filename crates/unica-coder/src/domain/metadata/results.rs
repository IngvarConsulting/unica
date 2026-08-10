use super::{
    MetaDiagnostic, MetaEventSource, MetaFillValue, MetaPredefinedAccountType, MetaPropertyKey,
    MetaPropertyValue, MetadataKind, MetadataType, MetadataTypeVariant,
};
use crate::domain::source_target::MetadataAddress;
use crate::domain::subsystem::SubsystemAddress;
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MetaValidationStatus {
    Passed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MetaValidationData {
    pub(crate) status: MetaValidationStatus,
    pub(crate) diagnostics: Vec<MetaDiagnostic>,
}

/// Who uses this object, read straight from the source tree.
///
/// Roles, event subscriptions and functional options are ordinary XML in the
/// configuration, so each list is exact and complete. They carry no index
/// metadata and no continuation: on a real vendor-class configuration the
/// largest of them is a few dozen entries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MetaUsageData {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) roles: Option<Vec<Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) subscriptions: Option<Vec<Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) functional_options: Option<Vec<Value>>,
}

/// Predefined items of the object itself.
///
/// This is the object's own content, read from its `Ext/Predefined.xml`, so it
/// sits beside `collections` rather than among the things that reference the
/// object. It keeps counters because it is the one list that genuinely runs
/// long: a BSP identifier catalog reaches hundreds of entries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MetaPredefinedItemsData {
    pub(crate) total: usize,
    pub(crate) returned: usize,
    pub(crate) truncated: bool,
    pub(crate) items: Vec<MetaPredefinedItemData>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MetaPredefinedItemData {
    pub(crate) id: String,
    pub(crate) parent_id: Option<String>,
    pub(crate) name: String,
    pub(crate) code: String,
    pub(crate) description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) is_folder: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) r#type: Option<MetadataType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) account_type: Option<MetaPredefinedAccountType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) off_balance: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) order: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) accounting_flags: Option<BTreeMap<String, bool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) ext_dimension_types: Option<Vec<MetaPredefinedExtDimensionTypeData>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) action_period_is_base: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MetaPredefinedExtDimensionTypeData {
    pub(crate) name: String,
    pub(crate) turnover: bool,
    pub(crate) accounting_flags: BTreeMap<String, bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MetaPropertyData {
    pub(crate) key: MetaPropertyKey,
    pub(crate) value: MetaPropertyValue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum MetadataMutationCapability {
    Editable,
    // The closed read model reserves this state for the first named platform
    // variant whose writer evidence is not yet available (ADR-0042).
    #[allow(dead_code)]
    ReadOnly,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ObservedMetadataType {
    pub(crate) variants: Vec<MetadataTypeVariant>,
    pub(crate) mutation_capability: MetadataMutationCapability,
}

impl ObservedMetadataType {
    pub(crate) fn editable(value: MetadataType) -> Self {
        Self {
            variants: value.variants,
            mutation_capability: MetadataMutationCapability::Editable,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MetaElementData {
    pub(crate) name: String,
    #[serde(skip_serializing_if = "is_false")]
    pub(crate) incomplete: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) synonym: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) comment: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) r#type: Option<ObservedMetadataType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) required: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) fill_value: Option<MetaFillValue>,
    pub(crate) attributes: Vec<MetaElementData>,
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MetaCollectionsData {
    pub(crate) attributes: Vec<MetaElementData>,
    pub(crate) tabular_sections: Vec<MetaElementData>,
    pub(crate) dimensions: Vec<MetaElementData>,
    pub(crate) resources: Vec<MetaElementData>,
    pub(crate) enum_values: Vec<MetaElementData>,
    pub(crate) columns: Vec<MetaElementData>,
    pub(crate) forms: Vec<MetaElementData>,
    pub(crate) templates: Vec<MetaElementData>,
    pub(crate) commands: Vec<MetaElementData>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MetaRelationTargetData {
    pub(crate) kind: String,
    pub(crate) value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MetaRelationsData {
    pub(crate) owners: Vec<MetaRelationTargetData>,
    pub(crate) register_records: Vec<MetaRelationTargetData>,
    pub(crate) based_on: Vec<MetaRelationTargetData>,
    pub(crate) input_by_string: Vec<MetaRelationTargetData>,
    pub(crate) source: Vec<MetaEventSource>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MetaSupportStatus {
    Supported,
    Locked,
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MetaInfoData {
    pub(crate) metadata_path: MetadataAddress,
    pub(crate) kind: MetadataKind,
    pub(crate) name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) synonym: Option<String>,
    pub(crate) support: MetaSupportStatus,
    pub(crate) properties: Vec<MetaPropertyData>,
    pub(crate) relations: MetaRelationsData,
    pub(crate) collections: MetaCollectionsData,
    /// Registered functional subsystems whose own `Content` contains this
    /// object. `None` means the topology was not proved, not an empty set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) functional_subsystems: Option<Vec<SubsystemAddress>>,
    /// Registered interface subsystems whose own `Content` contains this
    /// object. The role already includes every ancestor flag.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) interface_subsystems: Option<Vec<SubsystemAddress>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) predefined_items: Option<MetaPredefinedItemsData>,
    pub(crate) usage: MetaUsageData,
    pub(crate) validation: MetaValidationData,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MetaPublicationAction {
    Create,
    Update,
    Remove,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MetaPublicationResource {
    Descriptor,
    Registration,
    Module,
    Form,
    Template,
    Command,
    Dependency,
    PredefinedData,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MetaPublicationPlanEntry {
    pub(crate) action: MetaPublicationAction,
    pub(crate) resource: MetaPublicationResource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) metadata_path: Option<MetadataAddress>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MetaMutationData {
    pub(crate) metadata_path: MetadataAddress,
    pub(crate) changed: bool,
    pub(crate) publication_plan: Vec<MetaPublicationPlanEntry>,
    pub(crate) effects: Vec<MetaMutationEffect>,
    pub(crate) validation: MetaValidationData,
    pub(crate) diagnostics: Vec<MetaDiagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MetaMutationEffect {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) operation_index: Option<u64>,
    pub(crate) operation: String,
    pub(crate) target: String,
    pub(crate) before: Option<Value>,
    pub(crate) after: Option<Value>,
}
