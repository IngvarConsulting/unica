use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use std::time::Instant;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct InvocationId(Uuid);

impl InvocationId {
    pub(crate) fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct TaskId(Uuid);

impl TaskId {
    pub(crate) fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

fn encode_sha256(bytes: [u8; 32]) -> String {
    let mut encoded = String::with_capacity(64);
    for byte in bytes {
        use std::fmt::Write as _;
        write!(&mut encoded, "{byte:02x}").expect("writing to a String cannot fail");
    }
    encoded
}

fn deserialize_sha256<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let encoded = String::deserialize(deserializer)?;
    let normalized = encoded.len() == 64
        && encoded
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
    normalized
        .then_some(encoded)
        .ok_or_else(|| serde::de::Error::custom("expected a normalized lowercase SHA-256 digest"))
}

/// The identity of normalized arguments. Raw argument data has no place in the
/// invocation model, so callers can construct this value only from a digest.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub(crate) struct NormalizedArgumentsHash(String);

impl NormalizedArgumentsHash {
    pub(crate) fn from_sha256(bytes: [u8; 32]) -> Self {
        Self(encode_sha256(bytes))
    }
}

impl<'de> Deserialize<'de> for NormalizedArgumentsHash {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserialize_sha256(deserializer).map(Self)
    }
}

/// An opaque identity suitable for persistence in a resume descriptor.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub(crate) struct SafeIdentityHash(String);

impl SafeIdentityHash {
    pub(crate) fn from_sha256(bytes: [u8; 32]) -> Self {
        Self(encode_sha256(bytes))
    }
}

impl<'de> Deserialize<'de> for SafeIdentityHash {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserialize_sha256(deserializer).map(Self)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DomainResult {
    pub(crate) ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) at: Option<String>,
    pub(crate) summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) data: Option<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) changed: Vec<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) warnings: Vec<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) diagnostics: Vec<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) artifacts: Vec<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) next: Vec<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) rev: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) cursor: Option<String>,
}

impl DomainResult {
    pub(crate) fn success(summary: impl Into<String>) -> Self {
        Self {
            ok: true,
            at: None,
            summary: summary.into(),
            data: None,
            changed: Vec::new(),
            warnings: Vec::new(),
            diagnostics: Vec::new(),
            artifacts: Vec::new(),
            next: Vec::new(),
            rev: None,
            cursor: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct DeliveryResume {
    work_identity_hash: SafeIdentityHash,
}

impl DeliveryResume {
    pub(crate) fn new(work_identity_hash: SafeIdentityHash) -> Self {
        Self { work_identity_hash }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct IndexResume {
    workspace_identity_hash: SafeIdentityHash,
    source_revision_hash: SafeIdentityHash,
}

impl IndexResume {
    pub(crate) fn new(
        workspace_identity_hash: SafeIdentityHash,
        source_revision_hash: SafeIdentityHash,
    ) -> Self {
        Self {
            workspace_identity_hash,
            source_revision_hash,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ProviderResume {
    work_identity_hash: SafeIdentityHash,
}

impl ProviderResume {
    pub(crate) fn new(work_identity_hash: SafeIdentityHash) -> Self {
        Self { work_identity_hash }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RuntimeResume {
    work_identity_hash: SafeIdentityHash,
}

impl RuntimeResume {
    pub(crate) fn new(work_identity_hash: SafeIdentityHash) -> Self {
        Self { work_identity_hash }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub(crate) enum ResumeDescriptor {
    Delivery(DeliveryResume),
    Index(IndexResume),
    Provider(ProviderResume),
    Runtime(RuntimeResume),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum InvocationStatus {
    Queued,
    Working,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct InvocationFailure {
    pub(crate) code: String,
    pub(crate) message: String,
}

impl InvocationFailure {
    pub(crate) fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TaskSnapshot {
    pub(crate) task_id: TaskId,
    pub(crate) invocation_id: InvocationId,
    pub(crate) status: InvocationStatus,
    pub(crate) result: Option<DomainResult>,
    pub(crate) failure: Option<InvocationFailure>,
    pub(crate) resume: Option<ResumeDescriptor>,
    pub(crate) created_at: Instant,
    pub(crate) updated_at: Instant,
}

impl TaskSnapshot {
    pub(crate) fn terminal_result(&self) -> Option<&DomainResult> {
        (self.status == InvocationStatus::Completed)
            .then_some(self.result.as_ref())
            .flatten()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum InvocationOutcome {
    Direct(DomainResult),
    Task(TaskSnapshot),
}

impl InvocationOutcome {
    pub(crate) fn terminal_result(&self) -> Option<&DomainResult> {
        match self {
            Self::Direct(result) => Some(result),
            Self::Task(task) => task.terminal_result(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DeliveryResume, DomainResult, IndexResume, NormalizedArgumentsHash, ProviderResume,
        ResumeDescriptor, RuntimeResume, SafeIdentityHash,
    };
    use serde_json::{json, Value};

    #[test]
    fn empty_optional_result_slots_are_omitted_from_the_canonical_envelope() {
        let result = DomainResult::success("ready");

        assert_eq!(
            serde_json::to_value(result).expect("serialize domain result"),
            json!({"ok": true, "summary": "ready"})
        );
    }

    #[test]
    fn canonical_result_uses_only_the_approved_v013_slots() {
        let result = DomainResult {
            ok: true,
            at: Some("source://main/Catalog.Items".to_string()),
            summary: "applied".to_string(),
            data: Some(json!({"kind": "catalog"})),
            changed: vec![json!({"at": "source://main/Catalog.Items"})],
            warnings: vec![json!({"code": "support_warning"})],
            diagnostics: vec![json!({"severity": "warning"})],
            artifacts: vec![json!({"kind": "cf"})],
            next: vec![json!({"op": "view"})],
            rev: Some("rev-2".to_string()),
            cursor: Some("cursor-2".to_string()),
        };

        let serialized = serde_json::to_value(result).expect("serialize domain result");
        assert_eq!(
            serialized.as_object().unwrap().keys().collect::<Vec<_>>(),
            [
                "ok",
                "at",
                "summary",
                "data",
                "changed",
                "warnings",
                "diagnostics",
                "artifacts",
                "next",
                "rev",
                "cursor",
            ]
        );
        for forbidden in ["set", "sourceState", "fileExists", "job", "work"] {
            assert!(
                serialized.get(forbidden).is_none(),
                "forbidden slot {forbidden}"
            );
        }
    }

    #[test]
    fn canonical_result_rejects_legacy_slots_during_deserialization() {
        let with_legacy_job = json!({
            "ok": true,
            "summary": "ready",
            "job": {"id": "legacy"}
        });

        assert!(serde_json::from_value::<DomainResult>(with_legacy_job).is_err());
    }

    #[test]
    fn invocation_identity_accepts_a_normalized_hash_instead_of_raw_arguments() {
        let normalized = NormalizedArgumentsHash::from_sha256([0x2a; 32]);

        assert_eq!(
            serde_json::to_value(normalized).expect("serialize normalized hash"),
            Value::String("2a".repeat(32))
        );
    }

    #[test]
    fn persisted_identity_hashes_reject_raw_or_non_normalized_strings() {
        for unsafe_value in [
            "{\"raw\":\"arguments\"}",
            "https://user:password@example.invalid/tool",
            &"AA".repeat(32),
        ] {
            assert!(
                serde_json::from_value::<NormalizedArgumentsHash>(json!(unsafe_value)).is_err(),
                "normalized argument hash accepted {unsafe_value}"
            );
            assert!(
                serde_json::from_value::<SafeIdentityHash>(json!(unsafe_value)).is_err(),
                "safe identity hash accepted {unsafe_value}"
            );
        }
    }

    #[test]
    fn resume_descriptors_are_closed_and_contain_only_safe_typed_hashes() {
        let identity = || SafeIdentityHash::from_sha256([0x11; 32]);
        let descriptors = [
            ResumeDescriptor::Delivery(DeliveryResume::new(identity())),
            ResumeDescriptor::Index(IndexResume::new(identity(), identity())),
            ResumeDescriptor::Provider(ProviderResume::new(identity())),
            ResumeDescriptor::Runtime(RuntimeResume::new(identity())),
        ];

        let serialized = serde_json::to_value(descriptors).expect("serialize resume descriptors");
        let text = serialized.to_string();
        for forbidden in ["command", "credential", "password", "url", "path", "args"] {
            assert!(!text.contains(forbidden), "unsafe resume slot {forbidden}");
        }
        assert_eq!(
            serialized,
            json!([
                {"kind": "delivery", "workIdentityHash": "11".repeat(32)},
                {
                    "kind": "index",
                    "workspaceIdentityHash": "11".repeat(32),
                    "sourceRevisionHash": "11".repeat(32)
                },
                {"kind": "provider", "workIdentityHash": "11".repeat(32)},
                {"kind": "runtime", "workIdentityHash": "11".repeat(32)}
            ])
        );
    }

    #[test]
    fn resume_descriptors_reject_extra_unsafe_payload_slots() {
        let with_command = json!({
            "kind": "runtime",
            "workIdentityHash": "11".repeat(32),
            "command": ["runner", "--password", "secret"]
        });

        assert!(serde_json::from_value::<ResumeDescriptor>(with_command).is_err());
    }
}
