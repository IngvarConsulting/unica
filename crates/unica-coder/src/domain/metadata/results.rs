use super::{
    MetaDiagnostic, MetaPropertyKey, MetaPropertyValue, MetadataKind, MetadataReference,
    MetadataType,
};
use crate::domain::source_target::MetadataAddress;
use serde::Serialize;

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
    Fresh,
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
pub(crate) struct MetaRelatedItem {
    pub(crate) name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) metadata_path: Option<MetadataAddress>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) role: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MetaRelatedSections {
    pub(crate) modules: MetaRelatedSection<MetaRelatedItem>,
    pub(crate) roles: MetaRelatedSection<MetaRelatedItem>,
    pub(crate) subscriptions: MetaRelatedSection<MetaRelatedItem>,
    pub(crate) functional_options: MetaRelatedSection<MetaRelatedItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) predefined_items: Option<MetaRelatedSection<MetaRelatedItem>>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) synonym: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) comment: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) r#type: Option<MetadataType>,
    pub(crate) attributes: Vec<MetaElementData>,
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
    pub(crate) owners: Vec<MetadataReference>,
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
    pub(crate) validation: MetaValidationData,
    pub(crate) diagnostics: Vec<MetaDiagnostic>,
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
