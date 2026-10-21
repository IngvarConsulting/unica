use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;
use std::fmt;
use std::str::FromStr;
use unica_refusal::{RefusalCode, RefusalDetail};
use uuid::{Uuid, Variant, Version};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct InvocationId(Uuid);

impl InvocationId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for InvocationId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TaskId(Uuid);

impl TaskId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for TaskId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DurableIdParseError;

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
pub struct NormalizedArgumentsHash(String);

impl NormalizedArgumentsHash {
    pub fn from_sha256(bytes: [u8; 32]) -> Self {
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
pub struct SafeIdentityHash(String);

impl SafeIdentityHash {
    pub fn from_sha256(bytes: [u8; 32]) -> Self {
        Self(encode_sha256(bytes))
    }

    pub fn as_str(&self) -> &str {
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
pub struct DomainResult {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub at: Option<String>,
    pub summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub changed: Vec<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub next: Vec<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rev: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page: Option<Value>,
    /// Исходный текст предмета (`Body` модуля или метода BSL). Хранится и
    /// подписывается вместе с результатом, но наружу идёт отдельным текстовым
    /// блоком MCP, а не полем конверта.
    #[serde(
        default,
        rename = "sourceText",
        skip_serializing_if = "Option::is_none"
    )]
    pub source_text: Option<String>,
    /// Private, digest-covered coordinates and bounded prefix used by the
    /// host projection. Old completed records omit it without changing bytes.
    #[serde(
        default,
        rename = "sourceTextReadHint",
        skip_serializing_if = "Option::is_none"
    )]
    pub source_text_read_hint: Option<Box<SourceTextReadHint>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct SourceTextReadHint {
    pub from_line: usize,
    pub to_line: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefix: Option<String>,
}

impl DomainResult {
    pub fn success(summary: impl Into<String>) -> Self {
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
            page: None,
            source_text: None,
            source_text_read_hint: None,
        }
    }

    /// Отказ со свободным кодом — только для стенда
    /// `receipt-ledger-test-support`, который проигрывает протокол демона v5.
    /// Этот протокол в продукт не входит, и его коды в канонический словарь
    /// не объявляются. На поверхности код приходит типом:
    /// см. [`Self::canonical_rejection`].
    #[cfg(feature = "receipt-ledger-test-support")]
    pub fn scenario_rejection(at: Option<String>, code: &str, message: impl Into<String>) -> Self {
        let message = message.into();
        let mut result = Self::success(message.clone());
        result.ok = false;
        result.at = at;
        result.diagnostics = vec![serde_json::json!({"code": code, "message": message})];
        result
    }

    pub fn canonical_rejection(
        at: Option<String>,
        code: RefusalCode,
        message: impl Into<String>,
    ) -> Self {
        Self::rejection(at, code, None, message)
    }

    /// Отказ с уточнением: исход выбирает уточнение, а не умолчание кода.
    /// Нужен там, где один код покрывает несколько исходов, — иначе читатель
    /// не может выбрать действие, ради чего исход и вводится.
    pub fn canonical_rejection_detailed(
        at: Option<String>,
        detail: RefusalDetail,
        message: impl Into<String>,
    ) -> Self {
        Self::rejection(at, detail.code(), Some(detail), message)
    }

    fn rejection(
        at: Option<String>,
        code: RefusalCode,
        detail: Option<RefusalDetail>,
        message: impl Into<String>,
    ) -> Self {
        // Исход выводится, а не выбирается рядом с кодом: два закрытых словаря
        // со временем разошлись бы. Уточнение, когда оно есть, перекрывает
        // умолчание кода — так один код обслуживает несколько исходов, не
        // теряя различия.
        let outcome = detail.map_or_else(|| code.outcome(), RefusalDetail::outcome);
        let message = message.into();
        let mut result = Self::success(message.clone());
        result.ok = false;
        result.at = at;
        let mut diagnostic = serde_json::json!({
            "code": code.as_str(),
            "outcome": outcome.as_str(),
            "message": message,
        });
        if let Some(detail) = detail {
            diagnostic["detailCode"] = serde_json::Value::from(detail.as_str());
        }
        result.diagnostics = vec![diagnostic];
        result
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum InvocationStatus {
    Queued,
    Working,
    Completed,
    Failed,
    Cancelled,
}

/// Execution failure code: the workspace actor changed before the call ran or
/// before its result was published.
pub const WORKSPACE_CHANGED_FAILURE: &str = "workspace_changed";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvocationFailure {
    pub code: String,
    pub message: String,
}

impl InvocationFailure {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{DomainResult, InvocationId, NormalizedArgumentsHash, SafeIdentityHash, TaskId};
    use serde_json::{json, Value};

    #[test]
    fn legacy_inventory_size_diagnostic_remains_readable_as_data() {
        use super::DomainResult;
        use unica_refusal::RefusalCode;

        let mut historical = DomainResult::success("historical inventory refusal");
        historical.ok = false;
        historical.diagnostics.push(serde_json::json!({
            "code": RefusalCode::ProviderUnavailable.as_str(),
            "detailCode": "inventory_too_large",
            "outcome": "goElsewhere"
        }));
        let bytes = serde_json::to_vec(&historical).unwrap();
        let decoded: DomainResult = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(decoded, historical);
    }

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
            page: Some(json!({"stoppedBy": "limit"})),
            source_text: None,
            source_text_read_hint: None,
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
                "page",
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

    mod refusal_envelope {
        use super::super::*;
        use unica_refusal::{Outcome, RefusalCode, RefusalDetail};

        #[test]
        fn a_refusal_carries_code_outcome_and_message_and_nothing_else() {
            let result = DomainResult::canonical_rejection(
                Some("main:Catalog.Валюты".to_string()),
                RefusalCode::NotFound,
                "node was not found",
            );
            assert!(!result.ok);
            assert_eq!(result.diagnostics.len(), 1);
            let diagnostic = &result.diagnostics[0];
            assert_eq!(diagnostic["code"], "not_found");
            assert_eq!(diagnostic["outcome"], "fixCall");
            assert_eq!(diagnostic["message"], "node was not found");
            assert!(
                diagnostic.get("detailCode").is_none(),
                "уточнения не просили — поля быть не должно, иначе читатель \
                 не отличит «уточнения нет» от «уточнение пустое»"
            );
            let keys: Vec<&str> = diagnostic
                .as_object()
                .expect("диагностика — объект")
                .keys()
                .map(String::as_str)
                .collect();
            assert_eq!(keys, vec!["code", "outcome", "message"]);
        }

        #[test]
        fn a_detail_overrides_the_default_outcome_of_its_code() {
            // Умолчание `provider_unavailable` — «нужен человек»; уточнение
            // «исходник не читается» отправляет агента чинить предмет.
            assert_eq!(
                RefusalCode::ProviderUnavailable.outcome(),
                Outcome::NeedsHuman
            );
            let result = DomainResult::canonical_rejection_detailed(
                None,
                RefusalDetail::SourceUnreadable,
                "Configuration.xml is not UTF-8",
            );
            let diagnostic = &result.diagnostics[0];
            assert_eq!(diagnostic["code"], "provider_unavailable");
            assert_eq!(diagnostic["detailCode"], "source_unreadable");
            assert_eq!(diagnostic["outcome"], "fixSource");
        }

        #[test]
        fn every_code_answers_with_one_of_the_six_outcomes() {
            for code in RefusalCode::ALL {
                let result = DomainResult::canonical_rejection(None, code, "проба");
                let outcome = result.diagnostics[0]["outcome"]
                    .as_str()
                    .expect("исход обязателен у всякого отказа")
                    .to_string();
                assert!(
                    Outcome::ALL_WIRE_NAMES.contains(&outcome.as_str()),
                    "{code} ответил исходом {outcome} вне закрытого набора"
                );
            }
        }

        #[test]
        fn the_six_outcomes_are_spelled_as_published_on_the_wire() {
            assert_eq!(
                Outcome::ALL_WIRE_NAMES,
                [
                    "retry",
                    "fixCall",
                    "fixSource",
                    "needsHuman",
                    "goElsewhere",
                    "deadEnd"
                ],
            );
            let result =
                DomainResult::canonical_rejection(None, RefusalCode::DeadlineExceeded, "проба");
            assert_eq!(result.diagnostics[0]["outcome"], "retry");
        }

        #[test]
        fn a_pending_dependency_retries_without_claiming_engine_delivery() {
            let result = DomainResult::canonical_rejection_detailed(
                None,
                RefusalDetail::DependencyPending,
                "search index is not ready",
            );
            assert!(!result.ok);
            assert_eq!(result.diagnostics[0]["code"], "provider_unavailable");
            assert_eq!(result.diagnostics[0]["detailCode"], "dependency_pending");
            assert_eq!(result.diagnostics[0]["outcome"], "retry");
        }
    }
}
