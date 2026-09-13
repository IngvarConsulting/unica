//! Кодирование и разбор имён и байт хранилища. Здесь живёт единственный
//! сериализатор активных нетерминальных записей: байты Direct-терминала
//! принадлежат `terminal_codec_v5`, и второго кодека для них нет.

use super::*;

/// Sole serializer for active non-terminal receipt records.
///
/// Direct terminal bytes are owned by `terminal_codec_v5`; accepting one here
/// would create a second, potentially divergent codec for the same lifecycle.
pub(super) fn serialize_reserved_record(
    record: StoredActiveReceiptV1,
    maximum_encoded_bytes: u64,
) -> Result<(StoredActiveReceiptV1, Vec<u8>), ReceiptLedgerError> {
    if matches!(
        &record.lifecycle,
        StoredActiveLifecycleV1::DirectTerminalUnacked { .. }
    ) {
        return Err(ReceiptLedgerError::Corrupt(
            "Direct terminal rows must use the sole v5 terminal codec",
        ));
    }
    let encoded = match &record.lifecycle {
        StoredActiveLifecycleV1::AcknowledgedTombstone {
            terminal_digest,
            acknowledged_at_epoch_ms,
        } => serde_json::to_vec(&StoredAcknowledgedTombstoneV1 {
            key: record.key.clone(),
            terminal_digest: terminal_digest.clone(),
            ack_epoch_ms: *acknowledged_at_epoch_ms,
            record_version: (record.record_version.get() != 3).then_some(record.record_version),
        }),
        _ => serde_json::to_vec(&record),
    }
    .map_err(|_| ReceiptLedgerError::Corrupt("receipt row serialization failed"))?;
    let encoded_bytes =
        u64::try_from(encoded.len()).map_err(|_| ReceiptLedgerError::RecordTooLarge)?;
    if encoded_bytes > maximum_encoded_bytes || encoded_bytes > MAX_RECEIPT_ENTITLEMENT_BYTES {
        return Err(ReceiptLedgerError::RecordTooLarge);
    }
    Ok((record, encoded))
}

pub(super) fn validate_persisted_reserved_record_bytes(
    record: &StoredActiveReceiptV1,
    persisted_bytes: &[u8],
) -> Result<(), ReceiptLedgerError> {
    let maximum_encoded_bytes = match &record.lifecycle {
        StoredActiveLifecycleV1::CancelReserved { .. }
        | StoredActiveLifecycleV1::ExpiredDeletion { .. }
        | StoredActiveLifecycleV1::ExpiredTombstoneDeletion { .. }
        | StoredActiveLifecycleV1::ExpiredDirectDeletion { .. }
        | StoredActiveLifecycleV1::ExpiredTaskReceiptDeletion { .. }
        | StoredActiveLifecycleV1::AcknowledgementCommit { .. } => MAX_CANCEL_RESERVED_RECORD_BYTES,
        StoredActiveLifecycleV1::CompletedTaskHandoffDeletion { .. } => {
            MAX_COMPLETED_TASK_HANDOFF_WITNESS_BYTES
        }
        StoredActiveLifecycleV1::TaskHandoffActorBound {
            terminal_stage: StoredHandoffTerminalStageV1::Staged { .. },
            ..
        } => MAX_RECEIPT_ENTITLEMENT_BYTES,
        StoredActiveLifecycleV1::ReservedUnbound { .. }
        | StoredActiveLifecycleV1::ReservedActorBound { .. }
        | StoredActiveLifecycleV1::ReservedBegun { .. }
        | StoredActiveLifecycleV1::TaskPromisedUnbound { .. }
        | StoredActiveLifecycleV1::TaskPromisedActorBound { .. }
        | StoredActiveLifecycleV1::TaskHandoffActorBound {
            terminal_stage: StoredHandoffTerminalStageV1::NoTerminal,
            ..
        }
        | StoredActiveLifecycleV1::TaskReceiptOwnedActorBound { .. } => {
            MAX_TASK_RECORD_ENVELOPE_BYTES as u64
        }
        StoredActiveLifecycleV1::TaskTerminalReceiptBacked { .. } => MAX_RECEIPT_ENTITLEMENT_BYTES,
        StoredActiveLifecycleV1::AcknowledgedTombstone { .. } => MAX_ACKNOWLEDGED_TOMBSTONE_BYTES,
        StoredActiveLifecycleV1::DirectTerminalUnacked { .. } => {
            return Err(ReceiptLedgerError::Corrupt(
                "non-terminal receipt validator received a Direct terminal lifecycle",
            ))
        }
    };
    let (canonical_record, canonical_bytes) =
        serialize_reserved_record(record.clone(), maximum_encoded_bytes)?;
    if &canonical_record != record || canonical_bytes != persisted_bytes {
        return Err(ReceiptLedgerError::Corrupt(
            "receipt row is not canonical schema-v1 JSON",
        ));
    }
    Ok(())
}

pub(super) fn parse_receipt_record_name(
    name: &str,
) -> Result<ReceiptKeyDigest, ReceiptLedgerError> {
    let digest = name
        .strip_suffix(".json")
        .ok_or(ReceiptLedgerError::Corrupt(
            "receipt active entry has an unsupported name",
        ))?;
    digest
        .parse()
        .map_err(|_| ReceiptLedgerError::Corrupt("receipt row name is not a canonical digest"))
}

pub(super) fn parse_receipt_batch_name(name: &str) -> Result<Option<Uuid>, ReceiptLedgerError> {
    let Some(uuid_text) = name
        .strip_prefix("receipt-batch.")
        .and_then(|name| name.strip_suffix(".json"))
    else {
        return Ok(None);
    };
    let uuid = Uuid::parse_str(uuid_text).map_err(|_| {
        ReceiptLedgerError::Corrupt("receipt batch name does not contain a canonical UUIDv4")
    })?;
    if uuid.hyphenated().to_string() != uuid_text
        || uuid.get_version() != Some(uuid::Version::Random)
        || uuid.get_variant() != uuid::Variant::RFC4122
    {
        return Err(ReceiptLedgerError::Corrupt(
            "receipt batch name does not contain a canonical UUIDv4",
        ));
    }
    Ok(Some(uuid))
}

pub(super) fn decode_receipt_batch(
    bytes: &[u8],
) -> Result<DecodedReceiptBatchV1, ReceiptLedgerError> {
    if bytes.len() as u64 > MAX_RECEIPT_BATCH_BYTES {
        return Err(ReceiptLedgerError::RecordTooLarge);
    }
    let batch: StoredReceiptBatchV1 = serde_json::from_slice(bytes).map_err(|_| {
        ReceiptLedgerError::Corrupt("receipt batch is not canonical schema-v1 JSON")
    })?;
    if batch.schema_version != RECEIPT_BATCH_SCHEMA_VERSION
        || batch.rows.is_empty()
        || batch.rows.len() > MAX_RECEIPT_BATCH_ENVELOPE_ROWS
    {
        return Err(ReceiptLedgerError::Corrupt(
            "receipt batch has an unsupported schema or row count",
        ));
    }
    let rows = batch
        .rows
        .into_iter()
        .map(|row| {
            let encoded = decode_lower_hex(&row.persisted_hex)?;
            let record: StoredActiveReceiptV1 = match serde_json::from_slice(&encoded) {
                Ok(record) => record,
                Err(_) => {
                    let tombstone: StoredAcknowledgedTombstoneV1 = serde_json::from_slice(&encoded)
                        .map_err(|_| {
                            ReceiptLedgerError::Corrupt(
                                "receipt batch row is not a strict supported JSON record",
                            )
                        })?;
                    let marker_version = tombstone.record_version.unwrap_or(
                        ReceiptVersion::new(3).expect("tombstone marker version is nonzero"),
                    );
                    if marker_version != row.record_version {
                        return Err(ReceiptLedgerError::Corrupt(
                            "receipt batch metadata contradicts its tombstone version",
                        ));
                    }
                    StoredActiveReceiptV1 {
                        schema_version: RECEIPT_RECORD_SCHEMA_VERSION,
                        mutation_sequence: row.mutation_sequence,
                        record_version: marker_version,
                        key_digest: receipt_key_digest(&tombstone.key),
                        key: tombstone.key,
                        lifecycle: StoredActiveLifecycleV1::AcknowledgedTombstone {
                            terminal_digest: tombstone.terminal_digest,
                            acknowledged_at_epoch_ms: tombstone.ack_epoch_ms,
                        },
                    }
                }
            };
            if record.mutation_sequence != row.mutation_sequence
                || record.record_version != row.record_version
            {
                return Err(ReceiptLedgerError::Corrupt(
                    "receipt batch metadata contradicts its persisted row",
                ));
            }
            catalog_entry_from_batch_record(record.clone(), &encoded)?;
            Ok(ReceiptBatchRow { record, encoded })
        })
        .collect::<Result<Vec<_>, ReceiptLedgerError>>()?;
    let canonical = encode_receipt_batch_envelope(&rows)?;
    if canonical != bytes {
        return Err(ReceiptLedgerError::Corrupt(
            "receipt batch is not canonical schema-v1 JSON",
        ));
    }
    Ok(DecodedReceiptBatchV1 { rows })
}

pub(super) fn parse_receipt_temporary_name(name: &str) -> Result<bool, ReceiptLedgerError> {
    let Some(uuid_text) = name
        .strip_prefix(".receipt.")
        .and_then(|name| name.strip_suffix(".tmp"))
    else {
        return Ok(false);
    };
    let uuid = Uuid::parse_str(uuid_text).map_err(|_| {
        ReceiptLedgerError::Corrupt("receipt staging name does not contain a canonical UUIDv4")
    })?;
    if uuid.hyphenated().to_string() != uuid_text
        || uuid.get_version() != Some(uuid::Version::Random)
        || uuid.get_variant() != uuid::Variant::RFC4122
    {
        return Err(ReceiptLedgerError::Corrupt(
            "receipt staging name does not contain a canonical UUIDv4",
        ));
    }
    Ok(true)
}

pub(super) fn parse_generation_temporary_name(name: &str) -> Result<bool, ReceiptLedgerError> {
    let Some(uuid_text) = name
        .strip_prefix(".generation.")
        .and_then(|name| name.strip_suffix(".tmp"))
    else {
        return Ok(false);
    };
    let uuid = Uuid::parse_str(uuid_text).map_err(|_| {
        ReceiptLedgerError::Corrupt("generation staging name does not contain a canonical UUIDv4")
    })?;
    if uuid.hyphenated().to_string() != uuid_text
        || uuid.get_version() != Some(uuid::Version::Random)
        || uuid.get_variant() != uuid::Variant::RFC4122
    {
        return Err(ReceiptLedgerError::Corrupt(
            "generation staging name does not contain a canonical UUIDv4",
        ));
    }
    Ok(true)
}

pub(super) fn parse_cleanup_quarantine_name(name: &str) -> Result<bool, ReceiptLedgerError> {
    let Some(uuid_text) = name.strip_prefix(".unica-cleanup-") else {
        return Ok(false);
    };
    let uuid = Uuid::parse_str(uuid_text).map_err(|_| {
        ReceiptLedgerError::Corrupt("cleanup quarantine name is not a canonical UUIDv4")
    })?;
    if uuid.hyphenated().to_string() != uuid_text
        || uuid.get_version() != Some(uuid::Version::Random)
        || uuid.get_variant() != uuid::Variant::RFC4122
    {
        return Err(ReceiptLedgerError::Corrupt(
            "cleanup quarantine name is not a canonical UUIDv4",
        ));
    }
    Ok(true)
}
