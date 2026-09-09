//! Порт ledger поверх хранилища: адаптер, через который рантайм видит
//! квитанции. Своей логики у него нет — он переводит вызовы порта в
//! методы хранилища.

use super::*;

impl ReceiptLedgerPort for ReceiptLedgerStore {
    fn snapshot_catalog(
        &mut self,
        authority: ReceiptLedgerCatalogSnapshotAuthority,
        deadline: Instant,
    ) -> Result<ReceiptLedgerCatalogSnapshot, ReceiptLedgerError> {
        ReceiptLedgerStore::snapshot_catalog(self, authority, deadline)
    }

    fn generation(&mut self, deadline: Instant) -> Result<u64, ReceiptLedgerError> {
        check_deadline(deadline)?;
        let generation = ReceiptLedgerStore::generation(self)?;
        check_deadline(deadline)?;
        Ok(generation)
    }

    fn rotate_generation_for_test(&mut self, deadline: Instant) -> Result<u64, ReceiptLedgerError> {
        ReceiptLedgerStore::rotate_generation_for_test(self, deadline)
    }

    fn publish_cancelled_direct_batch(
        &mut self,
        requests: Vec<(ReceiptKey, OriginalCutoffDescriptor)>,
        terminal_epoch_ms: u64,
        terminal: V5CanonicalTerminal,
        deadline: Instant,
    ) -> Result<Vec<DirectTerminalUnackedReceipt>, ReceiptLedgerError> {
        ReceiptLedgerStore::publish_cancelled_direct_batch(
            self,
            requests,
            terminal_epoch_ms,
            terminal,
            deadline,
        )
    }

    fn reserve_batch(
        &mut self,
        requests: Vec<(ReceiptKey, OriginalCutoffDescriptor)>,
        deadline: Instant,
    ) -> Result<Vec<ReserveOutcome>, ReceiptLedgerError> {
        ReceiptLedgerStore::reserve_batch(self, requests, deadline)
    }

    fn bind_reserved_actor_batch(
        &mut self,
        requests: Vec<(ReceiptKey, ReceiptVersion, SafeIdentityHash)>,
        deadline: Instant,
    ) -> Result<Vec<ReservedReceipt>, ReceiptLedgerError> {
        ReceiptLedgerStore::bind_reserved_actor_batch(self, requests, deadline)
    }

    fn mark_reserved_begun_batch(
        &mut self,
        requests: Vec<(ReceiptKey, ReceiptVersion, SafeIdentityHash)>,
        deadline: Instant,
    ) -> Result<Vec<ReservedReceipt>, ReceiptLedgerError> {
        ReceiptLedgerStore::mark_reserved_begun_batch(self, requests, deadline)
    }

    fn publish_direct_terminal_batch(
        &mut self,
        requests: Vec<(ReceiptKey, ReceiptVersion, u64, V5CanonicalTerminal)>,
        deadline: Instant,
    ) -> Result<Vec<CommittedDirectPublication>, ReceiptLedgerError> {
        ReceiptLedgerStore::publish_direct_terminal_batch(self, requests, deadline)
    }

    fn acknowledge_direct_batch(
        &mut self,
        requests: Vec<(ReceiptKey, TerminalDigest, u64)>,
        deadline: Instant,
    ) -> Result<Vec<AcknowledgedTombstoneReceipt>, ReceiptLedgerError> {
        ReceiptLedgerStore::acknowledge_direct_batch(self, requests, deadline)
    }

    fn reserve(
        &mut self,
        key: ReceiptKey,
        original_cutoff: OriginalCutoffDescriptor,
        deadline: Instant,
    ) -> Result<ReserveOutcome, ReceiptLedgerError> {
        ReceiptLedgerStore::reserve(self, key, original_cutoff, deadline)
    }

    fn bind_reserved_actor(
        &mut self,
        key: &ReceiptKey,
        expected_version: ReceiptVersion,
        bound_workspace_identity: SafeIdentityHash,
        deadline: Instant,
    ) -> Result<ReservedReceipt, ReceiptLedgerError> {
        ReceiptLedgerStore::bind_reserved_actor(
            self,
            key,
            expected_version,
            bound_workspace_identity,
            deadline,
        )
    }

    fn mark_reserved_begun(
        &mut self,
        key: &ReceiptKey,
        expected_version: ReceiptVersion,
        deadline: Instant,
    ) -> Result<ReservedReceipt, ReceiptLedgerError> {
        ReceiptLedgerStore::mark_reserved_begun(self, key, expected_version, deadline)
    }

    fn promise_task_unbound(
        &mut self,
        key: &ReceiptKey,
        expected_version: ReceiptVersion,
        created_at_epoch_ms: u64,
        ttl_ms: u64,
        poll_interval_ms: u64,
        deadline: Instant,
    ) -> Result<TaskPromisedUnboundReceipt, ReceiptLedgerError> {
        ReceiptLedgerStore::promise_task_unbound(
            self,
            key,
            expected_version,
            created_at_epoch_ms,
            ttl_ms,
            poll_interval_ms,
            deadline,
        )
    }

    fn bind_promised_task_actor(
        &mut self,
        key: &ReceiptKey,
        expected_version: ReceiptVersion,
        workspace_identity_hash: SafeIdentityHash,
        deadline: Instant,
    ) -> Result<TaskPromisedActorBoundReceipt, ReceiptLedgerError> {
        ReceiptLedgerStore::bind_promised_task_actor(
            self,
            key,
            expected_version,
            workspace_identity_hash,
            deadline,
        )
    }

    fn begin_bound_task_handoff(
        &mut self,
        key: &ReceiptKey,
        expected_version: ReceiptVersion,
        created_at_epoch_ms: u64,
        ttl_ms: u64,
        poll_interval_ms: u64,
        deadline: Instant,
    ) -> Result<TaskHandoffActorBoundReceipt, ReceiptLedgerError> {
        ReceiptLedgerStore::begin_bound_task_handoff(
            self,
            key,
            expected_version,
            created_at_epoch_ms,
            ttl_ms,
            poll_interval_ms,
            deadline,
        )
    }

    fn stage_bound_task_handoff_terminal(
        &mut self,
        key: &ReceiptKey,
        expected_version: ReceiptVersion,
        terminal_epoch_ms: u64,
        terminal: V5CanonicalTerminal,
        certificate: StagedTerminalTransferCertificate,
        deadline: Instant,
    ) -> Result<TaskHandoffActorBoundReceipt, ReceiptLedgerError> {
        ReceiptLedgerStore::stage_bound_task_handoff_terminal(
            self,
            key,
            expected_version,
            terminal_epoch_ms,
            terminal,
            certificate,
            deadline,
        )
    }

    fn complete_bound_task_handoff(
        &mut self,
        key: &ReceiptKey,
        expected_version: ReceiptVersion,
        confirmed_task_bound: TaskBoundReceipt,
        deadline: Instant,
    ) -> Result<TaskBoundReceipt, ReceiptLedgerError> {
        ReceiptLedgerStore::complete_bound_task_handoff(
            self,
            key,
            expected_version,
            confirmed_task_bound,
            deadline,
        )
    }

    fn complete_staged_task_handoff(
        &mut self,
        key: &ReceiptKey,
        expected_version: ReceiptVersion,
        confirmed_terminal_bound: TaskTerminalBoundReceipt,
        deadline: Instant,
    ) -> Result<TaskTerminalBoundReceipt, ReceiptLedgerError> {
        ReceiptLedgerStore::complete_staged_task_handoff(
            self,
            key,
            expected_version,
            confirmed_terminal_bound,
            deadline,
        )
    }

    fn retain_begun_task_after_link_capacity(
        &mut self,
        key: &ReceiptKey,
        expected_version: ReceiptVersion,
        proven_link_capacity: ProvenTaskLinkCapacity,
        deadline: Instant,
    ) -> Result<TaskReceiptOwnedActorBoundReceipt, ReceiptLedgerError> {
        ReceiptLedgerStore::retain_begun_task_after_link_capacity(
            self,
            key,
            expected_version,
            proven_link_capacity,
            deadline,
        )
    }

    fn request_task_cancel(
        &mut self,
        key: &ReceiptKey,
        expected_state: TaskCancellationReceipt,
        deadline: Instant,
    ) -> Result<TaskCancellationReceipt, ReceiptLedgerError> {
        ReceiptLedgerStore::request_task_cancel(self, key, expected_state, deadline)
    }

    fn publish_receipt_backed_task_terminal(
        &mut self,
        key: &ReceiptKey,
        expected_state: TaskCancellationReceipt,
        terminal_epoch_ms: u64,
        terminal: V5CanonicalTerminal,
        deadline: Instant,
    ) -> Result<TaskTerminalReceiptBackedReceipt, ReceiptLedgerError> {
        ReceiptLedgerStore::publish_receipt_backed_task_terminal(
            self,
            key,
            expected_state,
            terminal_epoch_ms,
            terminal,
            deadline,
        )
    }

    fn request_cancel_or_reserve(
        &mut self,
        key: ReceiptKey,
        cancel_reserved_at_epoch_ms: u64,
        deadline: Instant,
    ) -> Result<CancelResolution, ReceiptLedgerError> {
        ReceiptLedgerStore::request_cancel_or_reserve(
            self,
            key,
            cancel_reserved_at_epoch_ms,
            deadline,
        )
    }

    fn expire_cancel_reserved(
        &mut self,
        key: ReceiptKey,
        expected_version: ReceiptVersion,
        expected_mutation_sequence: u64,
        observed_at_epoch_ms: u64,
        deadline: Instant,
    ) -> Result<CancelExpiryOutcome, ReceiptLedgerError> {
        ReceiptLedgerStore::expire_cancel_reserved(
            self,
            key,
            expected_version,
            expected_mutation_sequence,
            observed_at_epoch_ms,
            deadline,
        )
    }

    fn publish_direct_terminal(
        &mut self,
        key: &ReceiptKey,
        expected_version: ReceiptVersion,
        terminal_epoch_ms: u64,
        terminal: V5CanonicalTerminal,
        deadline: Instant,
    ) -> Result<CommittedDirectPublication, ReceiptLedgerError> {
        ReceiptLedgerStore::publish_direct_terminal_publication(
            self,
            key,
            expected_version,
            terminal_epoch_ms,
            terminal,
            deadline,
        )
    }

    fn acknowledge_direct(
        &mut self,
        key: &ReceiptKey,
        terminal_digest: &TerminalDigest,
        acknowledged_at_epoch_ms: u64,
        deadline: Instant,
    ) -> Result<AcknowledgedTombstoneReceipt, ReceiptLedgerError> {
        ReceiptLedgerStore::acknowledge_direct(
            self,
            key,
            terminal_digest,
            acknowledged_at_epoch_ms,
            deadline,
        )
    }

    fn reclaim_expired_tombstones(
        &mut self,
        observed_at_epoch_ms: u64,
        deadline: Instant,
    ) -> Result<usize, ReceiptLedgerError> {
        ReceiptLedgerStore::reclaim_expired_tombstones(self, observed_at_epoch_ms, deadline)
    }

    fn recover(
        &mut self,
        key: &ReceiptKey,
        deadline: Instant,
    ) -> Result<ReceiptState, ReceiptLedgerError> {
        self.recover_exact(key, deadline)
    }

    fn recover_at(
        &mut self,
        key: &ReceiptKey,
        observed_at_epoch_ms: u64,
        deadline: Instant,
    ) -> Result<ReceiptState, ReceiptLedgerError> {
        self.recover_exact_at(key, observed_at_epoch_ms, deadline)
    }

    fn resolve_task(
        &mut self,
        task_id: TaskId,
        deadline: Instant,
    ) -> Result<ReceiptState, ReceiptLedgerError> {
        self.resolve_task_exact(task_id, deadline)
    }
}
