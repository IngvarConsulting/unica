use crate::application::receipt_ledger::{
    canonical_v5_terminal_from_shared, receipt_key_digest, ArtifactSha256,
    DirectTerminalUnackedReceipt, OriginalCutoffDescriptor, PreparedReceiptRecord,
    PreparedReceiptTerminalPublication, PreparedWireFrame, ReceiptKey, ReceiptKeyDigest,
    ReceiptLedgerError, ReceiptTerminalBinding, ReceiptTerminalOutcome, ReceiptVersion,
    TerminalDigest, V5CanonicalTerminal, DIRECT_TERMINAL_RETENTION_MS,
    MAX_RECEIPT_ENTITLEMENT_BYTES,
};
use sha2::{Digest, Sha256};
use std::sync::Arc;

/// A ledger-owned description of the exact durable row that the terminal
/// codec is allowed to prepare. Callers outside the receipt infrastructure
/// cannot mint one, so neither the daemon transport nor a tool handler can
/// choose a mutation sequence for durable receipt state.
pub(in crate::infrastructure) struct DirectReceiptWriteSlot {
    key: ReceiptKey,
    key_digest: crate::application::receipt_ledger::ReceiptKeyDigest,
    expected_version: ReceiptVersion,
    committed_version: ReceiptVersion,
    generation_before: u64,
    mutation_sequence: u64,
    original_cutoff: OriginalCutoffDescriptor,
}

impl DirectReceiptWriteSlot {
    #[allow(clippy::too_many_arguments)]
    pub(in crate::infrastructure) fn new(
        key: &ReceiptKey,
        expected_version: ReceiptVersion,
        committed_version: ReceiptVersion,
        generation_before: u64,
        mutation_sequence: u64,
        original_cutoff: OriginalCutoffDescriptor,
    ) -> Result<Self, ReceiptLedgerError> {
        if expected_version.checked_next() != Some(committed_version) {
            return Err(ReceiptLedgerError::Corrupt(
                "direct write slot does not advance the exact receipt version",
            ));
        }
        if generation_before.checked_add(1) != Some(mutation_sequence) {
            return Err(ReceiptLedgerError::Corrupt(
                "direct write slot does not advance the ledger generation",
            ));
        }
        Ok(Self {
            key: key.clone(),
            key_digest: receipt_key_digest(key),
            expected_version,
            committed_version,
            generation_before,
            mutation_sequence,
            original_cutoff,
        })
    }

    pub(in crate::infrastructure) const fn generation_before(&self) -> u64 {
        self.generation_before
    }
}

struct EncodedDirectRecord {
    key_json: Vec<u8>,
    bytes: Box<[u8]>,
    encoded_bytes: u64,
    reserved_result_bytes: u64,
}

pub(in crate::infrastructure) fn prepare_direct_terminal(
    slot: DirectReceiptWriteSlot,
    terminal: V5CanonicalTerminal,
    terminal_epoch_ms: u64,
) -> Result<PreparedReceiptTerminalPublication, ReceiptLedgerError> {
    let DirectReceiptWriteSlot {
        key,
        key_digest,
        expected_version,
        committed_version,
        generation_before: _,
        mutation_sequence,
        original_cutoff,
    } = slot;
    let terminal_digest = terminal.digest().clone();
    let encoded = encode_direct_record(
        mutation_sequence,
        committed_version,
        &key,
        &key_digest,
        original_cutoff,
        terminal_epoch_ms,
        &terminal,
    )?;

    let binding = ReceiptTerminalBinding::new(
        key,
        key_digest,
        expected_version,
        committed_version,
        mutation_sequence,
        original_cutoff,
        terminal_epoch_ms,
        terminal_digest,
    );
    let wire_frame = prepare_direct_wire_frame(binding.clone(), &encoded.key_json, &terminal)?;
    let record_sha256 = artifact_sha256(&encoded.bytes);
    let record = PreparedReceiptRecord::new(
        binding.clone(),
        encoded.bytes,
        encoded.encoded_bytes,
        encoded.reserved_result_bytes,
        record_sha256,
        terminal,
    );
    Ok(PreparedReceiptTerminalPublication::new(record, wire_frame))
}

#[allow(clippy::too_many_arguments)]
pub(in crate::infrastructure) fn validate_persisted_direct_record_bytes(
    mutation_sequence: u64,
    record_version: ReceiptVersion,
    key: &ReceiptKey,
    key_digest: &ReceiptKeyDigest,
    original_cutoff: OriginalCutoffDescriptor,
    terminal_epoch_ms: u64,
    expected_terminal_digest: &TerminalDigest,
    outcome: Arc<ReceiptTerminalOutcome>,
    persisted_bytes: &[u8],
) -> Result<(), ReceiptLedgerError> {
    let terminal = restore_canonical_terminal(outcome, expected_terminal_digest)?;
    let encoded = encode_direct_record(
        mutation_sequence,
        record_version,
        key,
        key_digest,
        original_cutoff,
        terminal_epoch_ms,
        &terminal,
    )?;
    if encoded.bytes.as_ref() != persisted_bytes {
        return Err(ReceiptLedgerError::Corrupt(
            "receipt row is not canonical schema-v1 JSON",
        ));
    }
    Ok(())
}

pub(in crate::infrastructure) fn prepare_committed_direct_wire(
    receipt: &DirectTerminalUnackedReceipt,
) -> Result<PreparedWireFrame, ReceiptLedgerError> {
    receipt
        .terminal_epoch_ms()
        .checked_add(DIRECT_TERMINAL_RETENTION_MS)
        .ok_or(ReceiptLedgerError::TimestampOverflow)?;
    if receipt.key_digest() != &receipt_key_digest(receipt.key()) {
        return Err(ReceiptLedgerError::Corrupt(
            "committed Direct receipt key digest is inconsistent",
        ));
    }
    let expected_version =
        receipt
            .record_version()
            .checked_previous()
            .ok_or(ReceiptLedgerError::Corrupt(
                "committed Direct receipt has no predecessor version",
            ))?;
    let binding = ReceiptTerminalBinding::new(
        receipt.key().clone(),
        receipt.key_digest().clone(),
        expected_version,
        receipt.record_version(),
        receipt.mutation_sequence(),
        *receipt.original_cutoff(),
        receipt.terminal_epoch_ms(),
        receipt.terminal().digest().clone(),
    );
    let key_json = serde_json::to_vec(receipt.key())
        .map_err(|_| ReceiptLedgerError::Corrupt("receipt key serialization failed"))?;
    prepare_direct_wire_frame(binding, &key_json, receipt.terminal())
}

pub(in crate::infrastructure) fn restore_canonical_terminal(
    outcome: Arc<ReceiptTerminalOutcome>,
    expected_digest: &TerminalDigest,
) -> Result<V5CanonicalTerminal, ReceiptLedgerError> {
    let terminal = canonical_v5_terminal_from_shared(outcome)
        .map_err(|_| ReceiptLedgerError::Corrupt("receipt terminal cannot be canonicalized"))?;
    if terminal.digest() != expected_digest {
        return Err(ReceiptLedgerError::Corrupt(
            "receipt terminal digest does not match its canonical outcome",
        ));
    }
    Ok(terminal)
}

fn prepare_direct_wire_frame(
    binding: ReceiptTerminalBinding,
    key_json: &[u8],
    terminal: &V5CanonicalTerminal,
) -> Result<PreparedWireFrame, ReceiptLedgerError> {
    let mut wire_bytes = Vec::with_capacity(
        key_json
            .len()
            .saturating_add(terminal.payload().len())
            .saturating_add(320),
    );
    wire_bytes.extend_from_slice(
        b"{\"kind\":\"invocation\",\"outcome\":{\"resultType\":\"direct\",\"receipt\":{\"receiptKey\":",
    );
    wire_bytes.extend_from_slice(key_json);
    wire_bytes.extend_from_slice(b",\"terminal\":");
    wire_bytes.extend_from_slice(terminal.payload());
    wire_bytes.extend_from_slice(b",\"terminalDigest\":\"");
    wire_bytes.extend_from_slice(terminal.digest().as_str().as_bytes());
    wire_bytes.extend_from_slice(b"\",\"terminalEpochMs\":");
    append_u64(&mut wire_bytes, binding.terminal_epoch_ms());
    wire_bytes.extend_from_slice(b"}}}\n");
    let wire_encoded_bytes =
        u64::try_from(wire_bytes.len()).map_err(|_| ReceiptLedgerError::RecordTooLarge)?;
    if wire_encoded_bytes > MAX_RECEIPT_ENTITLEMENT_BYTES {
        return Err(ReceiptLedgerError::RecordTooLarge);
    }
    let wire_sha256 = artifact_sha256(&wire_bytes);
    Ok(PreparedWireFrame::new(
        binding,
        wire_bytes.into_boxed_slice(),
        wire_encoded_bytes,
        wire_sha256,
    ))
}

#[allow(clippy::too_many_arguments)]
fn encode_direct_record(
    mutation_sequence: u64,
    record_version: ReceiptVersion,
    key: &ReceiptKey,
    key_digest: &ReceiptKeyDigest,
    original_cutoff: OriginalCutoffDescriptor,
    terminal_epoch_ms: u64,
    terminal: &V5CanonicalTerminal,
) -> Result<EncodedDirectRecord, ReceiptLedgerError> {
    terminal_epoch_ms
        .checked_add(DIRECT_TERMINAL_RETENTION_MS)
        .ok_or(ReceiptLedgerError::TimestampOverflow)?;
    let key_json = serde_json::to_vec(key)
        .map_err(|_| ReceiptLedgerError::Corrupt("receipt key serialization failed"))?;
    let cutoff_json = serde_json::to_vec(&original_cutoff)
        .map_err(|_| ReceiptLedgerError::Corrupt("receipt cutoff serialization failed"))?;

    let mut prefix = Vec::with_capacity(768);
    prefix.extend_from_slice(b"{\"schemaVersion\":1,\"mutationSequence\":");
    append_u64(&mut prefix, mutation_sequence);
    prefix.extend_from_slice(b",\"recordVersion\":");
    append_u64(&mut prefix, record_version.get());
    prefix.extend_from_slice(b",\"key\":");
    prefix.extend_from_slice(&key_json);
    prefix.extend_from_slice(b",\"keyDigest\":\"");
    prefix.extend_from_slice(key_digest.as_str().as_bytes());
    prefix.extend_from_slice(
        b"\",\"lifecycle\":{\"state\":\"direct_terminal_unacked\",\"originalCutoff\":",
    );
    prefix.extend_from_slice(&cutoff_json);
    prefix.extend_from_slice(b",\"terminalEpochMs\":");
    append_u64(&mut prefix, terminal_epoch_ms);
    prefix.extend_from_slice(b",\"terminalDigest\":\"");
    prefix.extend_from_slice(terminal.digest().as_str().as_bytes());
    prefix.extend_from_slice(b"\",\"terminal\":");

    const SUFFIX: &[u8] = b"}}";
    let record_length = prefix
        .len()
        .checked_add(terminal.payload().len())
        .and_then(|length| length.checked_add(SUFFIX.len()))
        .ok_or(ReceiptLedgerError::RecordTooLarge)?;
    let encoded_bytes =
        u64::try_from(record_length).map_err(|_| ReceiptLedgerError::RecordTooLarge)?;
    let reserved_result_bytes = MAX_RECEIPT_ENTITLEMENT_BYTES
        .checked_sub(encoded_bytes)
        .ok_or(ReceiptLedgerError::RecordTooLarge)?;
    let capacity =
        usize::try_from(encoded_bytes).map_err(|_| ReceiptLedgerError::RecordTooLarge)?;
    let mut bytes = Vec::with_capacity(capacity);
    bytes.extend_from_slice(&prefix);
    bytes.extend_from_slice(terminal.payload());
    bytes.extend_from_slice(SUFFIX);
    if bytes.len() != capacity {
        return Err(ReceiptLedgerError::Corrupt(
            "direct receipt entitlement length calculation diverged",
        ));
    }
    Ok(EncodedDirectRecord {
        key_json,
        bytes: bytes.into_boxed_slice(),
        encoded_bytes,
        reserved_result_bytes,
    })
}

fn append_u64(target: &mut Vec<u8>, value: u64) {
    target.extend_from_slice(value.to_string().as_bytes());
}

fn artifact_sha256(bytes: &[u8]) -> ArtifactSha256 {
    ArtifactSha256::from_sha256(Sha256::digest(bytes).into())
}

#[cfg(test)]
mod tests {
    use super::{prepare_direct_terminal, DirectReceiptWriteSlot};
    use crate::application::receipt_ledger::{
        canonical_v5_terminal, request_scope_hash, ArtifactSha256, CoreIdentityDigest,
        OriginalCutoffDescriptor, ReceiptKey, ReceiptTerminalOutcome, ReceiptVersion,
        RequestIdentity, V5ToolIdentity,
    };
    use crate::domain::invocation::{InvocationId, NormalizedArgumentsHash, TaskId};
    use std::str::FromStr;

    fn exact_key() -> ReceiptKey {
        ReceiptKey::new(
            InvocationId::from_str("123e4567-e89b-42d3-a456-426614174000")
                .expect("canonical invocation id"),
            TaskId::from_str("123e4567-e89b-42d3-b456-426614174001")
                .expect("canonical reserved task id"),
            RequestIdentity::new(
                CoreIdentityDigest::from_str(&"00".repeat(32)).expect("frozen core digest"),
                V5ToolIdentity::View,
                NormalizedArgumentsHash::from_sha256([0x11; 32]),
                request_scope_hash("workspace-a").expect("bounded request scope"),
            ),
        )
    }

    fn exact_slot(key: &ReceiptKey) -> DirectReceiptWriteSlot {
        DirectReceiptWriteSlot::new(
            key,
            ReceiptVersion::initial(),
            ReceiptVersion::new(2).expect("second receipt version"),
            1,
            2,
            OriginalCutoffDescriptor::new(1_000, 7_000).expect("valid original cutoff"),
        )
        .expect("ledger-minted direct write slot")
    }

    #[test]
    fn cancelled_direct_prepares_exact_record_and_jsonl() {
        let key = exact_key();
        let slot = exact_slot(&key);
        let terminal = canonical_v5_terminal(&ReceiptTerminalOutcome::Cancelled)
            .expect("canonical cancelled terminal");

        let prepared = prepare_direct_terminal(slot, terminal, 2_000)
            .expect("prepare exact Direct terminal publication");

        assert_eq!(prepared.record().binding(), prepared.wire_frame().binding());
        assert_eq!(prepared.record().bytes().len(), 797);
        assert_eq!(
            prepared.record().sha256(),
            &ArtifactSha256::from_str(
                "debfeb17d064b74593bd162f9e05f4d3a8d409b28f1063d89d1c29909c2ef034"
            )
            .expect("frozen record artifact digest")
        );
        assert_ne!(prepared.record().bytes().last(), Some(&b'\n'));
        assert_eq!(
            prepared.record().encoded_bytes() + prepared.record().reserved_result_bytes(),
            crate::application::receipt_ledger::MAX_RECEIPT_ENTITLEMENT_BYTES
        );
        assert!(
            prepared.wire_frame().encoded_bytes()
                <= crate::application::receipt_ledger::MAX_RECEIPT_ENTITLEMENT_BYTES
        );
        let record: serde_json::Value =
            serde_json::from_slice(prepared.record().bytes()).expect("exact record JSON");
        assert_eq!(record["schemaVersion"], 1);
        assert_eq!(record["mutationSequence"], 2);
        assert_eq!(record["recordVersion"], 2);
        assert_eq!(record["lifecycle"]["state"], "direct_terminal_unacked");
        assert_eq!(
            record["lifecycle"]["originalCutoff"]["acceptedEpochMs"],
            1_000
        );
        assert_eq!(
            record["lifecycle"]["originalCutoff"]["responseBudgetMs"],
            7_000
        );
        assert_eq!(record["lifecycle"]["terminal"]["status"], "cancelled");

        assert_eq!(prepared.wire_frame().jsonl().len(), 621);
        assert_eq!(
            prepared.wire_frame().sha256(),
            &ArtifactSha256::from_str(
                "587cfbc539488a11da2387f3ff29d4a72e5c6167776fe3c691172d5ec5b2d4ed"
            )
            .expect("frozen wire artifact digest")
        );
        assert_eq!(prepared.wire_frame().jsonl().last(), Some(&b'\n'));
        let wire: serde_json::Value = serde_json::from_slice(
            &prepared.wire_frame().jsonl()[..prepared.wire_frame().jsonl().len() - 1],
        )
        .expect("exact Direct JSONL frame");
        assert_eq!(wire["kind"], "invocation");
        assert_eq!(wire["outcome"]["resultType"], "direct");
        assert_eq!(wire["outcome"]["receipt"]["receiptKey"], record["key"]);
        assert_eq!(
            wire["outcome"]["receipt"]["terminal"],
            record["lifecycle"]["terminal"]
        );
        assert_eq!(
            wire["outcome"]["receipt"]["terminalDigest"],
            record["lifecycle"]["terminalDigest"]
        );
        assert_eq!(wire["outcome"]["receipt"]["terminalEpochMs"], 2_000);
    }

    #[test]
    fn legal_direct_payload_cannot_fall_into_a_decimal_entitlement_gap() {
        let key = exact_key();
        let terminal = canonical_v5_terminal(&ReceiptTerminalOutcome::Completed {
            result: Box::new(crate::domain::invocation::DomainResult::success(
                "x".repeat(8_353_285),
            )),
        })
        .expect("result remains below the canonical 8 MiB limit");

        let prepared = prepare_direct_terminal(exact_slot(&key), terminal, 2_000)
            .expect("every legal canonical terminal has exact receipt accounting");

        assert_eq!(
            prepared.record().encoded_bytes() + prepared.record().reserved_result_bytes(),
            crate::application::receipt_ledger::MAX_RECEIPT_ENTITLEMENT_BYTES
        );
    }

    #[test]
    fn durable_direct_record_does_not_persist_its_derived_residual_quota() {
        let key = exact_key();
        let terminal = canonical_v5_terminal(&ReceiptTerminalOutcome::Cancelled)
            .expect("canonical cancelled terminal");

        let prepared = prepare_direct_terminal(exact_slot(&key), terminal, 2_000)
            .expect("prepare exact Direct terminal publication");
        let record: serde_json::Value =
            serde_json::from_slice(prepared.record().bytes()).expect("exact record JSON");

        assert!(record["lifecycle"].get("reservedResultBytes").is_none());
    }

    #[test]
    fn direct_write_slot_rejects_non_adjacent_version_and_generation() {
        let key = exact_key();
        let cutoff = OriginalCutoffDescriptor::new(1_000, 7_000).expect("valid original cutoff");

        assert!(matches!(
            DirectReceiptWriteSlot::new(
                &key,
                ReceiptVersion::initial(),
                ReceiptVersion::new(3).expect("third version"),
                1,
                2,
                cutoff,
            ),
            Err(
                crate::application::receipt_ledger::ReceiptLedgerError::Corrupt(
                    "direct write slot does not advance the exact receipt version"
                )
            )
        ));
        assert!(matches!(
            DirectReceiptWriteSlot::new(
                &key,
                ReceiptVersion::initial(),
                ReceiptVersion::new(2).expect("second version"),
                1,
                3,
                cutoff,
            ),
            Err(
                crate::application::receipt_ledger::ReceiptLedgerError::Corrupt(
                    "direct write slot does not advance the ledger generation"
                )
            )
        ));
    }

    #[test]
    fn direct_preflight_rejects_terminal_retention_overflow() {
        let key = exact_key();
        let terminal = canonical_v5_terminal(&ReceiptTerminalOutcome::Cancelled)
            .expect("canonical cancelled terminal");

        assert_eq!(
            prepare_direct_terminal(exact_slot(&key), terminal, u64::MAX)
                .expect_err("terminal expiry cannot exceed u64"),
            crate::application::receipt_ledger::ReceiptLedgerError::TimestampOverflow
        );
    }

    #[test]
    fn canonical_terminal_payload_is_spliced_once_into_each_artifact() {
        let key = exact_key();
        let terminal = canonical_v5_terminal(&ReceiptTerminalOutcome::Completed {
            result: Box::new(crate::domain::invocation::DomainResult::success(
                "quote=\" slash=\\ newline=\n snowman=☃",
            )),
        })
        .expect("canonical escaped terminal");
        let payload = terminal.payload().to_vec();

        let prepared = prepare_direct_terminal(exact_slot(&key), terminal, 2_000)
            .expect("prepare escaped Direct publication");

        assert_eq!(count_subslice(prepared.record().bytes(), &payload), 1);
        assert_eq!(count_subslice(prepared.wire_frame().jsonl(), &payload), 1);
    }

    #[test]
    fn prepared_debug_is_bounded_and_redacts_terminal_and_artifact_bytes() {
        let key = exact_key();
        let terminal = canonical_v5_terminal(&ReceiptTerminalOutcome::Completed {
            result: Box::new(crate::domain::invocation::DomainResult::success(
                "SECRET_SENTINEL".repeat(1_024),
            )),
        })
        .expect("canonical bounded terminal");
        let prepared = prepare_direct_terminal(exact_slot(&key), terminal, 2_000)
            .expect("prepare Direct publication");

        let debug = format!("{prepared:?}");

        assert!(!debug.contains("SECRET_SENTINEL"));
        assert!(
            debug.len() < 2_048,
            "debug output must remain metadata-sized"
        );
    }

    fn count_subslice(haystack: &[u8], needle: &[u8]) -> usize {
        haystack
            .windows(needle.len())
            .filter(|window| *window == needle)
            .count()
    }
}
