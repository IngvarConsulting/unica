//! Local channel declarations, independent of the IntegrationService writer.
use crate::MetaObservedProperty;
use serde::Serialize;

pub const INTEGRATION_CHANNEL_PROPERTIES: &[&str] = &[
    "Synonym",
    "Comment",
    "ExternalIntegrationServiceChannelName",
    "MessageDirection",
    "ReceiveMessageProcessing",
    "Transactioned",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IntegrationChannel {
    pub name: String,
    pub properties: Vec<MetaObservedProperty>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub property_limits: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IntegrationChannelLimit {
    pub field: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct IntegrationChannels {
    pub items: Option<Vec<IntegrationChannel>>,
    pub identity_incomplete: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub unavailable_names: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub limits: Vec<IntegrationChannelLimit>,
}
