use crate::domain::source_target::ResolvedTarget;
use serde::Serialize;
use std::fmt;

pub const SOURCE_MANIFEST_RESOURCE_MAX: usize = 100;
pub const SOURCE_RESOURCE_PAGE_LIMIT_MAX: usize = 50;
pub const SOURCE_READ_LIMIT_MAX: usize = 64 * 1024;
pub const SOURCE_SNAPSHOT_TTL_SECONDS: u64 = 5 * 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ResourceRole {
    BslModule,
    ConfigurationDescriptor,
    MetadataDescriptor,
    Registration,
    Form,
    Dcs,
    Mxl,
    Rights,
    BinaryTemplate,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ResourceCompleteness {
    Complete,
    Partial,
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum ResourceScope {
    #[serde(rename = "self")]
    SelfOnly,
    #[serde(rename = "aggregate")]
    Aggregate,
    #[serde(rename = "registrations")]
    Registrations,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ResourceAccess {
    Read,
    Replace,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum TextEncoding {
    #[serde(rename = "utf-8")]
    Utf8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum EolProfile {
    None,
    Lf,
    Crlf,
    Cr,
    Mixed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TextProfile {
    pub encoding: TextEncoding,
    pub bom_prefix_bytes: usize,
    pub eol: EolProfile,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceLimits {
    pub max_read_bytes: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceResource {
    pub resource_id: String,
    pub role: ResourceRole,
    pub media_type: String,
    pub size: usize,
    pub hash: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_profile: Option<TextProfile>,
    pub access: Vec<ResourceAccess>,
    pub limits: ResourceLimits,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceManifestPage {
    pub snapshot_id: String,
    pub source_set: String,
    pub target: ResolvedTarget,
    pub scope: ResourceScope,
    pub completeness: ResourceCompleteness,
    pub resources: Vec<SourceResource>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceReadResult {
    pub snapshot_id: String,
    pub resource_id: String,
    pub offset: usize,
    pub length: usize,
    pub size: usize,
    pub hash: String,
    pub content: String,
    pub content_encoding: String,
    pub eof: bool,
    pub applied_limit: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_profile: Option<TextProfile>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceResourceErrorCode {
    SnapshotNotFound,
    SnapshotExpired,
    SnapshotScopeMismatch,
    ResourceNotFound,
    ResourceNotReadable,
    InvalidCursor,
    InvalidRequest,
    LimitExceeded,
    SnapshotCapacityExceeded,
    OffsetOutOfRange,
    SourceUnavailable,
    Cancelled,
}

impl SourceResourceErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SnapshotNotFound => "snapshot_not_found",
            Self::SnapshotExpired => "snapshot_expired",
            Self::SnapshotScopeMismatch => "snapshot_scope_mismatch",
            Self::ResourceNotFound => "resource_not_found",
            Self::ResourceNotReadable => "resource_not_readable",
            Self::InvalidCursor => "invalid_cursor",
            Self::InvalidRequest => "invalid_request",
            Self::LimitExceeded => "limit_exceeded",
            Self::SnapshotCapacityExceeded => "snapshot_capacity_exceeded",
            Self::OffsetOutOfRange => "offset_out_of_range",
            Self::SourceUnavailable => "source_unavailable",
            Self::Cancelled => "cancelled",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceResourceError {
    pub code: SourceResourceErrorCode,
    pub message: String,
}

impl SourceResourceError {
    pub fn new(code: SourceResourceErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for SourceResourceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code.as_str(), self.message)
    }
}

impl std::error::Error for SourceResourceError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::source_target::{MetadataAddress, ResolvedTarget, TargetKind};
    use serde_json::json;

    #[test]
    fn resource_roles_are_a_closed_serializable_allowlist() {
        let roles = [
            ResourceRole::BslModule,
            ResourceRole::ConfigurationDescriptor,
            ResourceRole::MetadataDescriptor,
            ResourceRole::Registration,
            ResourceRole::Form,
            ResourceRole::Dcs,
            ResourceRole::Mxl,
            ResourceRole::Rights,
            ResourceRole::BinaryTemplate,
            ResourceRole::Unknown,
        ];

        assert_eq!(
            serde_json::to_value(roles).unwrap(),
            json!([
                "bslModule",
                "configurationDescriptor",
                "metadataDescriptor",
                "registration",
                "form",
                "dcs",
                "mxl",
                "rights",
                "binaryTemplate",
                "unknown"
            ])
        );
    }

    #[test]
    fn public_resource_values_serialize_without_provider_private_state() {
        let manifest = ResourceManifestPage {
            snapshot_id: "opaque-snapshot".to_string(),
            source_set: "main".to_string(),
            target: ResolvedTarget {
                source_set: "main".to_string(),
                metadata_path: Some(
                    MetadataAddress::parse(
                        crate::domain::source_target::PLATFORM_XML_8_3_27_FORMAT_2_20,
                        "CommonModule.Shared.Module",
                    )
                    .unwrap(),
                ),
                target_kind: TargetKind::Module,
            },
            scope: ResourceScope::SelfOnly,
            completeness: ResourceCompleteness::Complete,
            resources: vec![SourceResource {
                resource_id: "opaque-resource".to_string(),
                role: ResourceRole::BslModule,
                media_type: "text/x-bsl".to_string(),
                size: 12,
                hash: "sha256:0123".to_string(),
                text_profile: Some(TextProfile {
                    encoding: TextEncoding::Utf8,
                    bom_prefix_bytes: 3,
                    eol: EolProfile::Crlf,
                }),
                access: vec![ResourceAccess::Read],
                limits: ResourceLimits {
                    max_read_bytes: SOURCE_READ_LIMIT_MAX,
                },
            }],
            next_cursor: None,
        };

        let value = serde_json::to_value(manifest).unwrap();
        assert_eq!(value["scope"], "self");
        assert_eq!(value["completeness"], "complete");
        assert_eq!(value["resources"][0]["role"], "bslModule");
        assert_eq!(value["resources"][0]["textProfile"]["encoding"], "utf-8");
        assert_eq!(value["resources"][0]["textProfile"]["bomPrefixBytes"], 3);
        assert_eq!(value["resources"][0]["limits"]["maxReadBytes"], 65_536);
        let encoded = value.to_string();
        for forbidden in [
            "path",
            "handle",
            "provider",
            "providerRevision",
            "workspaceRoot",
        ] {
            assert!(
                !encoded.contains(forbidden),
                "leaked {forbidden}: {encoded}"
            );
        }
    }

    #[test]
    fn resource_limits_and_completeness_are_explicit_contract_values() {
        assert_eq!(SOURCE_MANIFEST_RESOURCE_MAX, 100);
        assert_eq!(SOURCE_RESOURCE_PAGE_LIMIT_MAX, 50);
        assert_eq!(SOURCE_READ_LIMIT_MAX, 64 * 1024);
        assert_eq!(SOURCE_SNAPSHOT_TTL_SECONDS, 5 * 60);
        assert_eq!(
            serde_json::to_value([
                ResourceCompleteness::Complete,
                ResourceCompleteness::Partial,
                ResourceCompleteness::Unavailable,
            ])
            .unwrap(),
            json!(["complete", "partial", "unavailable"])
        );
    }
}
