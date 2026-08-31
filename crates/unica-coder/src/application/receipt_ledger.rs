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
use std::num::NonZeroU64;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Instant;

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
checked_digest!(ArtifactSha256);

impl ArtifactSha256 {
    pub(crate) fn from_sha256(bytes: [u8; 32]) -> Self {
        Self::from_digest_bytes(bytes)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ReceiptLedgerError {
    AlreadyOwned,
    DeadlineExceeded,
    InvocationIdentityMismatch,
    ReservedTaskIdentityMismatch,
    ReceiptVersionMismatch {
        expected: ReceiptVersion,
        actual: ReceiptVersion,
    },
    ReceiptMutationSequenceMismatch {
        expected: u64,
        actual: u64,
    },
    TerminalMismatch,
    ReceiptDigestCollision,
    ReceiptNotFound,
    CapacityExceeded,
    TombstoneCapacityExceeded,
    RecordTooLarge,
    TimestampOverflow,
    StoreUnavailable,
    CommitUncertain {
        receipt_key_digest: ReceiptKeyDigest,
    },
    ConcurrentGenerationChange {
        generation_before: u64,
        generation_after: u64,
    },
    Corrupt(&'static str),
    ReceiptRowPresentUnsupported,
    Storage {
        operation: &'static str,
        message: String,
    },
}

impl ReceiptLedgerError {
    pub(crate) const fn requires_reopen(&self) -> bool {
        matches!(
            self,
            Self::StoreUnavailable
                | Self::CommitUncertain { .. }
                | Self::ConcurrentGenerationChange { .. }
                | Self::ReceiptDigestCollision
                | Self::Corrupt(_)
                | Self::Storage { .. }
        )
    }
}

impl fmt::Display for ReceiptLedgerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyOwned => formatter.write_str("receipt ledger is already owned"),
            Self::DeadlineExceeded => formatter.write_str("receipt ledger deadline expired"),
            Self::InvocationIdentityMismatch => {
                formatter.write_str("invocation id belongs to a different receipt key")
            }
            Self::ReservedTaskIdentityMismatch => {
                formatter.write_str("reserved task id belongs to a different receipt key")
            }
            Self::ReceiptVersionMismatch { expected, actual } => write!(
                formatter,
                "receipt record version mismatch: expected {}, actual {}",
                expected.get(),
                actual.get()
            ),
            Self::ReceiptMutationSequenceMismatch { expected, actual } => write!(
                formatter,
                "receipt mutation sequence mismatch: expected {expected}, actual {actual}"
            ),
            Self::TerminalMismatch => {
                formatter.write_str("receipt already owns a different terminal outcome")
            }
            Self::ReceiptDigestCollision => {
                formatter.write_str("receipt key digest belongs to a different exact key")
            }
            Self::ReceiptNotFound => formatter.write_str("receipt was not found"),
            Self::CapacityExceeded => formatter.write_str("receipt ledger capacity is exhausted"),
            Self::TombstoneCapacityExceeded => {
                formatter.write_str("receipt tombstone capacity is exhausted")
            }
            Self::RecordTooLarge => formatter.write_str("receipt record exceeds its byte limit"),
            Self::TimestampOverflow => {
                formatter.write_str("receipt retention timestamp exceeds u64")
            }
            Self::StoreUnavailable => {
                formatter.write_str("receipt ledger requires process-owned recovery")
            }
            Self::CommitUncertain {
                receipt_key_digest,
            } => write!(
                formatter,
                "receipt commit is uncertain for {receipt_key_digest}"
            ),
            Self::ConcurrentGenerationChange {
                generation_before,
                generation_after,
            } => write!(
                formatter,
                "receipt ledger generation changed during exact inspection: {generation_before} -> {generation_after}"
            ),
            Self::Corrupt(message) => write!(formatter, "corrupt receipt ledger: {message}"),
            Self::ReceiptRowPresentUnsupported => {
                formatter.write_str("receipt row exists but record decoding is not implemented")
            }
            Self::Storage { operation, message } => write!(formatter, "{operation}: {message}"),
        }
    }
}

impl std::error::Error for ReceiptLedgerError {}

#[cfg(feature = "receipt-ledger-test-support")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReceiptLedgerCatalogSnapshot {
    generation: u64,
    keys: Vec<ReceiptKey>,
    tombstones: Vec<AcknowledgedTombstoneReceipt>,
    invocation_index: Vec<ReceiptKey>,
    reserved_task_index: Vec<ReceiptKey>,
    live_count: u64,
    actual_bytes: u64,
    reserved_result_bytes: u64,
    tombstone_bytes: u64,
}

#[cfg(feature = "receipt-ledger-test-support")]
impl ReceiptLedgerCatalogSnapshot {
    pub(crate) const fn generation(&self) -> u64 {
        self.generation
    }

    pub(crate) fn keys(&self) -> &[ReceiptKey] {
        &self.keys
    }

    pub(crate) fn tombstones(&self) -> &[AcknowledgedTombstoneReceipt] {
        &self.tombstones
    }

    pub(crate) fn invocation_index(&self) -> &[ReceiptKey] {
        &self.invocation_index
    }

    pub(crate) fn reserved_task_index(&self) -> &[ReceiptKey] {
        &self.reserved_task_index
    }

    pub(crate) const fn live_count(&self) -> u64 {
        self.live_count
    }

    pub(crate) const fn actual_bytes(&self) -> u64 {
        self.actual_bytes
    }

    pub(crate) const fn reserved_result_bytes(&self) -> u64 {
        self.reserved_result_bytes
    }

    pub(crate) const fn tombstone_bytes(&self) -> u64 {
        self.tombstone_bytes
    }
}

/// One-shot construction authority for feature-only catalog telemetry.
///
/// Only the application actor can mint the authority. The concrete store may
/// consume it after observing its complete catalog under the retained writer
/// fence, while callers receive only the validated, read-only snapshot.
#[cfg(feature = "receipt-ledger-test-support")]
pub(crate) struct ReceiptLedgerCatalogSnapshotAuthority {
    _private: (),
}

#[cfg(feature = "receipt-ledger-test-support")]
pub(crate) struct ReceiptLedgerCatalogSnapshotParts {
    generation: u64,
    keys: Vec<ReceiptKey>,
    tombstones: Vec<AcknowledgedTombstoneReceipt>,
    invocation_index: Vec<ReceiptKey>,
    reserved_task_index: Vec<ReceiptKey>,
    live_count: u64,
    actual_bytes: u64,
    reserved_result_bytes: u64,
    tombstone_bytes: u64,
}

#[cfg(feature = "receipt-ledger-test-support")]
impl ReceiptLedgerCatalogSnapshotParts {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        generation: u64,
        keys: Vec<ReceiptKey>,
        tombstones: Vec<AcknowledgedTombstoneReceipt>,
        invocation_index: Vec<ReceiptKey>,
        reserved_task_index: Vec<ReceiptKey>,
        live_count: u64,
        actual_bytes: u64,
        reserved_result_bytes: u64,
        tombstone_bytes: u64,
    ) -> Self {
        Self {
            generation,
            keys,
            tombstones,
            invocation_index,
            reserved_task_index,
            live_count,
            actual_bytes,
            reserved_result_bytes,
            tombstone_bytes,
        }
    }
}

#[cfg(feature = "receipt-ledger-test-support")]
impl ReceiptLedgerCatalogSnapshotAuthority {
    pub(super) const fn new() -> Self {
        Self { _private: () }
    }

    pub(crate) fn seal(
        self,
        parts: ReceiptLedgerCatalogSnapshotParts,
    ) -> Result<ReceiptLedgerCatalogSnapshot, ReceiptLedgerError> {
        let ReceiptLedgerCatalogSnapshotParts {
            generation,
            keys,
            tombstones,
            invocation_index,
            reserved_task_index,
            live_count,
            actual_bytes,
            reserved_result_bytes,
            tombstone_bytes,
        } = parts;
        let observed_count = u64::try_from(keys.len()).map_err(|_| {
            ReceiptLedgerError::Corrupt("receipt catalog count does not fit telemetry")
        })?;
        if live_count != observed_count || keys.len() > MAX_LIVE_RECEIPTS {
            return Err(ReceiptLedgerError::Corrupt(
                "receipt catalog telemetry count contradicts its keys",
            ));
        }
        let tombstone_count = u64::try_from(tombstones.len()).map_err(|_| {
            ReceiptLedgerError::Corrupt("receipt tombstone count does not fit telemetry")
        })?;
        if tombstones.len() > MAX_ACKNOWLEDGED_TOMBSTONES
            || tombstones
                .iter()
                .any(|receipt| receipt.encoded_bytes() > MAX_ACKNOWLEDGED_TOMBSTONE_BYTES)
            || tombstones.iter().try_fold(0_u64, |bytes, receipt| {
                bytes.checked_add(receipt.encoded_bytes())
            }) != Some(tombstone_bytes)
            || tombstone_bytes > MAX_ACKNOWLEDGED_TOMBSTONE_POOL_BYTES
        {
            return Err(ReceiptLedgerError::Corrupt(
                "receipt catalog telemetry contradicts its tombstone pool",
            ));
        }
        let mut indexed_keys = keys.clone();
        indexed_keys.extend(tombstones.iter().map(|receipt| receipt.key().clone()));
        let indexed_count =
            live_count
                .checked_add(tombstone_count)
                .ok_or(ReceiptLedgerError::Corrupt(
                    "receipt catalog telemetry count overflow",
                ))?;
        if u64::try_from(invocation_index.len()).ok() != Some(indexed_count)
            || u64::try_from(reserved_task_index.len()).ok() != Some(indexed_count)
            || !same_exact_receipt_key_set(&indexed_keys, &invocation_index)
            || !same_exact_receipt_key_set(&indexed_keys, &reserved_task_index)
        {
            return Err(ReceiptLedgerError::Corrupt(
                "receipt catalog telemetry indexes contradict its keys",
            ));
        }
        for (offset, key) in indexed_keys.iter().enumerate() {
            let prior = &indexed_keys[..offset];
            if prior
                .iter()
                .any(|candidate| receipt_key_digest(candidate) == receipt_key_digest(key))
            {
                return Err(ReceiptLedgerError::Corrupt(
                    "receipt catalog telemetry contains a duplicate key digest",
                ));
            }
            if prior
                .iter()
                .any(|candidate| candidate.invocation_id() == key.invocation_id())
            {
                return Err(ReceiptLedgerError::Corrupt(
                    "receipt catalog telemetry contains a duplicate invocation id",
                ));
            }
            if prior
                .iter()
                .any(|candidate| candidate.reserved_task_id() == key.reserved_task_id())
            {
                return Err(ReceiptLedgerError::Corrupt(
                    "receipt catalog telemetry contains a duplicate reserved task id",
                ));
            }
        }
        if actual_bytes
            .checked_add(reserved_result_bytes)
            .is_none_or(|bytes| bytes > MAX_LIVE_RECEIPT_BYTES)
        {
            return Err(ReceiptLedgerError::Corrupt(
                "receipt catalog telemetry exceeds its byte entitlement",
            ));
        }
        Ok(ReceiptLedgerCatalogSnapshot {
            generation,
            keys,
            tombstones,
            invocation_index,
            reserved_task_index,
            live_count,
            actual_bytes,
            reserved_result_bytes,
            tombstone_bytes,
        })
    }
}

#[cfg(feature = "receipt-ledger-test-support")]
fn same_exact_receipt_key_set(expected: &[ReceiptKey], observed: &[ReceiptKey]) -> bool {
    expected
        .iter()
        .all(|key| observed.iter().any(|candidate| candidate == key))
}

/// Sole-writer boundary owned by the application actor.
///
/// The port deliberately requires only `Send`: the actor moves one concrete
/// writer to its worker thread and never shares it behind a mutex.
pub(crate) trait ReceiptLedgerPort: Send + 'static {
    #[cfg(feature = "receipt-ledger-test-support")]
    fn snapshot_catalog(
        &mut self,
        _authority: ReceiptLedgerCatalogSnapshotAuthority,
        _deadline: Instant,
    ) -> Result<ReceiptLedgerCatalogSnapshot, ReceiptLedgerError> {
        Err(ReceiptLedgerError::StoreUnavailable)
    }

    fn generation(&mut self, _deadline: Instant) -> Result<u64, ReceiptLedgerError> {
        Err(ReceiptLedgerError::StoreUnavailable)
    }

    fn reserve(
        &mut self,
        key: ReceiptKey,
        original_cutoff: OriginalCutoffDescriptor,
        deadline: Instant,
    ) -> Result<ReserveOutcome, ReceiptLedgerError>;

    fn request_cancel_or_reserve(
        &mut self,
        key: ReceiptKey,
        cancel_reserved_at_epoch_ms: u64,
        deadline: Instant,
    ) -> Result<CancelResolution, ReceiptLedgerError>;

    fn expire_cancel_reserved(
        &mut self,
        key: ReceiptKey,
        expected_version: ReceiptVersion,
        expected_mutation_sequence: u64,
        observed_at_epoch_ms: u64,
        deadline: Instant,
    ) -> Result<CancelExpiryOutcome, ReceiptLedgerError>;

    fn publish_direct_terminal(
        &mut self,
        key: &ReceiptKey,
        expected_version: ReceiptVersion,
        terminal_epoch_ms: u64,
        terminal: V5CanonicalTerminal,
        deadline: Instant,
    ) -> Result<CommittedDirectPublication, ReceiptLedgerError>;

    fn acknowledge_direct(
        &mut self,
        _key: &ReceiptKey,
        _terminal_digest: &TerminalDigest,
        _acknowledged_at_epoch_ms: u64,
        _deadline: Instant,
    ) -> Result<AcknowledgedTombstoneReceipt, ReceiptLedgerError> {
        Err(ReceiptLedgerError::StoreUnavailable)
    }

    fn reclaim_expired_tombstones(
        &mut self,
        _observed_at_epoch_ms: u64,
        _deadline: Instant,
    ) -> Result<usize, ReceiptLedgerError> {
        Err(ReceiptLedgerError::StoreUnavailable)
    }

    fn recover(
        &mut self,
        key: &ReceiptKey,
        deadline: Instant,
    ) -> Result<ReceiptState, ReceiptLedgerError>;

    fn recover_at(
        &mut self,
        key: &ReceiptKey,
        _observed_at_epoch_ms: u64,
        deadline: Instant,
    ) -> Result<ReceiptState, ReceiptLedgerError> {
        self.recover(key, deadline)
    }
}

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
pub(crate) const DIRECT_TERMINAL_RETENTION_MS: u64 = 3_600_000;
pub(crate) const CANCEL_RESERVATION_TTL_MS: u64 = 7_125;
pub(crate) const ACKNOWLEDGED_TOMBSTONE_TTL_MS: u64 = 900_000;
pub(crate) const MAX_ACKNOWLEDGED_TOMBSTONES: usize = 28_864;
pub(crate) const MAX_ACKNOWLEDGED_TOMBSTONE_BYTES: u64 = 512;
pub(crate) const MAX_ACKNOWLEDGED_TOMBSTONE_POOL_BYTES: u64 =
    MAX_ACKNOWLEDGED_TOMBSTONE_BYTES * MAX_ACKNOWLEDGED_TOMBSTONES as u64;

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

/// Per-record CAS version, distinct from the ledger-wide mutation sequence.
///
/// Every newly reserved record starts at one even when another record already
/// advanced the global ledger generation. Future transitions advance only the
/// exact record version they replace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub(crate) struct ReceiptVersion(NonZeroU64);

impl ReceiptVersion {
    pub(crate) const fn initial() -> Self {
        Self(NonZeroU64::MIN)
    }

    pub(crate) const fn new(value: u64) -> Option<Self> {
        match NonZeroU64::new(value) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }

    pub(crate) const fn get(self) -> u64 {
        self.0.get()
    }

    pub(crate) fn checked_next(self) -> Option<Self> {
        self.get().checked_add(1).and_then(Self::new)
    }

    pub(crate) fn checked_previous(self) -> Option<Self> {
        self.get().checked_sub(1).and_then(Self::new)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ReservedPhase {
    Unbound,
    ActorBound {
        bound_workspace_identity: SafeIdentityHash,
    },
    Begun {
        bound_workspace_identity: SafeIdentityHash,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AttemptPhase {
    NotBegun,
    Begun,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReceiptStateKind {
    CancelReserved,
    ReservedUnbound,
    ReservedActorBound,
    ReservedBegun,
    DirectTerminalUnacked,
    AcknowledgedTombstone,
    TaskPromisedUnbound,
    TaskPromisedActorBound,
    TaskHandoffActorBoundNotBegun,
    TaskHandoffActorBoundBegun,
    TaskReceiptOwnedActorBound,
    TaskTerminalReceiptBacked,
    TaskBoundNotBegun,
    TaskBoundBegun,
    TaskTerminalBound,
    TaskRetirementPending,
}

impl ReceiptStateKind {
    pub(crate) const ALL: [Self; 16] = [
        Self::CancelReserved,
        Self::ReservedUnbound,
        Self::ReservedActorBound,
        Self::ReservedBegun,
        Self::DirectTerminalUnacked,
        Self::AcknowledgedTombstone,
        Self::TaskPromisedUnbound,
        Self::TaskPromisedActorBound,
        Self::TaskHandoffActorBoundNotBegun,
        Self::TaskHandoffActorBoundBegun,
        Self::TaskReceiptOwnedActorBound,
        Self::TaskTerminalReceiptBacked,
        Self::TaskBoundNotBegun,
        Self::TaskBoundBegun,
        Self::TaskTerminalBound,
        Self::TaskRetirementPending,
    ];

    pub(crate) const fn diagnostic_name(self) -> &'static str {
        match self {
            Self::CancelReserved => "cancel_reserved",
            Self::ReservedUnbound => "reserved_unbound",
            Self::ReservedActorBound => "reserved_actor_bound",
            Self::ReservedBegun => "reserved_begun",
            Self::DirectTerminalUnacked => "direct_terminal_unacked",
            Self::AcknowledgedTombstone => "acknowledged_tombstone",
            Self::TaskPromisedUnbound => "task_promised_unbound",
            Self::TaskPromisedActorBound => "task_promised_actor_bound",
            Self::TaskHandoffActorBoundNotBegun => "task_handoff_actor_bound_not_begun",
            Self::TaskHandoffActorBoundBegun => "task_handoff_actor_bound_begun",
            Self::TaskReceiptOwnedActorBound => "task_receipt_owned_actor_bound",
            Self::TaskTerminalReceiptBacked => "task_terminal_receipt_backed",
            Self::TaskBoundNotBegun => "task_bound_not_begun",
            Self::TaskBoundBegun => "task_bound_begun",
            Self::TaskTerminalBound => "task_terminal_bound",
            Self::TaskRetirementPending => "task_retirement_pending",
        }
    }
}

/// Fields required to recover and CAS-replace every durable receipt body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReceiptRecordHeader {
    key: ReceiptKey,
    key_digest: ReceiptKeyDigest,
    record_version: ReceiptVersion,
    mutation_sequence: u64,
    encoded_bytes: u64,
}

impl ReceiptRecordHeader {
    pub(crate) fn new(
        key: ReceiptKey,
        key_digest: ReceiptKeyDigest,
        record_version: ReceiptVersion,
        mutation_sequence: u64,
        encoded_bytes: u64,
    ) -> Self {
        Self {
            key,
            key_digest,
            record_version,
            mutation_sequence,
            encoded_bytes,
        }
    }
}

/// Stable Task projection retained before TaskStore becomes sole owner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReceiptTaskProjection {
    task_id: TaskId,
    invocation_id: InvocationId,
    created_at_epoch_ms: u64,
    updated_at_epoch_ms: u64,
    ttl_ms: u64,
    poll_interval_ms: u64,
    version: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TaskLinkReference {
    identity: TaskLinkIdentity,
    digest: TaskLinkDigest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum V5CertificateProtocolIdentity {
    V5,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProvisionalTaskStatus {
    Queued,
    Working,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum StagedTaskPublicationCase {
    Absent {
        final_task_record_max_bytes: u64,
        task_response_frame_max_bytes: u64,
    },
    ExactProvisional {
        status: ProvisionalTaskStatus,
        version: u64,
        cancel_requested: bool,
        final_task_record_max_bytes: u64,
        task_response_frame_max_bytes: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StagedTaskPublicationCases([StagedTaskPublicationCase; 5]);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StagedTerminalTransferCertificateError {
    InvalidTaskPublicationCases,
    TaskTerminalBoundLinkRecordTooLarge,
}

impl StagedTaskPublicationCases {
    pub(crate) fn new(
        cases: [StagedTaskPublicationCase; 5],
    ) -> Result<Self, StagedTerminalTransferCertificateError> {
        match &cases {
            [StagedTaskPublicationCase::Absent { .. }, StagedTaskPublicationCase::ExactProvisional {
                status: ProvisionalTaskStatus::Queued,
                version: queued_version,
                cancel_requested: false,
                ..
            }, StagedTaskPublicationCase::ExactProvisional {
                status: ProvisionalTaskStatus::Queued,
                version: queued_cancel_version,
                cancel_requested: true,
                ..
            }, StagedTaskPublicationCase::ExactProvisional {
                status: ProvisionalTaskStatus::Working,
                version: working_version,
                cancel_requested: false,
                ..
            }, StagedTaskPublicationCase::ExactProvisional {
                status: ProvisionalTaskStatus::Working,
                version: working_cancel_version,
                cancel_requested: true,
                ..
            }] if [
                *queued_version,
                *queued_cancel_version,
                *working_version,
                *working_cancel_version,
            ] == [u64::MAX; 4] =>
            {
                Ok(Self(cases))
            }
            _ => Err(StagedTerminalTransferCertificateError::InvalidTaskPublicationCases),
        }
    }

    pub(crate) fn as_slice(&self) -> &[StagedTaskPublicationCase] {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum StagedCapacityFallbackCase {
    LinkCapacity {
        receipt_backed_record_max_bytes: u64,
        task_response_frame_max_bytes: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StagedTerminalTransferCertificate {
    certificate_version: u32,
    protocol_identity: V5CertificateProtocolIdentity,
    core_identity_digest: CoreIdentityDigest,
    receipt_key_digest: ReceiptKeyDigest,
    task_id: TaskId,
    invocation_id: InvocationId,
    task_link_digest: TaskLinkDigest,
    terminal_digest: TerminalDigest,
    terminal_epoch_ms: u64,
    receipt_record_schema_version: u32,
    task_record_schema_version: u32,
    lifecycle_link_record_schema_version: u32,
    terminal_codec_version: u32,
    max_daemon_response_line_bytes: u64,
    max_task_lifecycle_link_record_bytes: u64,
    staged_receipt_record_max_bytes: u64,
    task_terminal_bound_link_record_max_bytes: u64,
    task_publication_cases: StagedTaskPublicationCases,
    capacity_fallback_cases: [StagedCapacityFallbackCase; 1],
}

impl StagedTerminalTransferCertificate {
    const CERTIFICATE_VERSION: u32 = 1;
    const RECEIPT_RECORD_SCHEMA_VERSION: u32 = 1;
    const TASK_RECORD_SCHEMA_VERSION: u32 = 1;
    const LIFECYCLE_LINK_RECORD_SCHEMA_VERSION: u32 = 1;
    const TERMINAL_CODEC_VERSION: u32 = 1;
    const MAX_TASK_LIFECYCLE_LINK_RECORD_BYTES: u64 = 1_024;

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        core_identity_digest: CoreIdentityDigest,
        receipt_key_digest: ReceiptKeyDigest,
        task_id: TaskId,
        invocation_id: InvocationId,
        task_link_digest: TaskLinkDigest,
        terminal_digest: TerminalDigest,
        terminal_epoch_ms: u64,
        staged_receipt_record_max_bytes: u64,
        task_terminal_bound_link_record_max_bytes: u64,
        task_publication_cases: [StagedTaskPublicationCase; 5],
        capacity_fallback_cases: [StagedCapacityFallbackCase; 1],
    ) -> Result<Self, StagedTerminalTransferCertificateError> {
        let task_publication_cases = StagedTaskPublicationCases::new(task_publication_cases)?;
        if task_terminal_bound_link_record_max_bytes > Self::MAX_TASK_LIFECYCLE_LINK_RECORD_BYTES {
            return Err(
                StagedTerminalTransferCertificateError::TaskTerminalBoundLinkRecordTooLarge,
            );
        }

        Ok(Self {
            certificate_version: Self::CERTIFICATE_VERSION,
            protocol_identity: V5CertificateProtocolIdentity::V5,
            core_identity_digest,
            receipt_key_digest,
            task_id,
            invocation_id,
            task_link_digest,
            terminal_digest,
            terminal_epoch_ms,
            receipt_record_schema_version: Self::RECEIPT_RECORD_SCHEMA_VERSION,
            task_record_schema_version: Self::TASK_RECORD_SCHEMA_VERSION,
            lifecycle_link_record_schema_version: Self::LIFECYCLE_LINK_RECORD_SCHEMA_VERSION,
            terminal_codec_version: Self::TERMINAL_CODEC_VERSION,
            max_daemon_response_line_bytes: (MAX_CANONICAL_RESULT_BYTES
                + MAX_TASK_RECORD_ENVELOPE_BYTES)
                as u64,
            max_task_lifecycle_link_record_bytes: Self::MAX_TASK_LIFECYCLE_LINK_RECORD_BYTES,
            staged_receipt_record_max_bytes,
            task_terminal_bound_link_record_max_bytes,
            task_publication_cases,
            capacity_fallback_cases,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum HandoffTerminalStage {
    NoTerminal,
    Staged {
        terminal_epoch_ms: u64,
        terminal: V5CanonicalTerminal,
        certificate: Box<StagedTerminalTransferCertificate>,
    },
}

/// Closed evidence for either dimension of the Task/link capacity limit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ProvenTaskLinkCapacity {
    Count {
        observed_live_links: u64,
        maximum_live_links: u64,
    },
    Bytes {
        required_link_bytes: u64,
        available_link_bytes: u64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ClosedTerminalStatus {
    Completed,
    Failed,
    Cancelled,
}

/// Bound-family records are lifecycle-link records, not active receipt rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LifecycleLinkRecordHeader {
    key: ReceiptKey,
    key_digest: ReceiptKeyDigest,
    link: TaskLinkReference,
    lifecycle_link_version: u64,
    mutation_sequence: u64,
    encoded_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RetainedDualIdAccounting {
    invocation_index_bytes: u64,
    reserved_task_index_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CancelReservedReceipt {
    record: ReceiptRecordHeader,
    cancel_reserved_at_epoch_ms: u64,
    expires_at_epoch_ms: u64,
}

impl CancelReservedReceipt {
    pub(crate) fn new(
        key: ReceiptKey,
        record_version: ReceiptVersion,
        mutation_sequence: u64,
        encoded_bytes: u64,
        cancel_reserved_at_epoch_ms: u64,
    ) -> Result<Self, ReceiptLedgerError> {
        let expires_at_epoch_ms = cancel_reserved_at_epoch_ms
            .checked_add(CANCEL_RESERVATION_TTL_MS)
            .ok_or(ReceiptLedgerError::TimestampOverflow)?;
        let key_digest = receipt_key_digest(&key);

        Ok(Self {
            record: ReceiptRecordHeader::new(
                key,
                key_digest,
                record_version,
                mutation_sequence,
                encoded_bytes,
            ),
            cancel_reserved_at_epoch_ms,
            expires_at_epoch_ms,
        })
    }

    pub(crate) fn key(&self) -> &ReceiptKey {
        &self.record.key
    }

    pub(crate) fn key_digest(&self) -> &ReceiptKeyDigest {
        &self.record.key_digest
    }

    pub(crate) const fn record_version(&self) -> ReceiptVersion {
        self.record.record_version
    }

    pub(crate) const fn mutation_sequence(&self) -> u64 {
        self.record.mutation_sequence
    }

    pub(crate) const fn encoded_bytes(&self) -> u64 {
        self.record.encoded_bytes
    }

    pub(crate) const fn cancel_reserved_at_epoch_ms(&self) -> u64 {
        self.cancel_reserved_at_epoch_ms
    }

    pub(crate) const fn expires_at_epoch_ms(&self) -> u64 {
        self.expires_at_epoch_ms
    }

    pub(crate) const fn cancel_requested(&self) -> bool {
        true
    }
}

/// Exact readback of the first durable `Reserved::Unbound` transition.
///
/// A duplicate may observe this value but cannot replace its original cutoff,
/// mutation sequence, or fixed result-space entitlement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReservedReceipt {
    record: ReceiptRecordHeader,
    reserved_at_epoch_ms: u64,
    original_cutoff: OriginalCutoffDescriptor,
    phase: ReservedPhase,
    cancel_requested: bool,
    reserved_result_bytes: u64,
}

impl ReservedReceipt {
    pub(crate) fn new(
        record: ReceiptRecordHeader,
        reserved_at_epoch_ms: u64,
        original_cutoff: OriginalCutoffDescriptor,
        phase: ReservedPhase,
        cancel_requested: bool,
        reserved_result_bytes: u64,
    ) -> Self {
        Self {
            record,
            reserved_at_epoch_ms,
            original_cutoff,
            phase,
            cancel_requested,
            reserved_result_bytes,
        }
    }

    pub(crate) fn key(&self) -> &ReceiptKey {
        &self.record.key
    }

    pub(crate) fn key_digest(&self) -> &ReceiptKeyDigest {
        &self.record.key_digest
    }

    pub(crate) const fn reserved_at_epoch_ms(&self) -> u64 {
        self.reserved_at_epoch_ms
    }

    pub(crate) const fn original_cutoff(&self) -> &OriginalCutoffDescriptor {
        &self.original_cutoff
    }

    pub(crate) const fn phase(&self) -> &ReservedPhase {
        &self.phase
    }

    pub(crate) const fn cancel_requested(&self) -> bool {
        self.cancel_requested
    }

    pub(crate) const fn record_version(&self) -> ReceiptVersion {
        self.record.record_version
    }

    pub(crate) const fn mutation_sequence(&self) -> u64 {
        self.record.mutation_sequence
    }

    pub(crate) const fn encoded_bytes(&self) -> u64 {
        self.record.encoded_bytes
    }

    pub(crate) const fn reserved_result_bytes(&self) -> u64 {
        self.reserved_result_bytes
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DirectTerminalUnackedReceipt {
    record: ReceiptRecordHeader,
    original_cutoff: OriginalCutoffDescriptor,
    terminal_epoch_ms: u64,
    terminal: V5CanonicalTerminal,
    reserved_result_bytes: u64,
}

impl DirectTerminalUnackedReceipt {
    pub(crate) fn new(
        record: ReceiptRecordHeader,
        original_cutoff: OriginalCutoffDescriptor,
        terminal_epoch_ms: u64,
        terminal: V5CanonicalTerminal,
        reserved_result_bytes: u64,
    ) -> Self {
        Self {
            record,
            original_cutoff,
            terminal_epoch_ms,
            terminal,
            reserved_result_bytes,
        }
    }

    pub(crate) fn key(&self) -> &ReceiptKey {
        &self.record.key
    }

    pub(crate) fn key_digest(&self) -> &ReceiptKeyDigest {
        &self.record.key_digest
    }

    pub(crate) const fn record_version(&self) -> ReceiptVersion {
        self.record.record_version
    }

    pub(crate) const fn mutation_sequence(&self) -> u64 {
        self.record.mutation_sequence
    }

    pub(crate) const fn encoded_bytes(&self) -> u64 {
        self.record.encoded_bytes
    }

    pub(crate) const fn original_cutoff(&self) -> &OriginalCutoffDescriptor {
        &self.original_cutoff
    }

    pub(crate) const fn terminal_epoch_ms(&self) -> u64 {
        self.terminal_epoch_ms
    }

    pub(crate) fn terminal(&self) -> &V5CanonicalTerminal {
        &self.terminal
    }

    pub(crate) const fn reserved_result_bytes(&self) -> u64 {
        self.reserved_result_bytes
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AcknowledgedTombstoneReceipt {
    key: ReceiptKey,
    key_digest: ReceiptKeyDigest,
    terminal_digest: TerminalDigest,
    acknowledged_at_epoch_ms: u64,
    encoded_bytes: u64,
}

impl AcknowledgedTombstoneReceipt {
    pub(crate) fn new(
        key: ReceiptKey,
        key_digest: ReceiptKeyDigest,
        terminal_digest: TerminalDigest,
        acknowledged_at_epoch_ms: u64,
        encoded_bytes: u64,
    ) -> Result<Self, ReceiptLedgerError> {
        acknowledged_at_epoch_ms
            .checked_add(ACKNOWLEDGED_TOMBSTONE_TTL_MS)
            .ok_or(ReceiptLedgerError::TimestampOverflow)?;
        Ok(Self {
            key,
            key_digest,
            terminal_digest,
            acknowledged_at_epoch_ms,
            encoded_bytes,
        })
    }

    pub(crate) fn key(&self) -> &ReceiptKey {
        &self.key
    }

    pub(crate) fn key_digest(&self) -> &ReceiptKeyDigest {
        &self.key_digest
    }

    pub(crate) const fn terminal_digest(&self) -> &TerminalDigest {
        &self.terminal_digest
    }

    pub(crate) const fn acknowledged_at_epoch_ms(&self) -> u64 {
        self.acknowledged_at_epoch_ms
    }

    pub(crate) fn expires_at_epoch_ms(&self) -> u64 {
        self.acknowledged_at_epoch_ms + ACKNOWLEDGED_TOMBSTONE_TTL_MS
    }

    pub(crate) const fn encoded_bytes(&self) -> u64 {
        self.encoded_bytes
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TaskPromisedUnboundReceipt {
    record: ReceiptRecordHeader,
    task: ReceiptTaskProjection,
    cancel_requested: bool,
    reserved_result_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TaskPromisedActorBoundReceipt {
    record: ReceiptRecordHeader,
    task: ReceiptTaskProjection,
    link: TaskLinkReference,
    cancel_requested: bool,
    reserved_result_bytes: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TaskHandoffActorBoundReceipt {
    record: ReceiptRecordHeader,
    task: ReceiptTaskProjection,
    link: TaskLinkReference,
    phase: AttemptPhase,
    cancel_requested: bool,
    reserved_result_bytes: u64,
    terminal_stage: HandoffTerminalStage,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TaskReceiptOwnedActorBoundReceipt {
    record: ReceiptRecordHeader,
    task: ReceiptTaskProjection,
    link: TaskLinkReference,
    cancel_requested: bool,
    reserved_result_bytes: u64,
    proven_link_capacity: ProvenTaskLinkCapacity,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TaskTerminalReceiptBackedReceipt {
    record: ReceiptRecordHeader,
    task: ReceiptTaskProjection,
    terminal_epoch_ms: u64,
    terminal: V5CanonicalTerminal,
    cancel_requested: bool,
    reserved_result_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TaskBoundReceipt {
    record: LifecycleLinkRecordHeader,
    task: ReceiptTaskProjection,
    task_record_version: u64,
    bind_epoch_ms: u64,
    phase: AttemptPhase,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TaskTerminalBoundReceipt {
    record: LifecycleLinkRecordHeader,
    task: ReceiptTaskProjection,
    task_record_version: u64,
    terminal_status: ClosedTerminalStatus,
    terminal_digest: TerminalDigest,
    terminal_epoch_ms: u64,
    expires_at_epoch_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TaskRetirementPendingReceipt {
    record: LifecycleLinkRecordHeader,
    task: ReceiptTaskProjection,
    expected_terminal_task_version: u64,
    terminal_status: ClosedTerminalStatus,
    terminal_digest: TerminalDigest,
    terminal_epoch_ms: u64,
    expires_at_epoch_ms: u64,
    retained_link_bytes: u64,
    retained_dual_id_accounting: RetainedDualIdAccounting,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ReceiptState {
    CancelReserved(CancelReservedReceipt),
    Reserved(ReservedReceipt),
    DirectTerminalUnacked(DirectTerminalUnackedReceipt),
    AcknowledgedTombstone(AcknowledgedTombstoneReceipt),
    TaskPromisedUnbound(TaskPromisedUnboundReceipt),
    TaskPromisedActorBound(TaskPromisedActorBoundReceipt),
    TaskHandoffActorBound(TaskHandoffActorBoundReceipt),
    TaskReceiptOwnedActorBound(TaskReceiptOwnedActorBoundReceipt),
    TaskTerminalReceiptBacked(TaskTerminalReceiptBackedReceipt),
    TaskBound(TaskBoundReceipt),
    TaskTerminalBound(TaskTerminalBoundReceipt),
    TaskRetirementPending(TaskRetirementPendingReceipt),
}

impl ReceiptState {
    pub(crate) const fn kind(&self) -> ReceiptStateKind {
        match self {
            Self::CancelReserved(_) => ReceiptStateKind::CancelReserved,
            Self::Reserved(receipt) => match receipt.phase() {
                ReservedPhase::Unbound => ReceiptStateKind::ReservedUnbound,
                ReservedPhase::ActorBound { .. } => ReceiptStateKind::ReservedActorBound,
                ReservedPhase::Begun { .. } => ReceiptStateKind::ReservedBegun,
            },
            Self::DirectTerminalUnacked(_) => ReceiptStateKind::DirectTerminalUnacked,
            Self::AcknowledgedTombstone(_) => ReceiptStateKind::AcknowledgedTombstone,
            Self::TaskPromisedUnbound(_) => ReceiptStateKind::TaskPromisedUnbound,
            Self::TaskPromisedActorBound(_) => ReceiptStateKind::TaskPromisedActorBound,
            Self::TaskHandoffActorBound(receipt) => match receipt.phase {
                AttemptPhase::NotBegun => ReceiptStateKind::TaskHandoffActorBoundNotBegun,
                AttemptPhase::Begun => ReceiptStateKind::TaskHandoffActorBoundBegun,
            },
            Self::TaskReceiptOwnedActorBound(_) => ReceiptStateKind::TaskReceiptOwnedActorBound,
            Self::TaskTerminalReceiptBacked(_) => ReceiptStateKind::TaskTerminalReceiptBacked,
            Self::TaskBound(receipt) => match receipt.phase {
                AttemptPhase::NotBegun => ReceiptStateKind::TaskBoundNotBegun,
                AttemptPhase::Begun => ReceiptStateKind::TaskBoundBegun,
            },
            Self::TaskTerminalBound(_) => ReceiptStateKind::TaskTerminalBound,
            Self::TaskRetirementPending(_) => ReceiptStateKind::TaskRetirementPending,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ReserveOutcome {
    Created(ReservedReceipt),
    ExistingExact(ReceiptState),
}

/// Closed result of the pre-submit cancellation reservation transition.
///
/// The two reservation variants preserve whether this call performed the
/// durable write. A pre-existing non-reservation winner is returned in full so
/// an unacked direct terminal keeps its canonical payload and exact metadata.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum CancelResolution {
    NewlyReserved(CancelReservedReceipt),
    ExistingExact(CancelReservedReceipt),
    ExistingWinner(Box<ReceiptState>),
}

/// Closed result of an explicit cancellation-reservation expiry mutation.
///
/// Expiry is never hidden in a read-only recovery path. A not-yet-due record
/// and any competing durable winner retain their complete exact state.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum CancelExpiryOutcome {
    Expired,
    NotDue(CancelReservedReceipt),
    ExistingWinner(Box<ReceiptState>),
    Missing,
}

impl ReserveOutcome {
    pub(crate) fn reservation(&self) -> Option<&ReservedReceipt> {
        match self {
            Self::Created(reservation) => Some(reservation),
            Self::ExistingExact(ReceiptState::Reserved(reservation)) => Some(reservation),
            Self::ExistingExact(_) => None,
        }
    }

    pub(crate) fn into_reservation(self) -> Result<ReservedReceipt, Box<ReceiptState>> {
        match self {
            Self::Created(reservation) => Ok(reservation),
            Self::ExistingExact(ReceiptState::Reserved(reservation)) => Ok(reservation),
            Self::ExistingExact(state) => Err(Box::new(state)),
        }
    }

    pub(crate) fn into_state(self) -> ReceiptState {
        match self {
            Self::Created(reservation) => ReceiptState::Reserved(reservation),
            Self::ExistingExact(state) => state,
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

#[derive(Clone, PartialEq)]
pub(crate) struct V5CanonicalTerminal {
    outcome: Arc<ReceiptTerminalOutcome>,
    payload: Arc<[u8]>,
    digest: TerminalDigest,
}

impl fmt::Debug for V5CanonicalTerminal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let outcome = match self.outcome.as_ref() {
            ReceiptTerminalOutcome::Completed { .. } => "completed",
            ReceiptTerminalOutcome::Failed { .. } => "failed",
            ReceiptTerminalOutcome::Cancelled => "cancelled",
        };
        formatter
            .debug_struct("V5CanonicalTerminal")
            .field("outcome", &outcome)
            .field("payload_bytes", &self.payload.len())
            .field("digest", &self.digest)
            .finish()
    }
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

    pub(crate) fn outcome_shared(&self) -> Arc<ReceiptTerminalOutcome> {
        Arc::clone(&self.outcome)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReceiptTerminalBinding {
    key: ReceiptKey,
    key_digest: ReceiptKeyDigest,
    expected_version: ReceiptVersion,
    committed_version: ReceiptVersion,
    mutation_sequence: u64,
    original_cutoff: OriginalCutoffDescriptor,
    terminal_epoch_ms: u64,
    terminal_digest: TerminalDigest,
}

impl ReceiptTerminalBinding {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        key: ReceiptKey,
        key_digest: ReceiptKeyDigest,
        expected_version: ReceiptVersion,
        committed_version: ReceiptVersion,
        mutation_sequence: u64,
        original_cutoff: OriginalCutoffDescriptor,
        terminal_epoch_ms: u64,
        terminal_digest: TerminalDigest,
    ) -> Self {
        Self {
            key,
            key_digest,
            expected_version,
            committed_version,
            mutation_sequence,
            original_cutoff,
            terminal_epoch_ms,
            terminal_digest,
        }
    }

    pub(crate) fn key(&self) -> &ReceiptKey {
        &self.key
    }

    pub(crate) fn key_digest(&self) -> &ReceiptKeyDigest {
        &self.key_digest
    }

    pub(crate) const fn expected_version(&self) -> ReceiptVersion {
        self.expected_version
    }

    pub(crate) const fn committed_version(&self) -> ReceiptVersion {
        self.committed_version
    }

    pub(crate) const fn mutation_sequence(&self) -> u64 {
        self.mutation_sequence
    }

    pub(crate) const fn original_cutoff(&self) -> OriginalCutoffDescriptor {
        self.original_cutoff
    }

    pub(crate) const fn terminal_epoch_ms(&self) -> u64 {
        self.terminal_epoch_ms
    }

    pub(crate) fn terminal_digest(&self) -> &TerminalDigest {
        &self.terminal_digest
    }
}

pub(crate) struct PreparedReceiptRecord {
    binding: ReceiptTerminalBinding,
    bytes: Box<[u8]>,
    encoded_bytes: u64,
    reserved_result_bytes: u64,
    sha256: ArtifactSha256,
    terminal: V5CanonicalTerminal,
}

impl fmt::Debug for PreparedReceiptRecord {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PreparedReceiptRecord")
            .field("binding", &self.binding)
            .field("encoded_bytes", &self.encoded_bytes)
            .field("reserved_result_bytes", &self.reserved_result_bytes)
            .field("sha256", &self.sha256)
            .field("terminal", &self.terminal)
            .finish()
    }
}

impl PreparedReceiptRecord {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        binding: ReceiptTerminalBinding,
        bytes: Box<[u8]>,
        encoded_bytes: u64,
        reserved_result_bytes: u64,
        sha256: ArtifactSha256,
        terminal: V5CanonicalTerminal,
    ) -> Self {
        Self {
            binding,
            bytes,
            encoded_bytes,
            reserved_result_bytes,
            sha256,
            terminal,
        }
    }

    pub(crate) fn binding(&self) -> &ReceiptTerminalBinding {
        &self.binding
    }

    pub(crate) fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub(crate) const fn encoded_bytes(&self) -> u64 {
        self.encoded_bytes
    }

    pub(crate) const fn reserved_result_bytes(&self) -> u64 {
        self.reserved_result_bytes
    }

    pub(crate) fn sha256(&self) -> &ArtifactSha256 {
        &self.sha256
    }

    pub(crate) fn terminal(&self) -> &V5CanonicalTerminal {
        &self.terminal
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        ReceiptTerminalBinding,
        Box<[u8]>,
        u64,
        u64,
        ArtifactSha256,
        V5CanonicalTerminal,
    ) {
        (
            self.binding,
            self.bytes,
            self.encoded_bytes,
            self.reserved_result_bytes,
            self.sha256,
            self.terminal,
        )
    }
}

pub(crate) struct PreparedWireFrame {
    binding: ReceiptTerminalBinding,
    jsonl: Box<[u8]>,
    encoded_bytes: u64,
    sha256: ArtifactSha256,
}

impl fmt::Debug for PreparedWireFrame {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PreparedWireFrame")
            .field("binding", &self.binding)
            .field("encoded_bytes", &self.encoded_bytes)
            .field("sha256", &self.sha256)
            .finish()
    }
}

impl PreparedWireFrame {
    pub(crate) fn new(
        binding: ReceiptTerminalBinding,
        jsonl: Box<[u8]>,
        encoded_bytes: u64,
        sha256: ArtifactSha256,
    ) -> Self {
        Self {
            binding,
            jsonl,
            encoded_bytes,
            sha256,
        }
    }

    pub(crate) fn binding(&self) -> &ReceiptTerminalBinding {
        &self.binding
    }

    pub(crate) fn jsonl(&self) -> &[u8] {
        &self.jsonl
    }

    pub(crate) const fn encoded_bytes(&self) -> u64 {
        self.encoded_bytes
    }

    pub(crate) fn sha256(&self) -> &ArtifactSha256 {
        &self.sha256
    }

    pub(crate) fn into_jsonl(self) -> Box<[u8]> {
        self.jsonl
    }
}

pub(crate) struct PreparedReceiptTerminalPublication {
    record: PreparedReceiptRecord,
    wire_frame: PreparedWireFrame,
}

impl fmt::Debug for PreparedReceiptTerminalPublication {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PreparedReceiptTerminalPublication")
            .field("binding", self.record.binding())
            .field("record_encoded_bytes", &self.record.encoded_bytes())
            .field("record_sha256", self.record.sha256())
            .field("wire_encoded_bytes", &self.wire_frame.encoded_bytes())
            .field("wire_sha256", self.wire_frame.sha256())
            .finish()
    }
}

impl PreparedReceiptTerminalPublication {
    pub(crate) fn new(record: PreparedReceiptRecord, wire_frame: PreparedWireFrame) -> Self {
        Self { record, wire_frame }
    }

    pub(crate) fn record(&self) -> &PreparedReceiptRecord {
        &self.record
    }

    pub(crate) fn wire_frame(&self) -> &PreparedWireFrame {
        &self.wire_frame
    }

    pub(crate) fn into_parts(self) -> (PreparedReceiptRecord, PreparedWireFrame) {
        (self.record, self.wire_frame)
    }
}

#[derive(Debug)]
pub(crate) struct CommittedDirectPublication {
    receipt: DirectTerminalUnackedReceipt,
    wire_frame: PreparedWireFrame,
}

impl CommittedDirectPublication {
    pub(crate) fn new(
        receipt: DirectTerminalUnackedReceipt,
        wire_frame: PreparedWireFrame,
    ) -> Self {
        Self {
            receipt,
            wire_frame,
        }
    }

    pub(crate) fn receipt(&self) -> &DirectTerminalUnackedReceipt {
        &self.receipt
    }

    pub(crate) fn wire_frame(&self) -> &PreparedWireFrame {
        &self.wire_frame
    }

    pub(crate) fn into_parts(self) -> (DirectTerminalUnackedReceipt, PreparedWireFrame) {
        (self.receipt, self.wire_frame)
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
    canonical_v5_terminal_from_shared(Arc::new(outcome.clone()))
}

pub(crate) fn canonical_v5_terminal_from_shared(
    outcome: Arc<ReceiptTerminalOutcome>,
) -> Result<V5CanonicalTerminal, CanonicalTerminalError> {
    let payload = match outcome.as_ref() {
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
        outcome,
        payload: payload.into(),
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
        assert_checked_digest!(ArtifactSha256);
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
    fn receipt_version_is_nonzero_and_advances_independently() {
        let initial = ReceiptVersion::initial();

        assert_eq!(initial.get(), 1);
        assert_eq!(
            initial.checked_next().expect("second record version").get(),
            2
        );
        assert_eq!(
            serde_json::to_value(initial).expect("serialize record version"),
            json!(1)
        );
        assert_eq!(
            serde_json::from_value::<ReceiptVersion>(json!(1))
                .expect("deserialize nonzero record version"),
            initial
        );
        assert!(serde_json::from_value::<ReceiptVersion>(json!(0)).is_err());
        assert!(ReceiptVersion::new(u64::MAX)
            .expect("maximum nonzero record version")
            .checked_next()
            .is_none());
    }

    #[test]
    fn cancel_reserved_receipt_derives_the_fixed_7125ms_expiry_and_exact_header() {
        let key = frozen_receipt_key();
        let version = ReceiptVersion::new(7).expect("nonzero version");
        let receipt = CancelReservedReceipt::new(key.clone(), version, 41, 777, 1_000)
            .expect("fixed cancellation reservation expiry fits");

        assert_eq!(CANCEL_RESERVATION_TTL_MS, 7_125);
        assert_eq!(receipt.key(), &key);
        assert_eq!(receipt.key_digest(), &receipt_key_digest(&key));
        assert_eq!(receipt.record_version(), version);
        assert_eq!(receipt.mutation_sequence(), 41);
        assert_eq!(receipt.encoded_bytes(), 777);
        assert_eq!(receipt.cancel_reserved_at_epoch_ms(), 1_000);
        assert_eq!(receipt.expires_at_epoch_ms(), 8_125);
        assert!(receipt.cancel_requested());
    }

    #[test]
    fn cancel_reserved_receipt_rejects_fixed_expiry_overflow() {
        assert_eq!(
            CancelReservedReceipt::new(
                frozen_receipt_key(),
                ReceiptVersion::initial(),
                1,
                512,
                u64::MAX - CANCEL_RESERVATION_TTL_MS + 1,
            ),
            Err(ReceiptLedgerError::TimestampOverflow)
        );
    }

    #[test]
    fn cancel_expiry_outcome_preserves_not_due_and_existing_terminal_winners() {
        let key = frozen_receipt_key();
        let not_due =
            CancelReservedReceipt::new(key.clone(), ReceiptVersion::initial(), 1, 512, 1_000)
                .expect("fixed cancellation reservation expiry fits");
        assert_eq!(
            CancelExpiryOutcome::NotDue(not_due.clone()),
            CancelExpiryOutcome::NotDue(not_due)
        );
        assert_ne!(CancelExpiryOutcome::Expired, CancelExpiryOutcome::Missing);

        let terminal = canonical_v5_terminal(&ReceiptTerminalOutcome::Cancelled)
            .expect("canonical cancelled terminal");
        let winner = ReceiptState::DirectTerminalUnacked(DirectTerminalUnackedReceipt {
            record: ReceiptRecordHeader::new(
                key.clone(),
                receipt_key_digest(&key),
                ReceiptVersion::new(2).expect("nonzero terminal version"),
                2,
                700,
            ),
            original_cutoff: OriginalCutoffDescriptor::new(1_000, 7_000)
                .expect("valid original cutoff"),
            terminal_epoch_ms: 2_000,
            terminal,
            reserved_result_bytes: MAX_RECEIPT_ENTITLEMENT_BYTES - 700,
        });
        match CancelExpiryOutcome::ExistingWinner(Box::new(winner.clone())) {
            CancelExpiryOutcome::ExistingWinner(observed) => assert_eq!(*observed, winner),
            other => panic!("terminal winner must remain intact, got {other:?}"),
        }
    }

    #[test]
    fn receipt_state_outer_algebra_is_closed_over_sixteen_observable_kinds() {
        assert_eq!(
            ReceiptStateKind::ALL.map(ReceiptStateKind::diagnostic_name),
            [
                "cancel_reserved",
                "reserved_unbound",
                "reserved_actor_bound",
                "reserved_begun",
                "direct_terminal_unacked",
                "acknowledged_tombstone",
                "task_promised_unbound",
                "task_promised_actor_bound",
                "task_handoff_actor_bound_not_begun",
                "task_handoff_actor_bound_begun",
                "task_receipt_owned_actor_bound",
                "task_terminal_receipt_backed",
                "task_bound_not_begun",
                "task_bound_begun",
                "task_terminal_bound",
                "task_retirement_pending",
            ]
        );

        let key = frozen_receipt_key();
        let state = ReceiptState::Reserved(ReservedReceipt::new(
            ReceiptRecordHeader::new(
                key.clone(),
                receipt_key_digest(&key),
                ReceiptVersion::initial(),
                1,
                512,
            ),
            1_000,
            OriginalCutoffDescriptor::new(1_000, 7_000).expect("valid cutoff"),
            ReservedPhase::Unbound,
            false,
            1_024,
        ));
        assert_eq!(state.kind(), ReceiptStateKind::ReservedUnbound);

        fn exhaust_outer_bodies(state: &ReceiptState) -> ReceiptStateKind {
            match state {
                ReceiptState::CancelReserved(_)
                | ReceiptState::Reserved(_)
                | ReceiptState::DirectTerminalUnacked(_)
                | ReceiptState::AcknowledgedTombstone(_)
                | ReceiptState::TaskPromisedUnbound(_)
                | ReceiptState::TaskPromisedActorBound(_)
                | ReceiptState::TaskHandoffActorBound(_)
                | ReceiptState::TaskReceiptOwnedActorBound(_)
                | ReceiptState::TaskTerminalReceiptBacked(_)
                | ReceiptState::TaskBound(_)
                | ReceiptState::TaskTerminalBound(_)
                | ReceiptState::TaskRetirementPending(_) => state.kind(),
            }
        }

        assert_eq!(
            exhaust_outer_bodies(&state),
            ReceiptStateKind::ReservedUnbound
        );
    }

    #[test]
    fn every_observable_kind_is_derived_from_a_real_durable_state_body() {
        let key = frozen_receipt_key();
        let key_digest = receipt_key_digest(&key);
        let workspace_identity = SafeIdentityHash::from_sha256([0x77; 32]);
        let link_identity = TaskLinkIdentity::new(
            key_digest.clone(),
            key.reserved_task_id(),
            key.invocation_id(),
            workspace_identity.clone(),
        );
        let link = TaskLinkReference {
            digest: task_link_digest(&link_identity),
            identity: link_identity,
        };
        let task = ReceiptTaskProjection {
            task_id: key.reserved_task_id(),
            invocation_id: key.invocation_id(),
            created_at_epoch_ms: 1_000,
            updated_at_epoch_ms: 1_000,
            ttl_ms: 3_600_000,
            poll_interval_ms: 100,
            version: 1,
        };
        let terminal = canonical_v5_terminal(&ReceiptTerminalOutcome::Cancelled)
            .expect("canonical cancelled terminal");
        let terminal_digest = terminal.digest().clone();
        let record = || ReceiptRecordHeader {
            key: key.clone(),
            key_digest: key_digest.clone(),
            record_version: ReceiptVersion::initial(),
            mutation_sequence: 1,
            encoded_bytes: 512,
        };
        let link_record = || LifecycleLinkRecordHeader {
            key: key.clone(),
            key_digest: key_digest.clone(),
            link: link.clone(),
            lifecycle_link_version: 1,
            mutation_sequence: 1,
            encoded_bytes: 512,
        };
        let reserved = |phase| {
            ReceiptState::Reserved(ReservedReceipt::new(
                record(),
                1_000,
                OriginalCutoffDescriptor::new(1_000, 7_000).expect("valid cutoff"),
                phase,
                false,
                1_024,
            ))
        };
        let handoff = |phase| {
            ReceiptState::TaskHandoffActorBound(TaskHandoffActorBoundReceipt {
                record: record(),
                task: task.clone(),
                link: link.clone(),
                phase,
                cancel_requested: false,
                reserved_result_bytes: 1_024,
                terminal_stage: HandoffTerminalStage::NoTerminal,
            })
        };
        let bound = |phase| {
            ReceiptState::TaskBound(TaskBoundReceipt {
                record: link_record(),
                task: task.clone(),
                task_record_version: 1,
                bind_epoch_ms: 1_000,
                phase,
            })
        };

        let states = [
            ReceiptState::CancelReserved(CancelReservedReceipt {
                record: record(),
                cancel_reserved_at_epoch_ms: 1_000,
                expires_at_epoch_ms: 8_125,
            }),
            reserved(ReservedPhase::Unbound),
            reserved(ReservedPhase::ActorBound {
                bound_workspace_identity: workspace_identity.clone(),
            }),
            reserved(ReservedPhase::Begun {
                bound_workspace_identity: workspace_identity,
            }),
            ReceiptState::DirectTerminalUnacked(DirectTerminalUnackedReceipt {
                record: record(),
                original_cutoff: OriginalCutoffDescriptor::new(1_000, 7_000).expect("valid cutoff"),
                terminal_epoch_ms: 2_000,
                terminal: terminal.clone(),
                reserved_result_bytes: 1_024,
            }),
            ReceiptState::AcknowledgedTombstone(AcknowledgedTombstoneReceipt {
                key: key.clone(),
                key_digest: key_digest.clone(),
                terminal_digest: terminal_digest.clone(),
                acknowledged_at_epoch_ms: 2_100,
                encoded_bytes: 512,
            }),
            ReceiptState::TaskPromisedUnbound(TaskPromisedUnboundReceipt {
                record: record(),
                task: task.clone(),
                cancel_requested: false,
                reserved_result_bytes: 1_024,
            }),
            ReceiptState::TaskPromisedActorBound(TaskPromisedActorBoundReceipt {
                record: record(),
                task: task.clone(),
                link: link.clone(),
                cancel_requested: false,
                reserved_result_bytes: 1_024,
            }),
            handoff(AttemptPhase::NotBegun),
            handoff(AttemptPhase::Begun),
            ReceiptState::TaskReceiptOwnedActorBound(TaskReceiptOwnedActorBoundReceipt {
                record: record(),
                task: task.clone(),
                link: link.clone(),
                cancel_requested: false,
                reserved_result_bytes: 1_024,
                proven_link_capacity: ProvenTaskLinkCapacity::Count {
                    observed_live_links: 4_096,
                    maximum_live_links: 4_096,
                },
            }),
            ReceiptState::TaskTerminalReceiptBacked(TaskTerminalReceiptBackedReceipt {
                record: record(),
                task: task.clone(),
                terminal_epoch_ms: 2_000,
                terminal,
                cancel_requested: true,
                reserved_result_bytes: 1_024,
            }),
            bound(AttemptPhase::NotBegun),
            bound(AttemptPhase::Begun),
            ReceiptState::TaskTerminalBound(TaskTerminalBoundReceipt {
                record: link_record(),
                task: task.clone(),
                task_record_version: 2,
                terminal_status: ClosedTerminalStatus::Cancelled,
                terminal_digest: terminal_digest.clone(),
                terminal_epoch_ms: 2_000,
                expires_at_epoch_ms: 3_602_000,
            }),
            ReceiptState::TaskRetirementPending(TaskRetirementPendingReceipt {
                record: link_record(),
                task,
                expected_terminal_task_version: 2,
                terminal_status: ClosedTerminalStatus::Cancelled,
                terminal_digest,
                terminal_epoch_ms: 2_000,
                expires_at_epoch_ms: 3_602_000,
                retained_link_bytes: 512,
                retained_dual_id_accounting: RetainedDualIdAccounting {
                    invocation_index_bytes: 64,
                    reserved_task_index_bytes: 64,
                },
            }),
        ];

        assert_eq!(states.map(|state| state.kind()), ReceiptStateKind::ALL);
    }

    #[test]
    fn staged_terminal_certificate_carries_the_complete_closed_transfer_bound() {
        let key = frozen_receipt_key();
        let key_digest = receipt_key_digest(&key);
        let workspace_identity = SafeIdentityHash::from_sha256([0x77; 32]);
        let link_identity = TaskLinkIdentity::new(
            key_digest.clone(),
            key.reserved_task_id(),
            key.invocation_id(),
            workspace_identity,
        );
        let link_digest = task_link_digest(&link_identity);
        let terminal = canonical_v5_terminal(&ReceiptTerminalOutcome::Cancelled)
            .expect("canonical cancelled terminal");
        let terminal_digest = terminal.digest().clone();
        let task_publication_cases = [
            StagedTaskPublicationCase::Absent {
                final_task_record_max_bytes: 1_000,
                task_response_frame_max_bytes: 2_000,
            },
            StagedTaskPublicationCase::ExactProvisional {
                status: ProvisionalTaskStatus::Queued,
                version: u64::MAX,
                cancel_requested: false,
                final_task_record_max_bytes: 1_001,
                task_response_frame_max_bytes: 2_001,
            },
            StagedTaskPublicationCase::ExactProvisional {
                status: ProvisionalTaskStatus::Queued,
                version: u64::MAX,
                cancel_requested: true,
                final_task_record_max_bytes: 1_002,
                task_response_frame_max_bytes: 2_002,
            },
            StagedTaskPublicationCase::ExactProvisional {
                status: ProvisionalTaskStatus::Working,
                version: u64::MAX,
                cancel_requested: false,
                final_task_record_max_bytes: 1_003,
                task_response_frame_max_bytes: 2_003,
            },
            StagedTaskPublicationCase::ExactProvisional {
                status: ProvisionalTaskStatus::Working,
                version: u64::MAX,
                cancel_requested: true,
                final_task_record_max_bytes: 1_004,
                task_response_frame_max_bytes: 2_004,
            },
        ];
        let capacity_fallback_cases = [StagedCapacityFallbackCase::LinkCapacity {
            receipt_backed_record_max_bytes: 8_400_000,
            task_response_frame_max_bytes: 8_454_144,
        }];
        let certificate = StagedTerminalTransferCertificate::new(
            key.core_identity_digest().clone(),
            key_digest,
            key.reserved_task_id(),
            key.invocation_id(),
            link_digest,
            terminal_digest,
            2_000,
            8_400_000,
            1_024,
            task_publication_cases.clone(),
            capacity_fallback_cases.clone(),
        )
        .expect("exact closed transfer certificate");

        assert_eq!(certificate.task_publication_cases.as_slice().len(), 5);
        assert!(matches!(
            certificate.task_publication_cases.as_slice()[0],
            StagedTaskPublicationCase::Absent { .. }
        ));
        assert!(matches!(
            certificate.capacity_fallback_cases,
            [StagedCapacityFallbackCase::LinkCapacity { .. }]
        ));
        let mut wrong_order = task_publication_cases.clone();
        wrong_order.swap(1, 2);
        assert_eq!(
            StagedTaskPublicationCases::new(wrong_order),
            Err(StagedTerminalTransferCertificateError::InvalidTaskPublicationCases)
        );
        let mut wrong_version = task_publication_cases;
        wrong_version[1] = StagedTaskPublicationCase::ExactProvisional {
            status: ProvisionalTaskStatus::Queued,
            version: 1,
            cancel_requested: false,
            final_task_record_max_bytes: 1_001,
            task_response_frame_max_bytes: 2_001,
        };
        assert_eq!(
            StagedTaskPublicationCases::new(wrong_version),
            Err(StagedTerminalTransferCertificateError::InvalidTaskPublicationCases)
        );
        assert_eq!(
            StagedTerminalTransferCertificate::new(
                key.core_identity_digest().clone(),
                receipt_key_digest(&key),
                key.reserved_task_id(),
                key.invocation_id(),
                task_link_digest(&link_identity),
                terminal.digest().clone(),
                2_000,
                8_400_000,
                1_025,
                [
                    StagedTaskPublicationCase::Absent {
                        final_task_record_max_bytes: 1_000,
                        task_response_frame_max_bytes: 2_000,
                    },
                    StagedTaskPublicationCase::ExactProvisional {
                        status: ProvisionalTaskStatus::Queued,
                        version: u64::MAX,
                        cancel_requested: false,
                        final_task_record_max_bytes: 1_001,
                        task_response_frame_max_bytes: 2_001,
                    },
                    StagedTaskPublicationCase::ExactProvisional {
                        status: ProvisionalTaskStatus::Queued,
                        version: u64::MAX,
                        cancel_requested: true,
                        final_task_record_max_bytes: 1_002,
                        task_response_frame_max_bytes: 2_002,
                    },
                    StagedTaskPublicationCase::ExactProvisional {
                        status: ProvisionalTaskStatus::Working,
                        version: u64::MAX,
                        cancel_requested: false,
                        final_task_record_max_bytes: 1_003,
                        task_response_frame_max_bytes: 2_003,
                    },
                    StagedTaskPublicationCase::ExactProvisional {
                        status: ProvisionalTaskStatus::Working,
                        version: u64::MAX,
                        cancel_requested: true,
                        final_task_record_max_bytes: 1_004,
                        task_response_frame_max_bytes: 2_004,
                    },
                ],
                [StagedCapacityFallbackCase::LinkCapacity {
                    receipt_backed_record_max_bytes: 8_400_000,
                    task_response_frame_max_bytes: 8_454_144,
                }],
            ),
            Err(StagedTerminalTransferCertificateError::TaskTerminalBoundLinkRecordTooLarge)
        );
    }

    #[test]
    fn receipt_backed_completed_terminal_preserves_independent_cancel_request() {
        let key = frozen_receipt_key();
        let receipt = TaskTerminalReceiptBackedReceipt {
            record: ReceiptRecordHeader::new(
                key.clone(),
                receipt_key_digest(&key),
                ReceiptVersion::initial(),
                1,
                512,
            ),
            task: ReceiptTaskProjection {
                task_id: key.reserved_task_id(),
                invocation_id: key.invocation_id(),
                created_at_epoch_ms: 1_000,
                updated_at_epoch_ms: 2_000,
                ttl_ms: 3_600_000,
                poll_interval_ms: 100,
                version: 2,
            },
            terminal_epoch_ms: 2_000,
            terminal: canonical_v5_terminal(&ReceiptTerminalOutcome::Completed {
                result: Box::new(DomainResult::success(
                    "completed despite cancellation request",
                )),
            })
            .expect("canonical completed terminal"),
            cancel_requested: true,
            reserved_result_bytes: 1_024,
        };

        assert!(matches!(
            receipt.terminal.outcome(),
            ReceiptTerminalOutcome::Completed { .. }
        ));
        assert!(receipt.cancel_requested);
    }

    #[test]
    fn receipt_digest_collision_requires_process_owned_reopen() {
        assert!(ReceiptLedgerError::ReceiptDigestCollision.requires_reopen());
        assert!(!ReceiptLedgerError::ReceiptNotFound.requires_reopen());
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
