use std::collections::BTreeSet;
use std::fmt::{Display, Formatter};

use serde::Serialize;
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct FormatVersion(Vec<u32>);

impl FormatVersion {
    pub(crate) fn parse(raw: &str) -> Result<Self, SourceAdapterError> {
        let parts = raw
            .split('.')
            .map(str::parse::<u32>)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| {
                SourceAdapterError::new(
                    SourceAdapterErrorKind::FormatUnsupported,
                    format!("invalid format version `{raw}`"),
                )
            })?;
        if parts.is_empty() || parts.iter().all(|part| *part == 0) {
            return Err(SourceAdapterError::new(
                SourceAdapterErrorKind::FormatUnsupported,
                format!("invalid format version `{raw}`"),
            ));
        }
        Ok(Self(parts))
    }
}

impl Display for FormatVersion {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        let rendered = self
            .0
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(".");
        formatter.write_str(&rendered)
    }
}

impl Serialize for FormatVersion {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FormatRange {
    pub(crate) min_inclusive: FormatVersion,
    pub(crate) max_inclusive: FormatVersion,
}

impl FormatRange {
    pub(crate) fn exact(version: FormatVersion) -> Self {
        Self {
            min_inclusive: version.clone(),
            max_inclusive: version,
        }
    }

    pub(crate) fn contains(&self, version: &FormatVersion) -> bool {
        self.min_inclusive <= *version && *version <= self.max_inclusive
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum SourceFamily {
    PlatformXml,
    Edt,
    Cf,
    FileDatabase,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum SnapshotConsistency {
    Consistent,
    Partial,
    Changed,
    Unverifiable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum AdapterMaturity {
    Experimental,
    ProbeComplete,
    ReadCompatible,
    SemanticParity,
    WriteSafe,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum SourceAccess {
    ReadOnly,
    ReadWrite,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SourceId(String);

impl SourceId {
    pub(crate) fn new(raw: impl Into<String>) -> Result<Self, SourceAdapterError> {
        let raw = raw.into();
        validate_source_value(&raw, "source id")?;
        Ok(Self(raw))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

pub(crate) fn source_id_for_configured_source_set(
    source_set: &str,
) -> Result<SourceId, SourceAdapterError> {
    if source_set.is_empty() {
        return Err(SourceAdapterError::new(
            SourceAdapterErrorKind::SourceUnavailable,
            "configured source set has no logical token",
        ));
    }
    let safe = source_set != "."
        && source_set != ".."
        && !source_set.starts_with("encoded-")
        && source_set.bytes().enumerate().all(|(index, byte)| {
            (byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
                && !(index == 0 && matches!(byte, b'-' | b'.'))
        });
    if safe {
        return SourceId::new(format!("workspace:{source_set}"));
    }
    let mut digest = Sha256::new();
    digest.update(b"unica:workspace-source-id:v1\0");
    digest.update((source_set.len() as u64).to_be_bytes());
    digest.update(source_set.as_bytes());
    SourceId::new(format!("workspace:encoded-{:x}", digest.finalize()))
}

impl Serialize for SourceId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SourceRevision(String);

impl SourceRevision {
    pub(crate) fn new(raw: impl Into<String>) -> Result<Self, SourceAdapterError> {
        let raw = raw.into();
        validate_source_value(&raw, "source revision")?;
        Ok(Self(raw))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl Serialize for SourceRevision {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct TargetIdentity(String);

impl TargetIdentity {
    pub(crate) fn from_normalized_relative_path(path: &str) -> Result<Self, SourceAdapterError> {
        if path.is_empty()
            || path.chars().any(char::is_control)
            || path.contains('\\')
            || path
                .split('/')
                .any(|part| part.is_empty() || matches!(part, "." | ".."))
        {
            return Err(SourceAdapterError::new(
                SourceAdapterErrorKind::SourceUnavailable,
                "target path is not a normalized source-root-relative path",
            ));
        }
        let mut digest = Sha256::new();
        digest.update(b"unica:platform-xml-target:v1\0");
        digest.update((path.len() as u64).to_be_bytes());
        digest.update(path.as_bytes());
        Ok(Self(format!("target:sha256:{:x}", digest.finalize())))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct SourceBinding {
    pub(crate) source_id: SourceId,
    pub(crate) family: SourceFamily,
    pub(crate) format: Option<FormatVersion>,
    pub(crate) target_identity: TargetIdentity,
    pub(crate) revision: SourceRevision,
}

impl SourceBinding {
    pub(crate) fn new(
        source_id: SourceId,
        family: SourceFamily,
        format: Option<FormatVersion>,
        target_identity: TargetIdentity,
        revision: SourceRevision,
    ) -> Self {
        Self {
            source_id,
            family,
            format,
            target_identity,
            revision,
        }
    }
}

fn validate_source_value(raw: &str, value_name: &str) -> Result<(), SourceAdapterError> {
    if raw.is_empty() || raw.chars().any(char::is_control) {
        return Err(SourceAdapterError::new(
            SourceAdapterErrorKind::SourceUnavailable,
            format!("invalid {value_name}"),
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SnapshotEvidence {
    pub(crate) revision: SourceRevision,
    pub(crate) root_descriptor_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SourceDescriptor {
    pub(crate) source_id: SourceId,
    pub(crate) family: SourceFamily,
    pub(crate) format_version: FormatVersion,
    pub(crate) producer_version: Option<FormatVersion>,
    pub(crate) detected_features: BTreeSet<String>,
    pub(crate) probe_evidence: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) snapshot_evidence: Option<SnapshotEvidence>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SourceSnapshot {
    pub(crate) source_id: SourceId,
    pub(crate) revision: SourceRevision,
    pub(crate) consistency: SnapshotConsistency,
    pub(crate) adapter_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AdapterManifest {
    pub(crate) adapter_id: &'static str,
    pub(crate) adapter_version: &'static str,
    pub(crate) source_family: SourceFamily,
    pub(crate) supported_formats: Vec<FormatRange>,
    pub(crate) required_features: BTreeSet<String>,
    pub(crate) excluded_features: BTreeSet<String>,
    pub(crate) source_access: SourceAccess,
    pub(crate) maturity: AdapterMaturity,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum SourceAdapterErrorKind {
    SourceUnavailable,
    ProbeAmbiguous,
    FormatUnsupported,
    SnapshotInconsistent,
    SnapshotStale,
    DecodeCorrupted,
    ProjectionAmbiguous,
    IdentityCollision,
    CapabilityBlocked,
    MutationConflict,
    ValidationFailed,
    RecoveryRequired,
    ResourceLimit,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SourceAdapterError {
    pub(crate) kind: SourceAdapterErrorKind,
    pub(crate) message: String,
}

impl SourceAdapterError {
    pub(crate) fn new(kind: SourceAdapterErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub(crate) fn code(&self) -> &'static str {
        match self.kind {
            SourceAdapterErrorKind::SourceUnavailable => "source_unavailable",
            SourceAdapterErrorKind::ProbeAmbiguous => "probe_ambiguous",
            SourceAdapterErrorKind::FormatUnsupported => "format_unsupported",
            SourceAdapterErrorKind::SnapshotInconsistent => "snapshot_inconsistent",
            SourceAdapterErrorKind::SnapshotStale => "snapshot_stale",
            SourceAdapterErrorKind::DecodeCorrupted => "decode_corrupted",
            SourceAdapterErrorKind::ProjectionAmbiguous => "projection_ambiguous",
            SourceAdapterErrorKind::IdentityCollision => "identity_collision",
            SourceAdapterErrorKind::CapabilityBlocked => "capability_blocked",
            SourceAdapterErrorKind::MutationConflict => "mutation_conflict",
            SourceAdapterErrorKind::ValidationFailed => "validation_failed",
            SourceAdapterErrorKind::RecoveryRequired => "recovery_required",
            SourceAdapterErrorKind::ResourceLimit => "resource_limit",
        }
    }
}

impl Display for SourceAdapterError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for SourceAdapterError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_ranges_are_explicit_and_do_not_select_nearest_versions() {
        let range = FormatRange::exact(FormatVersion::parse("2.20").unwrap());

        assert!(range.contains(&FormatVersion::parse("2.20").unwrap()));
        assert!(!range.contains(&FormatVersion::parse("2.19").unwrap()));
        assert!(!range.contains(&FormatVersion::parse("2.21").unwrap()));
    }

    #[test]
    fn invalid_format_versions_are_structured_failures() {
        let error = FormatVersion::parse("2.latest").unwrap_err();

        assert_eq!(error.kind, SourceAdapterErrorKind::FormatUnsupported);
        assert_eq!(error.code(), "format_unsupported");
    }

    #[test]
    fn snapshot_serialization_does_not_expose_physical_locations() {
        let snapshot = SourceSnapshot {
            source_id: SourceId::new("workspace:main").unwrap(),
            revision: SourceRevision::new("sha256:abc").unwrap(),
            consistency: SnapshotConsistency::Consistent,
            adapter_id: "platform-xml-2.20".to_string(),
        };
        let value = serde_json::to_value(snapshot).unwrap();
        let text = value.to_string();

        assert!(!text.contains("/Users/"));
        assert!(!text.contains("C:\\\\"));
    }

    #[test]
    fn configured_source_ids_preserve_safe_ascii_and_encode_unicode_opaque() {
        assert_eq!(
            source_id_for_configured_source_set("main")
                .unwrap()
                .as_str(),
            "workspace:main"
        );
        let first = source_id_for_configured_source_set("Основная").unwrap();
        let second = source_id_for_configured_source_set("Основная").unwrap();
        let other = source_id_for_configured_source_set("основная").unwrap();
        assert_eq!(first, second);
        assert_ne!(first, other);
        assert!(first.as_str().starts_with("workspace:encoded-"));
        assert!(first.as_str().is_ascii());
        assert!(!first.as_str().contains("Основная"));
    }
}
