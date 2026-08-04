use super::{
    MetaDiagnostic, MetaFillValue, MetaPropertyKey, MetaPropertyValue, MetadataKind, MetadataType,
};
use crate::domain::source_target::MetadataAddress;
use serde::Serialize;
use serde_json::Value;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MetaCompleteness {
    Complete,
    Partial,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MetaFreshness {
    Current,
    Stale,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MetaRelatedStatus {
    Ready,
    Partial,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MetaRelatedSection<T> {
    pub(crate) status: MetaRelatedStatus,
    pub(crate) freshness: MetaFreshness,
    pub(crate) completeness: MetaCompleteness,
    pub(crate) total: usize,
    pub(crate) returned: usize,
    pub(crate) truncated: bool,
    pub(crate) items: Vec<T>,
    pub(crate) diagnostics: Vec<MetaDiagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MetaRelatedSections {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) modules: Option<MetaRelatedSection<Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) roles: Option<MetaRelatedSection<Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) subscriptions: Option<MetaRelatedSection<Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) functional_options: Option<MetaRelatedSection<Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) predefined_items: Option<MetaRelatedSection<Value>>,
}

/// Transitional typed profile answer owned by the metadata domain. The legacy
/// application compatibility bridge reuses it until the atomic surface switch.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetaProfileResult {
    pub object_name: String,
    pub category: Option<String>,
    pub sections: Vec<MetaProfileSection>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetaProfileSection {
    pub name: String,
    pub status: String,
    pub total: u64,
    pub total_is_lower_bound: bool,
    pub returned: u64,
    pub items: Vec<Value>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MetaPropertyData {
    pub(crate) key: MetaPropertyKey,
    pub(crate) value: MetaPropertyValue,
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
    pub(crate) r#type: Option<MetadataType>,
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
    pub(crate) validation: MetaValidationData,
    pub(crate) related: MetaRelatedSections,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn related_section_serialization_keeps_completeness_and_freshness_explicit() {
        let section = MetaRelatedSection::<String> {
            status: MetaRelatedStatus::Partial,
            freshness: MetaFreshness::Stale,
            completeness: MetaCompleteness::Partial,
            total: 3,
            returned: 1,
            truncated: true,
            items: vec!["one".into()],
            diagnostics: vec![],
        };

        let value = serde_json::to_value(section).unwrap();
        assert_eq!(value["status"], "partial");
        assert_eq!(value["freshness"], "stale");
        assert_eq!(value["completeness"], "partial");
        assert_eq!(value["total"], 3);
        assert_eq!(value["returned"], 1);
        assert_eq!(value["truncated"], true);
    }
}
