//! Сборка durable-записей хранилища: резервирование, смена фазы, отмена,
//! удаление истёкшего и подтверждение. Только форма записи — решение о
//! её законности принимает `validation`.

use super::*;

pub(super) fn build_reserved_record(
    key: ReceiptKey,
    key_digest: ReceiptKeyDigest,
    original_cutoff: OriginalCutoffDescriptor,
    mutation_sequence: u64,
    record_version: ReceiptVersion,
    cancel_requested: bool,
) -> StoredActiveReceiptV1 {
    StoredActiveReceiptV1 {
        schema_version: RECEIPT_RECORD_SCHEMA_VERSION,
        mutation_sequence,
        record_version,
        key,
        key_digest,
        lifecycle: StoredActiveLifecycleV1::ReservedUnbound {
            reserved_at_epoch_ms: original_cutoff.accepted_epoch_ms(),
            original_cutoff,
            cancel_requested,
        },
    }
}

pub(super) enum ReservedPhaseTransition {
    BindActor(SafeIdentityHash),
    MarkBegun,
}

pub(super) fn build_reserved_phase_record(
    expected: &CatalogEntry,
    phase: ReservedPhase,
    mutation_sequence: u64,
    record_version: ReceiptVersion,
) -> Result<StoredActiveReceiptV1, ReceiptLedgerError> {
    let reserved = expected.reservation()?;
    let lifecycle = match phase {
        ReservedPhase::Unbound => {
            return Err(ReceiptLedgerError::Corrupt(
                "reserved phase transition cannot return to unbound",
            ))
        }
        ReservedPhase::ActorBound {
            bound_workspace_identity,
        } => StoredActiveLifecycleV1::ReservedActorBound {
            reserved_at_epoch_ms: reserved.reserved_at_epoch_ms(),
            original_cutoff: *reserved.original_cutoff(),
            bound_workspace_identity,
            cancel_requested: reserved.cancel_requested(),
        },
        ReservedPhase::Begun {
            bound_workspace_identity,
        } => StoredActiveLifecycleV1::ReservedBegun {
            reserved_at_epoch_ms: reserved.reserved_at_epoch_ms(),
            original_cutoff: *reserved.original_cutoff(),
            bound_workspace_identity,
            cancel_requested: reserved.cancel_requested(),
        },
    };
    Ok(StoredActiveReceiptV1 {
        schema_version: RECEIPT_RECORD_SCHEMA_VERSION,
        mutation_sequence,
        record_version,
        key: expected.record.key.clone(),
        key_digest: expected.record.key_digest.clone(),
        lifecycle,
    })
}

pub(super) fn build_reserved_cancel_record(
    expected: &CatalogEntry,
    mutation_sequence: u64,
    record_version: ReceiptVersion,
) -> Result<StoredActiveReceiptV1, ReceiptLedgerError> {
    let reserved = expected.reservation()?;
    let lifecycle = match reserved.phase() {
        ReservedPhase::Unbound => StoredActiveLifecycleV1::ReservedUnbound {
            reserved_at_epoch_ms: reserved.reserved_at_epoch_ms(),
            original_cutoff: *reserved.original_cutoff(),
            cancel_requested: true,
        },
        ReservedPhase::ActorBound {
            bound_workspace_identity,
        } => StoredActiveLifecycleV1::ReservedActorBound {
            reserved_at_epoch_ms: reserved.reserved_at_epoch_ms(),
            original_cutoff: *reserved.original_cutoff(),
            bound_workspace_identity: bound_workspace_identity.clone(),
            cancel_requested: true,
        },
        ReservedPhase::Begun {
            bound_workspace_identity,
        } => StoredActiveLifecycleV1::ReservedBegun {
            reserved_at_epoch_ms: reserved.reserved_at_epoch_ms(),
            original_cutoff: *reserved.original_cutoff(),
            bound_workspace_identity: bound_workspace_identity.clone(),
            cancel_requested: true,
        },
    };
    Ok(StoredActiveReceiptV1 {
        schema_version: RECEIPT_RECORD_SCHEMA_VERSION,
        mutation_sequence,
        record_version,
        key: expected.record.key.clone(),
        key_digest: expected.record.key_digest.clone(),
        lifecycle,
    })
}

pub(super) fn build_cancel_reserved_record(
    key: ReceiptKey,
    key_digest: ReceiptKeyDigest,
    cancel_reserved_at_epoch_ms: u64,
    expires_at_epoch_ms: u64,
    mutation_sequence: u64,
) -> StoredActiveReceiptV1 {
    StoredActiveReceiptV1 {
        schema_version: RECEIPT_RECORD_SCHEMA_VERSION,
        mutation_sequence,
        record_version: ReceiptVersion::initial(),
        key,
        key_digest,
        lifecycle: StoredActiveLifecycleV1::CancelReserved {
            cancel_reserved_at_epoch_ms,
            expires_at_epoch_ms,
            cancel_requested: true,
        },
    }
}

pub(super) fn build_expired_deletion_record(
    expected: &CatalogEntry,
    observed_at_epoch_ms: u64,
    mutation_sequence: u64,
    record_version: ReceiptVersion,
) -> Result<StoredActiveReceiptV1, ReceiptLedgerError> {
    let (prior_cancel_reserved_at_epoch_ms, prior_expires_at_epoch_ms) =
        match &expected.record.lifecycle {
            StoredActiveLifecycleV1::CancelReserved {
                cancel_reserved_at_epoch_ms,
                expires_at_epoch_ms,
                ..
            } => (*cancel_reserved_at_epoch_ms, *expires_at_epoch_ms),
            _ => {
                return Err(ReceiptLedgerError::Corrupt(
                    "expired deletion witness requires a CancelReserved predecessor",
                ))
            }
        };
    Ok(StoredActiveReceiptV1 {
        schema_version: RECEIPT_RECORD_SCHEMA_VERSION,
        mutation_sequence,
        record_version,
        key: expected.record.key.clone(),
        key_digest: expected.record.key_digest.clone(),
        lifecycle: StoredActiveLifecycleV1::ExpiredDeletion {
            observed_at_epoch_ms,
            prior_record_version: expected.record.record_version,
            prior_mutation_sequence: expected.record.mutation_sequence,
            prior_cancel_reserved_at_epoch_ms,
            prior_expires_at_epoch_ms,
        },
    })
}

pub(super) fn build_expired_tombstone_deletion_record(
    expected: &CatalogEntry,
    observed_at_epoch_ms: u64,
    mutation_sequence: u64,
) -> Result<StoredActiveReceiptV1, ReceiptLedgerError> {
    let (prior_acknowledged_at_epoch_ms, prior_terminal_digest) = match &expected.record.lifecycle {
        StoredActiveLifecycleV1::AcknowledgedTombstone {
            terminal_digest,
            acknowledged_at_epoch_ms,
        } => (*acknowledged_at_epoch_ms, terminal_digest.clone()),
        _ => {
            return Err(ReceiptLedgerError::Corrupt(
                "expired tombstone deletion witness requires an acknowledged predecessor",
            ))
        }
    };
    Ok(StoredActiveReceiptV1 {
        schema_version: RECEIPT_RECORD_SCHEMA_VERSION,
        mutation_sequence,
        record_version: ReceiptVersion::new(4)
            .expect("expired tombstone witness version is nonzero"),
        key: expected.record.key.clone(),
        key_digest: expected.record.key_digest.clone(),
        lifecycle: StoredActiveLifecycleV1::ExpiredTombstoneDeletion {
            observed_at_epoch_ms,
            prior_acknowledged_at_epoch_ms,
            prior_terminal_digest,
        },
    })
}

pub(super) fn build_expired_direct_deletion_record(
    expected: &CatalogEntry,
    observed_at_epoch_ms: u64,
    mutation_sequence: u64,
) -> Result<StoredActiveReceiptV1, ReceiptLedgerError> {
    let (prior_terminal_epoch_ms, prior_terminal_digest) = match &expected.record.lifecycle {
        StoredActiveLifecycleV1::DirectTerminalUnacked {
            terminal_epoch_ms,
            terminal_digest,
            ..
        } => (*terminal_epoch_ms, terminal_digest.clone()),
        _ => {
            return Err(ReceiptLedgerError::Corrupt(
                "expired Direct deletion witness requires a Direct predecessor",
            ))
        }
    };
    let record_version =
        expected
            .record
            .record_version
            .checked_next()
            .ok_or(ReceiptLedgerError::Corrupt(
                "receipt record version exhausted u64",
            ))?;
    Ok(StoredActiveReceiptV1 {
        schema_version: RECEIPT_RECORD_SCHEMA_VERSION,
        mutation_sequence,
        record_version,
        key: expected.record.key.clone(),
        key_digest: expected.record.key_digest.clone(),
        lifecycle: StoredActiveLifecycleV1::ExpiredDirectDeletion {
            observed_at_epoch_ms,
            prior_record_version: expected.record.record_version,
            prior_mutation_sequence: expected.record.mutation_sequence,
            prior_terminal_epoch_ms,
            prior_terminal_digest,
        },
    })
}

pub(super) fn build_expired_task_receipt_deletion_record(
    expected: &CatalogEntry,
    observed_at_epoch_ms: u64,
    mutation_sequence: u64,
) -> Result<StoredActiveReceiptV1, ReceiptLedgerError> {
    let (prior_terminal_epoch_ms, prior_ttl_ms, prior_terminal_digest) =
        match &expected.record.lifecycle {
            StoredActiveLifecycleV1::TaskTerminalReceiptBacked {
                terminal_epoch_ms,
                ttl_ms,
                terminal_digest,
                ..
            } => (*terminal_epoch_ms, *ttl_ms, terminal_digest.clone()),
            _ => return Err(ReceiptLedgerError::Corrupt(
                "expired receipt-backed Task deletion witness requires a Task terminal predecessor",
            )),
        };
    let record_version =
        expected
            .record
            .record_version
            .checked_next()
            .ok_or(ReceiptLedgerError::Corrupt(
                "receipt record version exhausted u64",
            ))?;
    Ok(StoredActiveReceiptV1 {
        schema_version: RECEIPT_RECORD_SCHEMA_VERSION,
        mutation_sequence,
        record_version,
        key: expected.record.key.clone(),
        key_digest: expected.record.key_digest.clone(),
        lifecycle: StoredActiveLifecycleV1::ExpiredTaskReceiptDeletion {
            observed_at_epoch_ms,
            prior_record_version: expected.record.record_version,
            prior_mutation_sequence: expected.record.mutation_sequence,
            prior_terminal_epoch_ms,
            prior_ttl_ms,
            prior_terminal_digest,
        },
    })
}

pub(super) fn build_completed_task_handoff_deletion_record(
    expected: &CatalogEntry,
    confirmed_task_bound: &TaskBoundReceipt,
    mutation_sequence: u64,
) -> Result<StoredActiveReceiptV1, ReceiptLedgerError> {
    let (
        prior_created_at_epoch_ms,
        prior_task_version,
        workspace_identity_hash,
        task_link_digest,
        phase,
    ) = match &expected.record.lifecycle {
        StoredActiveLifecycleV1::TaskPromisedActorBound {
            created_at_epoch_ms,
            task_version,
            workspace_identity_hash,
            task_link_digest,
            ..
        } => (
            *created_at_epoch_ms,
            *task_version,
            workspace_identity_hash.clone(),
            task_link_digest.clone(),
            AttemptPhase::NotBegun,
        ),
        StoredActiveLifecycleV1::TaskHandoffActorBound {
            created_at_epoch_ms,
            task_version,
            workspace_identity_hash,
            task_link_digest,
            phase,
            ..
        } => (
            *created_at_epoch_ms,
            *task_version,
            workspace_identity_hash.clone(),
            task_link_digest.clone(),
            *phase,
        ),
        _ => {
            return Err(ReceiptLedgerError::Corrupt(
                "completed handoff deletion witness requires an actor-bound Task predecessor",
            ))
        }
    };
    let record_version =
        expected
            .record
            .record_version
            .checked_next()
            .ok_or(ReceiptLedgerError::Corrupt(
                "receipt record version exhausted u64",
            ))?;
    Ok(StoredActiveReceiptV1 {
        schema_version: RECEIPT_RECORD_SCHEMA_VERSION,
        mutation_sequence,
        record_version,
        key: expected.record.key.clone(),
        key_digest: expected.record.key_digest.clone(),
        lifecycle: StoredActiveLifecycleV1::CompletedTaskHandoffDeletion {
            prior_record_version: expected.record.record_version,
            prior_mutation_sequence: expected.record.mutation_sequence,
            prior_created_at_epoch_ms,
            prior_task_version,
            workspace_identity_hash,
            task_link_digest,
            task_bound_lifecycle_link_version: confirmed_task_bound.lifecycle_link_version(),
            task_bound_mutation_sequence: confirmed_task_bound.mutation_sequence(),
            task_record_version: confirmed_task_bound.task_record_version(),
            bind_epoch_ms: confirmed_task_bound.bind_epoch_ms(),
            phase,
            terminal_staged: false,
        },
    })
}

pub(super) fn build_completed_staged_task_handoff_deletion_record(
    expected: &CatalogEntry,
    confirmed_terminal_bound: &TaskTerminalBoundReceipt,
    mutation_sequence: u64,
) -> Result<StoredActiveReceiptV1, ReceiptLedgerError> {
    let (
        prior_created_at_epoch_ms,
        prior_task_version,
        workspace_identity_hash,
        task_link_digest,
        phase,
    ) = match &expected.record.lifecycle {
        StoredActiveLifecycleV1::TaskHandoffActorBound {
            created_at_epoch_ms,
            task_version,
            workspace_identity_hash,
            task_link_digest,
            phase,
            terminal_stage: StoredHandoffTerminalStageV1::Staged { .. },
            ..
        } => (
            *created_at_epoch_ms,
            *task_version,
            workspace_identity_hash.clone(),
            task_link_digest.clone(),
            *phase,
        ),
        _ => {
            return Err(ReceiptLedgerError::Corrupt(
                "completed staged handoff witness requires a staged Task handoff predecessor",
            ))
        }
    };
    let record_version =
        expected
            .record
            .record_version
            .checked_next()
            .ok_or(ReceiptLedgerError::Corrupt(
                "receipt record version exhausted u64",
            ))?;
    Ok(StoredActiveReceiptV1 {
        schema_version: RECEIPT_RECORD_SCHEMA_VERSION,
        mutation_sequence,
        record_version,
        key: expected.record.key.clone(),
        key_digest: expected.record.key_digest.clone(),
        lifecycle: StoredActiveLifecycleV1::CompletedTaskHandoffDeletion {
            prior_record_version: expected.record.record_version,
            prior_mutation_sequence: expected.record.mutation_sequence,
            prior_created_at_epoch_ms,
            prior_task_version,
            workspace_identity_hash,
            task_link_digest,
            task_bound_lifecycle_link_version: confirmed_terminal_bound.lifecycle_link_version(),
            task_bound_mutation_sequence: confirmed_terminal_bound.mutation_sequence(),
            task_record_version: confirmed_terminal_bound.task_record_version(),
            bind_epoch_ms: confirmed_terminal_bound.terminal_epoch_ms(),
            phase,
            terminal_staged: true,
        },
    })
}

pub(super) fn build_acknowledgement_commit_record(
    expected: &CatalogEntry,
    terminal_digest: TerminalDigest,
    acknowledged_at_epoch_ms: u64,
    mutation_sequence: u64,
) -> Result<StoredActiveReceiptV1, ReceiptLedgerError> {
    if !matches!(
        &expected.record.lifecycle,
        StoredActiveLifecycleV1::DirectTerminalUnacked { .. }
    ) {
        return Err(ReceiptLedgerError::Corrupt(
            "acknowledgement commit witness requires a Direct predecessor",
        ));
    }
    let record_version =
        expected
            .record
            .record_version
            .checked_next()
            .ok_or(ReceiptLedgerError::Corrupt(
                "receipt record version exhausted u64",
            ))?;
    Ok(StoredActiveReceiptV1 {
        schema_version: RECEIPT_RECORD_SCHEMA_VERSION,
        mutation_sequence,
        record_version,
        key: expected.record.key.clone(),
        key_digest: expected.record.key_digest.clone(),
        lifecycle: StoredActiveLifecycleV1::AcknowledgementCommit {
            terminal_digest,
            acknowledged_at_epoch_ms,
            prior_record_version: expected.record.record_version,
            prior_mutation_sequence: expected.record.mutation_sequence,
        },
    })
}

pub(super) fn build_acknowledged_tombstone_record_from_witness(
    witness: &CatalogEntry,
) -> Result<StoredActiveReceiptV1, ReceiptLedgerError> {
    let (terminal_digest, acknowledged_at_epoch_ms) = match &witness.record.lifecycle {
        StoredActiveLifecycleV1::AcknowledgementCommit {
            terminal_digest,
            acknowledged_at_epoch_ms,
            ..
        } => (terminal_digest.clone(), *acknowledged_at_epoch_ms),
        _ => {
            return Err(ReceiptLedgerError::Corrupt(
                "compact acknowledgement requires an acknowledgement witness",
            ))
        }
    };
    Ok(StoredActiveReceiptV1 {
        schema_version: RECEIPT_RECORD_SCHEMA_VERSION,
        mutation_sequence: 0,
        record_version: witness.record.record_version,
        key: witness.record.key.clone(),
        key_digest: witness.record.key_digest.clone(),
        lifecycle: StoredActiveLifecycleV1::AcknowledgedTombstone {
            terminal_digest,
            acknowledged_at_epoch_ms,
        },
    })
}
