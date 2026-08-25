use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;
use std::fmt;
use std::str::FromStr;
use std::time::Instant;
use uuid::{Uuid, Variant, Version};

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DurableIdParseError;

impl fmt::Display for DurableIdParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("expected a canonical RFC4122 UUIDv4 identity")
    }
}

impl std::error::Error for DurableIdParseError {}

fn parse_durable_uuid(encoded: &str) -> Result<Uuid, DurableIdParseError> {
    let uuid = Uuid::parse_str(encoded).map_err(|_| DurableIdParseError)?;
    if encoded.len() != 36
        || uuid.hyphenated().to_string() != encoded
        || uuid.get_variant() != Variant::RFC4122
        || uuid.get_version() != Some(Version::Random)
    {
        return Err(DurableIdParseError);
    }
    Ok(uuid)
}

macro_rules! durable_uuid_identity {
    ($identity:ty) => {
        impl fmt::Display for $identity {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::Display::fmt(&self.0.hyphenated(), formatter)
            }
        }

        impl FromStr for $identity {
            type Err = DurableIdParseError;

            fn from_str(encoded: &str) -> Result<Self, Self::Err> {
                parse_durable_uuid(encoded).map(Self)
            }
        }

        impl Serialize for $identity {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: Serializer,
            {
                serializer.collect_str(self)
            }
        }

        impl<'de> Deserialize<'de> for $identity {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                let encoded = String::deserialize(deserializer)?;
                encoded.parse().map_err(serde::de::Error::custom)
            }
        }
    };
}

durable_uuid_identity!(InvocationId);
durable_uuid_identity!(TaskId);

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

    pub(crate) fn as_str(&self) -> &str {
        &self.0
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

    pub(crate) fn canonical_rejection(
        at: Option<String>,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        let code = code.into();
        let message = message.into();
        let mut result = Self::success(message.clone());
        result.ok = false;
        result.at = at;
        result.diagnostics = vec![serde_json::json!({"code": code, "message": message})];
        result
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
    /// Restart-stable persisted Task timestamps. The monotonic fields above
    /// remain local state-machine evidence and are never projected onto MCP.
    pub(crate) created_at_epoch_ms: u64,
    pub(crate) updated_at_epoch_ms: u64,
    pub(crate) ttl_ms: u64,
    pub(crate) poll_interval_ms: u64,
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
        DeliveryResume, DomainResult, IndexResume, InvocationId, NormalizedArgumentsHash,
        ProviderResume, ResumeDescriptor, RuntimeResume, SafeIdentityHash, TaskId,
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

    #[test]
    fn durable_ids_round_trip_as_canonical_uuid_v4_strings() {
        let invocation_id = InvocationId::new();
        let invocation_json = serde_json::to_value(invocation_id).unwrap();
        let invocation_text = invocation_json.as_str().unwrap().to_string();
        assert_eq!(invocation_text.len(), 36);
        assert_eq!(invocation_text, invocation_text.to_ascii_lowercase());
        assert_eq!(
            serde_json::from_value::<InvocationId>(invocation_json).unwrap(),
            invocation_id
        );
        assert_eq!(invocation_id.to_string(), invocation_text);

        let task_id = TaskId::new();
        let task_json = serde_json::to_value(task_id).unwrap();
        let task_text = task_json.as_str().unwrap().to_string();
        assert_eq!(task_text.len(), 36);
        assert_eq!(task_text, task_text.to_ascii_lowercase());
        assert_eq!(
            serde_json::from_value::<TaskId>(task_json).unwrap(),
            task_id
        );
        assert_eq!(task_id.to_string(), task_text);
    }

    #[test]
    fn durable_ids_reject_noncanonical_non_v4_and_malformed_values() {
        for encoded in [
            "00000000-0000-0000-0000-000000000000",
            "550e8400-e29b-11d4-a716-446655440000",
            "6ba7b810-9dad-31d1-80b4-00c04fd430c8",
            "550E8400-E29B-41D4-A716-446655440000",
            "550e8400e29b41d4a716446655440000",
            "not-a-uuid",
        ] {
            assert!(
                encoded.parse::<InvocationId>().is_err(),
                "accepted {encoded}"
            );
            assert!(encoded.parse::<TaskId>().is_err(), "accepted {encoded}");
            assert!(serde_json::from_value::<InvocationId>(json!(encoded)).is_err());
            assert!(serde_json::from_value::<TaskId>(json!(encoded)).is_err());
        }
    }
}
