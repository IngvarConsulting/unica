//! Проверка активной записи квитанции перед фиксацией: единственное
//! место, где решается, законна ли durable-запись.

use super::*;

pub(super) fn validate_active_record(
    record: &StoredActiveReceiptV1,
    encoded: &[u8],
    expected_digest: &ReceiptKeyDigest,
) -> Result<(), ReceiptLedgerError> {
    if record.schema_version != RECEIPT_RECORD_SCHEMA_VERSION {
        return Err(ReceiptLedgerError::Corrupt(
            "receipt row schema version is unsupported",
        ));
    }
    if record.mutation_sequence == 0
        && !matches!(
            &record.lifecycle,
            StoredActiveLifecycleV1::AcknowledgedTombstone { .. }
        )
    {
        return Err(ReceiptLedgerError::Corrupt(
            "receipt mutation sequence must be positive",
        ));
    }
    if &record.key_digest != expected_digest || receipt_key_digest(&record.key) != record.key_digest
    {
        return Err(ReceiptLedgerError::Corrupt(
            "receipt row digest does not match its name and exact key",
        ));
    }
    let per_record_limit = match &record.lifecycle {
        StoredActiveLifecycleV1::CancelReserved {
            cancel_reserved_at_epoch_ms,
            expires_at_epoch_ms,
            cancel_requested,
        } => {
            if record.record_version != ReceiptVersion::initial() {
                return Err(ReceiptLedgerError::Corrupt(
                    "CancelReserved receipt must retain its initial record version",
                ));
            }
            if !cancel_requested {
                return Err(ReceiptLedgerError::Corrupt(
                    "CancelReserved receipt must persist cancelRequested=true",
                ));
            }
            let expected_expiry = cancel_reserved_at_epoch_ms
                .checked_add(CANCEL_RESERVATION_TTL_MS)
                .ok_or(ReceiptLedgerError::Corrupt(
                    "CancelReserved expiry exceeds u64",
                ))?;
            if *expires_at_epoch_ms != expected_expiry {
                return Err(ReceiptLedgerError::Corrupt(
                    "CancelReserved expiry is not the fixed absolute TTL",
                ));
            }
            MAX_CANCEL_RESERVED_RECORD_BYTES
        }
        StoredActiveLifecycleV1::ExpiredDeletion {
            observed_at_epoch_ms,
            prior_record_version,
            prior_mutation_sequence,
            prior_cancel_reserved_at_epoch_ms,
            prior_expires_at_epoch_ms,
        } => {
            if *prior_record_version != ReceiptVersion::initial() {
                return Err(ReceiptLedgerError::Corrupt(
                    "expired deletion witness predecessor is not an initial CancelReserved",
                ));
            }
            if prior_record_version.checked_next() != Some(record.record_version) {
                return Err(ReceiptLedgerError::Corrupt(
                    "expired deletion witness does not advance its predecessor version",
                ));
            }
            if *prior_mutation_sequence == 0 || *prior_mutation_sequence >= record.mutation_sequence
            {
                return Err(ReceiptLedgerError::Corrupt(
                    "expired deletion witness does not follow its predecessor mutation",
                ));
            }
            if prior_cancel_reserved_at_epoch_ms.checked_add(CANCEL_RESERVATION_TTL_MS)
                != Some(*prior_expires_at_epoch_ms)
            {
                return Err(ReceiptLedgerError::Corrupt(
                    "expired deletion witness predecessor expiry is not its fixed absolute TTL",
                ));
            }
            if observed_at_epoch_ms < prior_expires_at_epoch_ms {
                return Err(ReceiptLedgerError::Corrupt(
                    "expired deletion witness predates the absolute expiry boundary",
                ));
            }
            MAX_CANCEL_RESERVED_RECORD_BYTES
        }
        StoredActiveLifecycleV1::ExpiredTombstoneDeletion {
            observed_at_epoch_ms,
            prior_acknowledged_at_epoch_ms,
            ..
        } => {
            if record.record_version.get() != 4 {
                return Err(ReceiptLedgerError::Corrupt(
                    "expired tombstone deletion witness has an invalid record version",
                ));
            }
            let expires_at_epoch_ms = prior_acknowledged_at_epoch_ms
                .checked_add(ACKNOWLEDGED_TOMBSTONE_TTL_MS)
                .ok_or(ReceiptLedgerError::Corrupt(
                    "expired tombstone deletion witness expiry exceeds u64",
                ))?;
            if *observed_at_epoch_ms < expires_at_epoch_ms {
                return Err(ReceiptLedgerError::Corrupt(
                    "expired tombstone deletion witness predates the absolute expiry boundary",
                ));
            }
            MAX_CANCEL_RESERVED_RECORD_BYTES
        }
        StoredActiveLifecycleV1::ExpiredDirectDeletion {
            observed_at_epoch_ms,
            prior_record_version,
            prior_mutation_sequence,
            prior_terminal_epoch_ms,
            ..
        } => {
            if *prior_record_version == ReceiptVersion::initial()
                || prior_record_version.checked_next() != Some(record.record_version)
            {
                return Err(ReceiptLedgerError::Corrupt(
                    "expired Direct deletion witness does not advance a terminal predecessor",
                ));
            }
            if *prior_mutation_sequence == 0 || *prior_mutation_sequence >= record.mutation_sequence
            {
                return Err(ReceiptLedgerError::Corrupt(
                    "expired Direct deletion witness does not follow its predecessor mutation",
                ));
            }
            let expires_at_epoch_ms = prior_terminal_epoch_ms
                .checked_add(DIRECT_TERMINAL_RETENTION_MS)
                .ok_or(ReceiptLedgerError::Corrupt(
                    "expired Direct deletion witness expiry exceeds u64",
                ))?;
            if *observed_at_epoch_ms < expires_at_epoch_ms {
                return Err(ReceiptLedgerError::Corrupt(
                    "expired Direct deletion witness predates the absolute expiry boundary",
                ));
            }
            MAX_CANCEL_RESERVED_RECORD_BYTES
        }
        StoredActiveLifecycleV1::ExpiredTaskReceiptDeletion {
            observed_at_epoch_ms,
            prior_record_version,
            prior_mutation_sequence,
            prior_terminal_epoch_ms,
            prior_ttl_ms,
            ..
        } => {
            if *prior_record_version == ReceiptVersion::initial()
                || prior_record_version.checked_next() != Some(record.record_version)
            {
                return Err(ReceiptLedgerError::Corrupt(
                    "expired receipt-backed Task deletion witness does not advance a terminal predecessor",
                ));
            }
            if *prior_mutation_sequence == 0 || *prior_mutation_sequence >= record.mutation_sequence
            {
                return Err(ReceiptLedgerError::Corrupt(
                    "expired receipt-backed Task deletion witness does not follow its predecessor mutation",
                ));
            }
            let expires_at_epoch_ms = prior_terminal_epoch_ms.checked_add(*prior_ttl_ms).ok_or(
                ReceiptLedgerError::Corrupt(
                    "expired receipt-backed Task deletion witness expiry exceeds u64",
                ),
            )?;
            if *prior_ttl_ms == 0 || *observed_at_epoch_ms < expires_at_epoch_ms {
                return Err(ReceiptLedgerError::Corrupt(
                    "expired receipt-backed Task deletion witness predates the absolute expiry boundary",
                ));
            }
            MAX_CANCEL_RESERVED_RECORD_BYTES
        }
        StoredActiveLifecycleV1::CompletedTaskHandoffDeletion {
            prior_record_version,
            prior_mutation_sequence,
            prior_created_at_epoch_ms,
            prior_task_version,
            workspace_identity_hash,
            task_link_digest,
            task_bound_lifecycle_link_version,
            task_bound_mutation_sequence,
            task_record_version,
            bind_epoch_ms,
            phase,
            terminal_staged,
        } => {
            let minimum_prior_version = match phase {
                AttemptPhase::NotBegun => 3,
                AttemptPhase::Begun => 4,
            };
            let expected_link = TaskLinkReference::new(
                record.key_digest.clone(),
                record.key.reserved_task_id(),
                record.key.invocation_id(),
                workspace_identity_hash.clone(),
            );
            if prior_record_version.get() < minimum_prior_version
                || prior_record_version.checked_next() != Some(record.record_version)
                || *prior_mutation_sequence == 0
                || *prior_mutation_sequence >= record.mutation_sequence
                || *prior_task_version == 0
                || if *terminal_staged {
                    *task_record_version <= *prior_task_version
                } else {
                    *task_record_version != *prior_task_version
                }
                || *task_bound_lifecycle_link_version == 0
                || *task_bound_mutation_sequence == 0
                || *bind_epoch_ms < *prior_created_at_epoch_ms
                || expected_link.digest() != task_link_digest
            {
                return Err(ReceiptLedgerError::Corrupt(
                    "completed Task handoff witness contradicts its receipt or confirmed TaskBound",
                ));
            }
            MAX_COMPLETED_TASK_HANDOFF_WITNESS_BYTES
        }
        StoredActiveLifecycleV1::ReservedUnbound {
            reserved_at_epoch_ms,
            original_cutoff,
            ..
        }
        | StoredActiveLifecycleV1::ReservedActorBound {
            reserved_at_epoch_ms,
            original_cutoff,
            ..
        }
        | StoredActiveLifecycleV1::ReservedBegun {
            reserved_at_epoch_ms,
            original_cutoff,
            ..
        } => {
            if reserved_at_epoch_ms != &original_cutoff.accepted_epoch_ms() {
                return Err(ReceiptLedgerError::Corrupt(
                    "receipt reserve epoch does not match its accepted request epoch",
                ));
            }
            MAX_TASK_RECORD_ENVELOPE_BYTES as u64
        }
        StoredActiveLifecycleV1::TaskPromisedUnbound {
            original_cutoff,
            task_id,
            invocation_id,
            created_at_epoch_ms,
            updated_at_epoch_ms,
            ttl_ms,
            poll_interval_ms,
            task_version,
            ..
        } => {
            if record.record_version == ReceiptVersion::initial()
                || *task_id != record.key.reserved_task_id()
                || *invocation_id != record.key.invocation_id()
                || updated_at_epoch_ms < created_at_epoch_ms
                || created_at_epoch_ms < &original_cutoff.accepted_epoch_ms()
                || *ttl_ms == 0
                || *poll_interval_ms == 0
                || *task_version == 0
            {
                return Err(ReceiptLedgerError::Corrupt(
                    "promised Task row contradicts its receipt identity or lifecycle",
                ));
            }
            MAX_TASK_RECORD_ENVELOPE_BYTES as u64
        }
        StoredActiveLifecycleV1::TaskPromisedActorBound {
            original_cutoff,
            task_id,
            invocation_id,
            created_at_epoch_ms,
            updated_at_epoch_ms,
            ttl_ms,
            poll_interval_ms,
            task_version,
            workspace_identity_hash,
            task_link_digest,
            ..
        } => {
            let expected_link = TaskLinkReference::new(
                record.key_digest.clone(),
                *task_id,
                *invocation_id,
                workspace_identity_hash.clone(),
            );
            if record.record_version.get() < 3
                || *task_id != record.key.reserved_task_id()
                || *invocation_id != record.key.invocation_id()
                || updated_at_epoch_ms < created_at_epoch_ms
                || created_at_epoch_ms < &original_cutoff.accepted_epoch_ms()
                || *ttl_ms == 0
                || *poll_interval_ms == 0
                || *task_version == 0
                || expected_link.digest() != task_link_digest
            {
                return Err(ReceiptLedgerError::Corrupt(
                    "actor-bound promised Task row contradicts its receipt or link identity",
                ));
            }
            MAX_TASK_RECORD_ENVELOPE_BYTES as u64
        }
        StoredActiveLifecycleV1::TaskHandoffActorBound {
            original_cutoff,
            task_id,
            invocation_id,
            created_at_epoch_ms,
            updated_at_epoch_ms,
            ttl_ms,
            poll_interval_ms,
            task_version,
            workspace_identity_hash,
            task_link_digest,
            phase,
            terminal_stage,
            ..
        } => {
            let expected_link = TaskLinkReference::new(
                record.key_digest.clone(),
                *task_id,
                *invocation_id,
                workspace_identity_hash.clone(),
            );
            let minimum_version = match phase {
                AttemptPhase::NotBegun => 3,
                AttemptPhase::Begun => 4,
            };
            if record.record_version.get() < minimum_version
                || *task_id != record.key.reserved_task_id()
                || *invocation_id != record.key.invocation_id()
                || updated_at_epoch_ms < created_at_epoch_ms
                || created_at_epoch_ms < &original_cutoff.accepted_epoch_ms()
                || *ttl_ms == 0
                || *poll_interval_ms == 0
                || *task_version == 0
                || expected_link.digest() != task_link_digest
            {
                return Err(ReceiptLedgerError::Corrupt(
                    "Task handoff row contradicts its receipt, attempt, or link identity",
                ));
            }
            match terminal_stage {
                StoredHandoffTerminalStageV1::NoTerminal => MAX_TASK_RECORD_ENVELOPE_BYTES as u64,
                StoredHandoffTerminalStageV1::Staged {
                    terminal_epoch_ms,
                    terminal_digest,
                    terminal,
                } => {
                    if terminal_epoch_ms < updated_at_epoch_ms {
                        return Err(ReceiptLedgerError::Corrupt(
                            "staged Task handoff terminal predates its Task projection",
                        ));
                    }
                    restore_canonical_terminal(Arc::clone(terminal), terminal_digest)?;
                    MAX_RECEIPT_ENTITLEMENT_BYTES
                }
            }
        }
        StoredActiveLifecycleV1::TaskReceiptOwnedActorBound {
            original_cutoff,
            task_id,
            invocation_id,
            created_at_epoch_ms,
            updated_at_epoch_ms,
            ttl_ms,
            poll_interval_ms,
            task_version,
            workspace_identity_hash,
            task_link_digest,
            proven_link_capacity,
            ..
        } => {
            let expected_link = TaskLinkReference::new(
                record.key_digest.clone(),
                *task_id,
                *invocation_id,
                workspace_identity_hash.clone(),
            );
            let capacity_is_exhausted = match proven_link_capacity {
                StoredProvenTaskLinkCapacityV1::Count {
                    observed_live_links,
                    maximum_live_links,
                } => *maximum_live_links > 0 && observed_live_links >= maximum_live_links,
                StoredProvenTaskLinkCapacityV1::Bytes {
                    required_link_bytes,
                    available_link_bytes,
                } => *required_link_bytes > 0 && required_link_bytes > available_link_bytes,
            };
            if record.record_version.get() < 5
                || *task_id != record.key.reserved_task_id()
                || *invocation_id != record.key.invocation_id()
                || updated_at_epoch_ms < created_at_epoch_ms
                || created_at_epoch_ms < &original_cutoff.accepted_epoch_ms()
                || *ttl_ms == 0
                || *poll_interval_ms == 0
                || *task_version == 0
                || expected_link.digest() != task_link_digest
                || !capacity_is_exhausted
            {
                return Err(ReceiptLedgerError::Corrupt(
                    "receipt-owned Task row contradicts its receipt, begun attempt, link, or capacity evidence",
                ));
            }
            MAX_TASK_RECORD_ENVELOPE_BYTES as u64
        }
        StoredActiveLifecycleV1::DirectTerminalUnacked {
            original_cutoff,
            terminal_epoch_ms,
            terminal_digest,
            terminal,
            ..
        } => {
            if record.record_version == ReceiptVersion::initial() {
                return Err(ReceiptLedgerError::Corrupt(
                    "direct terminal receipt must advance its record version",
                ));
            }
            terminal_epoch_ms
                .checked_add(DIRECT_TERMINAL_RETENTION_MS)
                .ok_or(ReceiptLedgerError::Corrupt(
                    "receipt terminal expiry exceeds u64",
                ))?;
            validate_persisted_direct_record_bytes(
                record.mutation_sequence,
                record.record_version,
                &record.key,
                &record.key_digest,
                *original_cutoff,
                *terminal_epoch_ms,
                terminal_digest,
                Arc::clone(terminal),
                encoded,
            )?;
            MAX_RECEIPT_ENTITLEMENT_BYTES
        }
        StoredActiveLifecycleV1::TaskTerminalReceiptBacked {
            task_id,
            invocation_id,
            created_at_epoch_ms,
            updated_at_epoch_ms,
            ttl_ms,
            poll_interval_ms,
            task_version,
            terminal_epoch_ms,
            terminal_digest,
            terminal,
            ..
        } => {
            if record.record_version == ReceiptVersion::initial()
                || task_id != &record.key.reserved_task_id()
                || invocation_id != &record.key.invocation_id()
                || updated_at_epoch_ms < created_at_epoch_ms
                || updated_at_epoch_ms != terminal_epoch_ms
                || *ttl_ms == 0
                || *poll_interval_ms == 0
                || *task_version == 0
            {
                return Err(ReceiptLedgerError::Corrupt(
                    "receipt-backed Task terminal has inconsistent identity or lifecycle metadata",
                ));
            }
            terminal_epoch_ms
                .checked_add(*ttl_ms)
                .ok_or(ReceiptLedgerError::Corrupt(
                    "receipt-backed Task expiry exceeds u64",
                ))?;
            restore_canonical_terminal(Arc::clone(terminal), terminal_digest)?;
            MAX_RECEIPT_ENTITLEMENT_BYTES
        }
        StoredActiveLifecycleV1::AcknowledgementCommit {
            acknowledged_at_epoch_ms,
            prior_record_version,
            prior_mutation_sequence,
            ..
        } => {
            if prior_record_version.checked_next() != Some(record.record_version) {
                return Err(ReceiptLedgerError::Corrupt(
                    "acknowledgement witness does not advance its predecessor version",
                ));
            }
            if *prior_mutation_sequence == 0 || *prior_mutation_sequence >= record.mutation_sequence
            {
                return Err(ReceiptLedgerError::Corrupt(
                    "acknowledgement witness does not follow its predecessor mutation",
                ));
            }
            acknowledged_at_epoch_ms
                .checked_add(ACKNOWLEDGED_TOMBSTONE_TTL_MS)
                .ok_or(ReceiptLedgerError::Corrupt(
                    "acknowledgement witness expiry exceeds u64",
                ))?;
            MAX_CANCEL_RESERVED_RECORD_BYTES
        }
        StoredActiveLifecycleV1::AcknowledgedTombstone {
            acknowledged_at_epoch_ms,
            ..
        } => {
            if record.record_version.get() < 3 {
                return Err(ReceiptLedgerError::Corrupt(
                    "acknowledged tombstone must follow a direct terminal record",
                ));
            }
            acknowledged_at_epoch_ms
                .checked_add(ACKNOWLEDGED_TOMBSTONE_TTL_MS)
                .ok_or(ReceiptLedgerError::Corrupt(
                    "acknowledged tombstone expiry exceeds u64",
                ))?;
            MAX_ACKNOWLEDGED_TOMBSTONE_BYTES
        }
    };
    if u64::try_from(encoded.len()).map_or(true, |length| length > per_record_limit) {
        return Err(ReceiptLedgerError::Corrupt(
            "persisted receipt row exceeds its byte limit",
        ));
    }
    Ok(())
}
