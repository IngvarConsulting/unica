use crate::application::invocation_store::{
    MAX_CANONICAL_RESULT_BYTES, MAX_TASK_RECORD_ENVELOPE_BYTES,
};
use crate::application::invocation_store_v5::V5SafeFailureReason;
use crate::domain::invocation::{
    DomainResult, InvocationId, NormalizedArgumentsHash, SafeIdentityHash, TaskId,
};
use serde::{Deserialize, Deserializer, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;
use std::str::FromStr;

const REQUEST_SCOPE_DOMAIN: &[u8] = b"unica.request-scope.v1\0";
const RECEIPT_KEY_DOMAIN: &[u8] = b"unica.receipt-key.v1\0";
const TASK_LINK_DOMAIN: &[u8] = b"unica.task-link.v1\0";
const TERMINAL_OUTCOME_DOMAIN: &[u8] = b"unica.terminal-outcome.v1\0";

/// An application ceiling for the exact request-scope component.
///
/// The outer daemon request has the same 16 KiB ceiling, so a valid envelope
/// can never rely on a larger scope being truncated before hashing.
pub(crate) const MAX_REQUEST_SCOPE_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DigestParseError;

impl fmt::Display for DigestParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("expected a normalized lowercase SHA-256 digest")
    }
}

impl std::error::Error for DigestParseError {}

fn is_normalized_sha256(encoded: &str) -> bool {
    encoded.len() == 64
        && encoded
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn encode_sha256(bytes: [u8; 32]) -> String {
    let mut encoded = String::with_capacity(64);
    for byte in bytes {
        use fmt::Write as _;
        write!(&mut encoded, "{byte:02x}").expect("writing to a String cannot fail");
    }
    encoded
}

macro_rules! checked_digest {
    ($name:ident) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
        #[serde(transparent)]
        pub(crate) struct $name(String);

        impl $name {
            pub(crate) fn as_str(&self) -> &str {
                &self.0
            }

            fn from_digest_bytes(bytes: [u8; 32]) -> Self {
                Self(encode_sha256(bytes))
            }
        }

        impl FromStr for $name {
            type Err = DigestParseError;

            fn from_str(encoded: &str) -> Result<Self, Self::Err> {
                is_normalized_sha256(encoded)
                    .then(|| Self(encoded.to_owned()))
                    .ok_or(DigestParseError)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(self.as_str())
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                String::deserialize(deserializer)?
                    .parse()
                    .map_err(serde::de::Error::custom)
            }
        }
    };
}

checked_digest!(CoreIdentityDigest);
checked_digest!(RequestScopeHash);
checked_digest!(ReceiptKeyDigest);
checked_digest!(TaskLinkDigest);
checked_digest!(TerminalDigest);

impl CoreIdentityDigest {
    pub(crate) fn from_sha256(bytes: [u8; 32]) -> Self {
        Self::from_digest_bytes(bytes)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(crate) enum V5ToolIdentity {
    #[serde(rename = "unica.view")]
    View,
    #[serde(rename = "unica.apply")]
    Apply,
    #[serde(rename = "unica.find")]
    Find,
    #[serde(rename = "unica.search")]
    Search,
    #[serde(rename = "unica.check")]
    Check,
    #[serde(rename = "unica.diff")]
    Diff,
    #[serde(rename = "unica.run")]
    Run,
    #[serde(rename = "unica.docs")]
    Docs,
}

impl V5ToolIdentity {
    pub(crate) const ALL: [Self; 8] = [
        Self::View,
        Self::Apply,
        Self::Find,
        Self::Search,
        Self::Check,
        Self::Diff,
        Self::Run,
        Self::Docs,
    ];

    pub(crate) const fn wire_name(self) -> &'static str {
        match self {
            Self::View => "unica.view",
            Self::Apply => "unica.apply",
            Self::Find => "unica.find",
            Self::Search => "unica.search",
            Self::Check => "unica.check",
            Self::Diff => "unica.diff",
            Self::Run => "unica.run",
            Self::Docs => "unica.docs",
        }
    }

    pub(crate) fn from_wire_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|identity| identity.wire_name() == name)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RequestIdentity {
    core_identity_digest: CoreIdentityDigest,
    tool: V5ToolIdentity,
    normalized_arguments_hash: NormalizedArgumentsHash,
    request_scope_hash: RequestScopeHash,
}

impl RequestIdentity {
    pub(crate) fn new(
        core_identity_digest: CoreIdentityDigest,
        tool: V5ToolIdentity,
        normalized_arguments_hash: NormalizedArgumentsHash,
        request_scope_hash: RequestScopeHash,
    ) -> Self {
        Self {
            core_identity_digest,
            tool,
            normalized_arguments_hash,
            request_scope_hash,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ReceiptKey {
    invocation_id: InvocationId,
    reserved_task_id: TaskId,
    core_identity_digest: CoreIdentityDigest,
    tool: V5ToolIdentity,
    normalized_arguments_hash: NormalizedArgumentsHash,
    request_scope_hash: RequestScopeHash,
}

impl ReceiptKey {
    pub(crate) fn new(
        invocation_id: InvocationId,
        reserved_task_id: TaskId,
        request_identity: RequestIdentity,
    ) -> Self {
        let RequestIdentity {
            core_identity_digest,
            tool,
            normalized_arguments_hash,
            request_scope_hash,
        } = request_identity;
        Self {
            invocation_id,
            reserved_task_id,
            core_identity_digest,
            tool,
            normalized_arguments_hash,
            request_scope_hash,
        }
    }

    pub(crate) fn invocation_id(&self) -> InvocationId {
        self.invocation_id
    }

    pub(crate) fn reserved_task_id(&self) -> TaskId {
        self.reserved_task_id
    }

    pub(crate) fn core_identity_digest(&self) -> &CoreIdentityDigest {
        &self.core_identity_digest
    }

    pub(crate) fn tool(&self) -> V5ToolIdentity {
        self.tool
    }

    pub(crate) fn normalized_arguments_hash(&self) -> &NormalizedArgumentsHash {
        &self.normalized_arguments_hash
    }

    pub(crate) fn request_scope_hash(&self) -> &RequestScopeHash {
        &self.request_scope_hash
    }

    pub(crate) fn request_identity(&self) -> RequestIdentity {
        RequestIdentity::new(
            self.core_identity_digest.clone(),
            self.tool,
            self.normalized_arguments_hash.clone(),
            self.request_scope_hash.clone(),
        )
    }
}

pub(crate) const MAX_ORIGINAL_RESPONSE_BUDGET_MS: u64 = 7_000;
pub(crate) const MAX_LIVE_RECEIPTS: usize = 64;
pub(crate) const MAX_RECEIPT_ENTITLEMENT_BYTES: u64 =
    (MAX_CANONICAL_RESULT_BYTES + MAX_TASK_RECORD_ENVELOPE_BYTES) as u64;
pub(crate) const MAX_LIVE_RECEIPT_BYTES: u64 =
    MAX_RECEIPT_ENTITLEMENT_BYTES * MAX_LIVE_RECEIPTS as u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OriginalCutoffDescriptor {
    accepted_epoch_ms: u64,
    response_budget_ms: u64,
}

impl OriginalCutoffDescriptor {
    pub(crate) fn new(
        accepted_epoch_ms: u64,
        response_budget_ms: u64,
    ) -> Result<Self, OriginalCutoffDescriptorError> {
        if response_budget_ms > MAX_ORIGINAL_RESPONSE_BUDGET_MS {
            return Err(OriginalCutoffDescriptorError::ResponseBudgetTooLarge);
        }
        if accepted_epoch_ms.checked_add(response_budget_ms).is_none() {
            return Err(OriginalCutoffDescriptorError::EpochBudgetOverflow);
        }
        Ok(Self {
            accepted_epoch_ms,
            response_budget_ms,
        })
    }

    pub(crate) const fn accepted_epoch_ms(self) -> u64 {
        self.accepted_epoch_ms
    }

    pub(crate) const fn response_budget_ms(self) -> u64 {
        self.response_budget_ms
    }
}

impl<'de> Deserialize<'de> for OriginalCutoffDescriptor {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct StrictOriginalCutoffDescriptor {
            accepted_epoch_ms: u64,
            response_budget_ms: u64,
        }

        let decoded = StrictOriginalCutoffDescriptor::deserialize(deserializer)?;
        Self::new(decoded.accepted_epoch_ms, decoded.response_budget_ms)
            .map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OriginalCutoffDescriptorError {
    ResponseBudgetTooLarge,
    EpochBudgetOverflow,
}

impl fmt::Display for OriginalCutoffDescriptorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ResponseBudgetTooLarge => "original response budget exceeds 7000 ms",
            Self::EpochBudgetOverflow => "original response cutoff epoch exceeds u64",
        })
    }
}

impl std::error::Error for OriginalCutoffDescriptorError {}

/// Exact readback of the first durable `Reserved::Unbound` transition.
///
/// A duplicate may observe this value but cannot replace its original cutoff,
/// mutation sequence, or fixed result-space entitlement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReservedReceipt {
    key: ReceiptKey,
    key_digest: ReceiptKeyDigest,
    original_cutoff: OriginalCutoffDescriptor,
    cancel_requested: bool,
    mutation_sequence: u64,
    encoded_bytes: u64,
    reserved_result_bytes: u64,
}

impl ReservedReceipt {
    pub(crate) fn new(
        key: ReceiptKey,
        key_digest: ReceiptKeyDigest,
        original_cutoff: OriginalCutoffDescriptor,
        cancel_requested: bool,
        mutation_sequence: u64,
        encoded_bytes: u64,
        reserved_result_bytes: u64,
    ) -> Self {
        Self {
            key,
            key_digest,
            original_cutoff,
            cancel_requested,
            mutation_sequence,
            encoded_bytes,
            reserved_result_bytes,
        }
    }

    pub(crate) fn key(&self) -> &ReceiptKey {
        &self.key
    }

    pub(crate) fn key_digest(&self) -> &ReceiptKeyDigest {
        &self.key_digest
    }

    pub(crate) const fn original_cutoff(&self) -> &OriginalCutoffDescriptor {
        &self.original_cutoff
    }

    pub(crate) const fn cancel_requested(&self) -> bool {
        self.cancel_requested
    }

    pub(crate) const fn mutation_sequence(&self) -> u64 {
        self.mutation_sequence
    }

    pub(crate) const fn encoded_bytes(&self) -> u64 {
        self.encoded_bytes
    }

    pub(crate) const fn reserved_result_bytes(&self) -> u64 {
        self.reserved_result_bytes
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ReserveOutcome {
    Created(ReservedReceipt),
    ExistingExact(ReservedReceipt),
}

impl ReserveOutcome {
    pub(crate) fn reservation(&self) -> &ReservedReceipt {
        match self {
            Self::Created(reservation) | Self::ExistingExact(reservation) => reservation,
        }
    }

    pub(crate) fn into_reservation(self) -> ReservedReceipt {
        match self {
            Self::Created(reservation) | Self::ExistingExact(reservation) => reservation,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TaskLinkIdentity {
    receipt_key_digest: ReceiptKeyDigest,
    task_id: TaskId,
    invocation_id: InvocationId,
    workspace_identity_hash: SafeIdentityHash,
}

impl TaskLinkIdentity {
    pub(crate) fn new(
        receipt_key_digest: ReceiptKeyDigest,
        task_id: TaskId,
        invocation_id: InvocationId,
        workspace_identity_hash: SafeIdentityHash,
    ) -> Self {
        Self {
            receipt_key_digest,
            task_id,
            invocation_id,
            workspace_identity_hash,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum ReceiptTerminalOutcome {
    Completed { result: Box<DomainResult> },
    Failed { reason: V5SafeFailureReason },
    Cancelled,
}

impl<'de> Deserialize<'de> for ReceiptTerminalOutcome {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum StrictTerminalOutcome {
            Completed(StrictCompleted),
            Failed(StrictFailed),
            Cancelled(StrictCancelled),
        }

        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct StrictCompleted {
            status: CompletedStatus,
            result: Box<DomainResult>,
        }

        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct StrictFailed {
            status: FailedStatus,
            reason: V5SafeFailureReason,
        }

        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct StrictCancelled {
            status: CancelledStatus,
        }

        #[derive(Deserialize)]
        enum CompletedStatus {
            #[serde(rename = "completed")]
            Completed,
        }

        #[derive(Deserialize)]
        enum FailedStatus {
            #[serde(rename = "failed")]
            Failed,
        }

        #[derive(Deserialize)]
        enum CancelledStatus {
            #[serde(rename = "cancelled")]
            Cancelled,
        }

        match StrictTerminalOutcome::deserialize(deserializer)? {
            StrictTerminalOutcome::Completed(StrictCompleted { status, result }) => {
                let CompletedStatus::Completed = status;
                Ok(Self::Completed { result })
            }
            StrictTerminalOutcome::Failed(StrictFailed { status, reason }) => {
                let FailedStatus::Failed = status;
                Ok(Self::Failed { reason })
            }
            StrictTerminalOutcome::Cancelled(StrictCancelled { status }) => {
                let CancelledStatus::Cancelled = status;
                Ok(Self::Cancelled)
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct V5CanonicalTerminal {
    outcome: ReceiptTerminalOutcome,
    payload: Vec<u8>,
    digest: TerminalDigest,
}

impl V5CanonicalTerminal {
    pub(crate) fn outcome(&self) -> &ReceiptTerminalOutcome {
        &self.outcome
    }

    pub(crate) fn payload(&self) -> &[u8] {
        &self.payload
    }

    pub(crate) fn digest(&self) -> &TerminalDigest {
        &self.digest
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RequestScopeHashError {
    Empty,
    ControlCharacter,
    TooLong,
}

impl fmt::Display for RequestScopeHashError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Empty => "request scope must not be empty",
            Self::ControlCharacter => "request scope must not contain control characters",
            Self::TooLong => "request scope exceeds the byte limit",
        })
    }
}

impl std::error::Error for RequestScopeHashError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CanonicalTerminalError {
    ResultTooLarge,
    Serialization,
}

impl fmt::Display for CanonicalTerminalError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ResultTooLarge => "canonical DomainResult exceeds the byte limit",
            Self::Serialization => "canonical terminal serialization failed",
        })
    }
}

impl std::error::Error for CanonicalTerminalError {}

fn update_framed(digest: &mut Sha256, bytes: &[u8]) {
    let length = u32::try_from(bytes.len()).expect("bounded identity component fits in u32");
    digest.update(length.to_be_bytes());
    digest.update(bytes);
}

fn normalized_arguments_hash_text(hash: &NormalizedArgumentsHash) -> String {
    match serde_json::to_value(hash).expect("normalized digest serialization cannot fail") {
        serde_json::Value::String(encoded) => encoded,
        _ => unreachable!("normalized digest serializes as a string"),
    }
}

pub(crate) fn request_scope_hash(
    workspace_hint: &str,
) -> Result<RequestScopeHash, RequestScopeHashError> {
    if workspace_hint.is_empty() {
        return Err(RequestScopeHashError::Empty);
    }
    if workspace_hint.chars().any(char::is_control) {
        return Err(RequestScopeHashError::ControlCharacter);
    }
    if workspace_hint.len() > MAX_REQUEST_SCOPE_BYTES {
        return Err(RequestScopeHashError::TooLong);
    }

    let mut digest = Sha256::new();
    digest.update(REQUEST_SCOPE_DOMAIN);
    update_framed(&mut digest, workspace_hint.as_bytes());
    Ok(RequestScopeHash::from_digest_bytes(
        digest.finalize().into(),
    ))
}

pub(crate) fn receipt_key_digest(key: &ReceiptKey) -> ReceiptKeyDigest {
    let invocation_id = key.invocation_id.to_string();
    let reserved_task_id = key.reserved_task_id.to_string();
    let normalized_arguments_hash = normalized_arguments_hash_text(&key.normalized_arguments_hash);
    let mut digest = Sha256::new();
    digest.update(RECEIPT_KEY_DOMAIN);
    for component in [
        invocation_id.as_bytes(),
        reserved_task_id.as_bytes(),
        key.core_identity_digest.as_str().as_bytes(),
        key.tool.wire_name().as_bytes(),
        normalized_arguments_hash.as_bytes(),
        key.request_scope_hash.as_str().as_bytes(),
    ] {
        update_framed(&mut digest, component);
    }
    ReceiptKeyDigest::from_digest_bytes(digest.finalize().into())
}

pub(crate) fn task_link_digest(identity: &TaskLinkIdentity) -> TaskLinkDigest {
    let task_id = identity.task_id.to_string();
    let invocation_id = identity.invocation_id.to_string();
    let mut digest = Sha256::new();
    digest.update(TASK_LINK_DOMAIN);
    for component in [
        identity.receipt_key_digest.as_str().as_bytes(),
        task_id.as_bytes(),
        invocation_id.as_bytes(),
        identity.workspace_identity_hash.as_str().as_bytes(),
    ] {
        update_framed(&mut digest, component);
    }
    TaskLinkDigest::from_digest_bytes(digest.finalize().into())
}

pub(crate) fn canonical_v5_terminal(
    outcome: &ReceiptTerminalOutcome,
) -> Result<V5CanonicalTerminal, CanonicalTerminalError> {
    let payload = match outcome {
        ReceiptTerminalOutcome::Completed { result } => {
            let result =
                serde_json::to_vec(result).map_err(|_| CanonicalTerminalError::Serialization)?;
            if result.len() > MAX_CANONICAL_RESULT_BYTES {
                return Err(CanonicalTerminalError::ResultTooLarge);
            }
            let mut payload = Vec::with_capacity(result.len() + 33);
            payload.extend_from_slice(br#"{"status":"completed","result":"#);
            payload.extend_from_slice(&result);
            payload.push(b'}');
            payload
        }
        ReceiptTerminalOutcome::Failed { reason } => {
            format!(r#"{{"status":"failed","reason":"{}"}}"#, reason.wire_name()).into_bytes()
        }
        ReceiptTerminalOutcome::Cancelled => br#"{"status":"cancelled"}"#.to_vec(),
    };

    let mut digest = Sha256::new();
    digest.update(TERMINAL_OUTCOME_DOMAIN);
    update_framed(&mut digest, &payload);
    Ok(V5CanonicalTerminal {
        outcome: outcome.clone(),
        payload,
        digest: TerminalDigest::from_digest_bytes(digest.finalize().into()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::invocation::{
        DomainResult, InvocationId, NormalizedArgumentsHash, SafeIdentityHash, TaskId,
    };
    use serde_json::json;
    use std::str::FromStr;

    const INVOCATION_ID: &str = "123e4567-e89b-42d3-a456-426614174000";
    const RESERVED_TASK_ID: &str = "123e4567-e89b-42d3-b456-426614174001";

    fn frozen_receipt_key() -> ReceiptKey {
        ReceiptKey::new(
            InvocationId::from_str(INVOCATION_ID).expect("canonical invocation id"),
            TaskId::from_str(RESERVED_TASK_ID).expect("canonical reserved task id"),
            RequestIdentity::new(
                CoreIdentityDigest::from_str(&"00".repeat(32)).expect("core digest"),
                V5ToolIdentity::View,
                NormalizedArgumentsHash::from_sha256([0x11; 32]),
                request_scope_hash("workspace-a").expect("request scope"),
            ),
        )
    }

    #[test]
    fn request_scope_hash_matches_frozen_exact_utf8_vector() {
        let digest = request_scope_hash("workspace-a").expect("valid request scope");

        assert_eq!(
            digest.as_str(),
            "9f7a5a77bb6eb469cd20147a9aeee9d9769a8372f587bd89635d15684ee02b39"
        );
        assert_ne!(
            request_scope_hash(" workspace-a")
                .expect("leading space is exact input")
                .as_str(),
            digest.as_str()
        );
        assert_ne!(
            request_scope_hash("Workspace-a")
                .expect("case is exact input")
                .as_str(),
            digest.as_str()
        );
    }

    #[test]
    fn request_scope_hash_rejects_non_text_and_oversized_scope() {
        assert_eq!(request_scope_hash(""), Err(RequestScopeHashError::Empty));
        assert_eq!(
            request_scope_hash("workspace\nchild"),
            Err(RequestScopeHashError::ControlCharacter)
        );
        assert_eq!(
            request_scope_hash(&"x".repeat(MAX_REQUEST_SCOPE_BYTES + 1)),
            Err(RequestScopeHashError::TooLong)
        );
    }

    #[test]
    fn checked_digest_types_reject_noncanonical_hex() {
        macro_rules! assert_checked_digest {
            ($digest:ty) => {{
                assert!(<$digest>::from_str(&"a".repeat(64)).is_ok());
                assert!(<$digest>::from_str(&"A".repeat(64)).is_err());
                assert!(<$digest>::from_str(&"a".repeat(63)).is_err());
                assert!(serde_json::from_value::<$digest>(json!("g".repeat(64))).is_err());
            }};
        }

        assert_checked_digest!(CoreIdentityDigest);
        assert_checked_digest!(RequestScopeHash);
        assert_checked_digest!(ReceiptKeyDigest);
        assert_checked_digest!(TaskLinkDigest);
        assert_checked_digest!(TerminalDigest);
    }

    #[test]
    fn receipt_key_digest_matches_frozen_six_component_vector() {
        let key = frozen_receipt_key();

        assert_eq!(
            receipt_key_digest(&key).as_str(),
            "9d8f104e7dfb2f4827a24d4b41aefe6c6704bf31bd3df191d84f9893071db549"
        );
    }

    #[test]
    fn receipt_key_serde_is_flat_strict_and_exact() {
        let key = frozen_receipt_key();
        let encoded = serde_json::to_value(&key).expect("receipt key serialization");

        assert_eq!(
            encoded,
            json!({
                "invocationId": INVOCATION_ID,
                "reservedTaskId": RESERVED_TASK_ID,
                "coreIdentityDigest": "00".repeat(32),
                "tool": "unica.view",
                "normalizedArgumentsHash": "11".repeat(32),
                "requestScopeHash": "9f7a5a77bb6eb469cd20147a9aeee9d9769a8372f587bd89635d15684ee02b39"
            })
        );
        assert_eq!(
            serde_json::from_value::<ReceiptKey>(encoded).expect("strict receipt key decode"),
            key
        );
        assert!(serde_json::from_value::<ReceiptKey>(json!({
            "invocationId": INVOCATION_ID,
            "reservedTaskId": RESERVED_TASK_ID,
            "coreIdentityDigest": "00".repeat(32),
            "tool": "unica.view",
            "normalizedArgumentsHash": "11".repeat(32),
            "requestScopeHash": "22".repeat(32),
            "unknown": true
        }))
        .is_err());
    }

    #[test]
    fn every_receipt_key_component_changes_the_digest() {
        let baseline = frozen_receipt_key();
        let baseline_digest = receipt_key_digest(&baseline);
        let alternatives = [
            ReceiptKey::new(
                InvocationId::from_str("123e4567-e89b-42d3-a456-426614174002")
                    .expect("alternate invocation id"),
                baseline.reserved_task_id(),
                baseline.request_identity(),
            ),
            ReceiptKey::new(
                baseline.invocation_id(),
                TaskId::from_str("123e4567-e89b-42d3-b456-426614174003")
                    .expect("alternate reserved task id"),
                baseline.request_identity(),
            ),
            ReceiptKey::new(
                baseline.invocation_id(),
                baseline.reserved_task_id(),
                RequestIdentity::new(
                    CoreIdentityDigest::from_str(&"22".repeat(32)).expect("alternate core"),
                    baseline.tool(),
                    baseline.normalized_arguments_hash().clone(),
                    baseline.request_scope_hash().clone(),
                ),
            ),
            ReceiptKey::new(
                baseline.invocation_id(),
                baseline.reserved_task_id(),
                RequestIdentity::new(
                    baseline.core_identity_digest().clone(),
                    V5ToolIdentity::Find,
                    baseline.normalized_arguments_hash().clone(),
                    baseline.request_scope_hash().clone(),
                ),
            ),
            ReceiptKey::new(
                baseline.invocation_id(),
                baseline.reserved_task_id(),
                RequestIdentity::new(
                    baseline.core_identity_digest().clone(),
                    baseline.tool(),
                    NormalizedArgumentsHash::from_sha256([0x33; 32]),
                    baseline.request_scope_hash().clone(),
                ),
            ),
            ReceiptKey::new(
                baseline.invocation_id(),
                baseline.reserved_task_id(),
                RequestIdentity::new(
                    baseline.core_identity_digest().clone(),
                    baseline.tool(),
                    baseline.normalized_arguments_hash().clone(),
                    request_scope_hash("workspace-b").expect("alternate request scope"),
                ),
            ),
        ];

        for alternative in alternatives {
            assert_ne!(receipt_key_digest(&alternative), baseline_digest);
        }
    }

    #[test]
    fn original_cutoff_descriptor_is_strict_bounded_and_restart_stable() {
        let cutoff =
            OriginalCutoffDescriptor::new(1_234, 7_000).expect("maximum bounded response cutoff");
        assert_eq!(cutoff.accepted_epoch_ms(), 1_234);
        assert_eq!(cutoff.response_budget_ms(), 7_000);
        assert_eq!(
            serde_json::to_value(cutoff).expect("serialize cutoff descriptor"),
            json!({"acceptedEpochMs": 1_234, "responseBudgetMs": 7_000})
        );
        assert_eq!(
            serde_json::from_value::<OriginalCutoffDescriptor>(json!({
                "acceptedEpochMs": 1_234,
                "responseBudgetMs": 7_000
            }))
            .expect("strict cutoff descriptor"),
            cutoff
        );
        assert!(OriginalCutoffDescriptor::new(1_234, 7_001).is_err());
        assert!(OriginalCutoffDescriptor::new(u64::MAX, 1).is_err());
        assert!(serde_json::from_value::<OriginalCutoffDescriptor>(json!({
            "acceptedEpochMs": 1_234,
            "responseBudgetMs": 7_001
        }))
        .is_err());
        assert!(serde_json::from_value::<OriginalCutoffDescriptor>(json!({
            "acceptedEpochMs": u64::MAX,
            "responseBudgetMs": 1
        }))
        .is_err());
        assert!(serde_json::from_value::<OriginalCutoffDescriptor>(json!({
            "acceptedEpochMs": 1_234,
            "responseBudgetMs": 7_000,
            "deadlineMs": 8_000
        }))
        .is_err());
    }

    #[test]
    fn task_link_digest_matches_frozen_stable_identity_vector() {
        let identity = TaskLinkIdentity::new(
            ReceiptKeyDigest::from_str(&"0".repeat(64)).expect("receipt key digest"),
            TaskId::from_str("11111111-1111-4111-8111-111111111111").expect("canonical task id"),
            InvocationId::from_str("22222222-2222-4222-8222-222222222222")
                .expect("canonical invocation id"),
            SafeIdentityHash::from_sha256([0xaa; 32]),
        );

        assert_eq!(
            task_link_digest(&identity).as_str(),
            "4c73d08219973c72e759a9f85e156fa42c9d8e61a56e704b70d1c7c042b73da0"
        );
    }

    #[test]
    fn canonical_cancelled_terminal_matches_frozen_payload_and_digest() {
        let terminal = canonical_v5_terminal(&ReceiptTerminalOutcome::Cancelled)
            .expect("canonical cancelled terminal");

        assert_eq!(terminal.outcome(), &ReceiptTerminalOutcome::Cancelled);
        assert_eq!(terminal.payload(), br#"{"status":"cancelled"}"#);
        assert_eq!(
            terminal.digest().as_str(),
            "f2d0423d2613a0d09397b750542e4542f7653d78ebd5e0448f1326d09145d9ae"
        );
    }

    #[test]
    fn canonical_terminal_uses_exact_variant_key_order_and_strict_union() {
        let completed = ReceiptTerminalOutcome::Completed {
            result: Box::new(DomainResult::success("done")),
        };
        let completed = canonical_v5_terminal(&completed).expect("canonical completed terminal");
        assert_eq!(
            completed.payload(),
            br#"{"status":"completed","result":{"ok":true,"summary":"done"}}"#
        );

        let failed = ReceiptTerminalOutcome::Failed {
            reason: V5SafeFailureReason::OutcomeUncertain,
        };
        let failed = canonical_v5_terminal(&failed).expect("canonical failed terminal");
        assert_eq!(
            failed.payload(),
            br#"{"status":"failed","reason":"outcome_uncertain"}"#
        );

        assert!(serde_json::from_str::<ReceiptTerminalOutcome>(
            r#"{"status":"cancelled","reason":"interrupted"}"#
        )
        .is_err());
        assert!(serde_json::from_str::<ReceiptTerminalOutcome>(
            r#"{"status":"failed","reason":"foreign"}"#
        )
        .is_err());
    }

    #[test]
    fn canonical_completed_terminal_rejects_result_over_eight_mib() {
        let oversized = ReceiptTerminalOutcome::Completed {
            result: Box::new(DomainResult::success(
                "x".repeat(MAX_CANONICAL_RESULT_BYTES),
            )),
        };

        assert_eq!(
            canonical_v5_terminal(&oversized),
            Err(CanonicalTerminalError::ResultTooLarge)
        );
    }
}
