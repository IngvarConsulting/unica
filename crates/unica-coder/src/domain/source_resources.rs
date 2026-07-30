use crate::domain::source_target::ResolvedTarget;
use serde::{Deserialize, Serialize};
use std::fmt;

pub const SOURCE_MANIFEST_RESOURCE_MAX: usize = 100;
pub const SOURCE_RESOURCE_PAGE_LIMIT_MAX: usize = 50;
pub const SOURCE_READ_LIMIT_MAX: usize = 64 * 1024;
pub const SOURCE_REPLACEMENT_MAX_BYTES: usize = 1024 * 1024;
pub const SOURCE_SNAPSHOT_TTL_SECONDS: u64 = 5 * 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceChangedRange {
    pub start_byte: usize,
    pub end_byte: usize,
    pub start_line: usize,
    pub start_column: usize,
    pub end_line: usize,
    pub end_column: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceValidationEvidence {
    pub kind: String,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceApplyResult {
    pub snapshot_id: String,
    pub resource_id: String,
    pub source_set: String,
    pub target: ResolvedTarget,
    pub role: ResourceRole,
    pub pre_hash: String,
    pub post_hash: String,
    pub no_op: bool,
    pub changed_ranges: Vec<SourceChangedRange>,
    pub diff: String,
    pub validation: SourceValidationEvidence,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceResourceErrorCode {
    SnapshotNotFound,
    SnapshotExpired,
    SnapshotIncomplete,
    SnapshotScopeMismatch,
    ResourceNotFound,
    ResourceNotReadable,
    ResourceNotReplaceable,
    InvalidCursor,
    InvalidRequest,
    LimitExceeded,
    ContentTooLarge,
    SnapshotCapacityExceeded,
    OffsetOutOfRange,
    StaleRevision,
    HashMismatch,
    ContainmentDenied,
    SupportDenied,
    FormatDenied,
    ValidationFailed,
    AtomicityUnproven,
    IntegrityFailed,
    SourceUnavailable,
    Cancelled,
}

impl SourceResourceErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SnapshotNotFound => "snapshot_not_found",
            Self::SnapshotExpired => "snapshot_expired",
            Self::SnapshotIncomplete => "snapshot_incomplete",
            Self::SnapshotScopeMismatch => "snapshot_scope_mismatch",
            Self::ResourceNotFound => "resource_not_found",
            Self::ResourceNotReadable => "resource_not_readable",
            Self::ResourceNotReplaceable => "resource_not_replaceable",
            Self::InvalidCursor => "invalid_cursor",
            Self::InvalidRequest => "invalid_request",
            Self::LimitExceeded => "limit_exceeded",
            Self::ContentTooLarge => "content_too_large",
            Self::SnapshotCapacityExceeded => "snapshot_capacity_exceeded",
            Self::OffsetOutOfRange => "offset_out_of_range",
            Self::StaleRevision => "stale_revision",
            Self::HashMismatch => "hash_mismatch",
            Self::ContainmentDenied => "containment_denied",
            Self::SupportDenied => "support_denied",
            Self::FormatDenied => "format_denied",
            Self::ValidationFailed => "validation_failed",
            Self::AtomicityUnproven => "atomicity_unproven",
            Self::IntegrityFailed => "integrity_failed",
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
        assert_eq!(
            value,
            json!({
                "snapshotId": "opaque-snapshot",
                "sourceSet": "main",
                "target": {
                    "sourceSet": "main",
                    "metadataPath": "CommonModule.Shared.Module",
                    "targetKind": "module"
                },
                "scope": "self",
                "completeness": "complete",
                "resources": [{
                    "resourceId": "opaque-resource",
                    "role": "bslModule",
                    "mediaType": "text/x-bsl",
                    "size": 12,
                    "hash": "sha256:0123",
                    "textProfile": {
                        "encoding": "utf-8",
                        "bomPrefixBytes": 3,
                        "eol": "crlf"
                    },
                    "access": ["read"],
                    "limits": {
                        "maxReadBytes": 65_536
                    }
                }]
            })
        );
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

    #[test]
    fn source_apply_result_is_path_free_and_reports_exact_plan_evidence() {
        let result = SourceApplyResult {
            snapshot_id: "opaque-snapshot".to_string(),
            resource_id: "opaque-resource".to_string(),
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
            role: ResourceRole::BslModule,
            pre_hash: "sha256:before".to_string(),
            post_hash: "sha256:after".to_string(),
            no_op: false,
            changed_ranges: vec![SourceChangedRange {
                start_byte: 10,
                end_byte: 20,
                start_line: 2,
                start_column: 1,
                end_line: 3,
                end_column: 1,
            }],
            diff: "--- a/CommonModule.Shared.Module\n+++ b/CommonModule.Shared.Module\n"
                .to_string(),
            validation: SourceValidationEvidence {
                kind: "bsl-analyzer-parser".to_string(),
                status: "passed".to_string(),
            },
        };

        let value = serde_json::to_value(result).unwrap();
        assert_eq!(value["role"], "bslModule");
        assert_eq!(value["changedRanges"][0]["startByte"], 10);
        assert_eq!(value["validation"]["status"], "passed");
        let encoded = value.to_string();
        for forbidden in ["path", "handle", "provider", "workspaceRoot"] {
            assert!(
                !encoded.contains(forbidden),
                "source.apply leaked {forbidden}: {encoded}"
            );
        }
    }

    #[test]
    fn source_apply_limits_and_denial_codes_are_stable() {
        assert_eq!(SOURCE_REPLACEMENT_MAX_BYTES, 1024 * 1024);
        for (code, expected) in [
            (
                SourceResourceErrorCode::SnapshotIncomplete,
                "snapshot_incomplete",
            ),
            (
                SourceResourceErrorCode::ResourceNotReplaceable,
                "resource_not_replaceable",
            ),
            (SourceResourceErrorCode::StaleRevision, "stale_revision"),
            (SourceResourceErrorCode::HashMismatch, "hash_mismatch"),
            (
                SourceResourceErrorCode::ContentTooLarge,
                "content_too_large",
            ),
            (
                SourceResourceErrorCode::ContainmentDenied,
                "containment_denied",
            ),
            (SourceResourceErrorCode::SupportDenied, "support_denied"),
            (SourceResourceErrorCode::FormatDenied, "format_denied"),
            (
                SourceResourceErrorCode::ValidationFailed,
                "validation_failed",
            ),
            (
                SourceResourceErrorCode::AtomicityUnproven,
                "atomicity_unproven",
            ),
            (SourceResourceErrorCode::IntegrityFailed, "integrity_failed"),
        ] {
            assert_eq!(code.as_str(), expected);
        }
    }
}
