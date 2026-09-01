use crate::application::receipt_ledger::{
    canonical_v5_terminal, CancelExpiryOutcome, CancelReservedReceipt, CancelResolution,
    CanonicalTerminalError, DirectTerminalUnackedReceipt, ReceiptKey, ReceiptState,
    ReceiptStateKind, ReceiptTerminalOutcome, ReceiptVersion, ReserveOutcome, ReservedReceipt,
    V5CanonicalTerminal,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CancelReservationDisposition {
    NewlyReserved,
    ExistingExact,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ReceiptDecisionRejection {
    state: Box<ReceiptState>,
}

impl ReceiptDecisionRejection {
    fn new(state: ReceiptState) -> Self {
        Self {
            state: Box::new(state),
        }
    }

    pub(crate) fn state_kind(&self) -> ReceiptStateKind {
        self.state.kind()
    }

    pub(crate) fn into_state(self) -> ReceiptState {
        *self.state
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum CancelInvocationDecision {
    Accepted {
        disposition: CancelReservationDisposition,
        receipt: CancelReservedReceipt,
    },
    ExistingDirectTerminal(DirectTerminalUnackedReceipt),
    Rejected(ReceiptDecisionRejection),
}

pub(crate) fn decide_cancel_resolution(resolution: CancelResolution) -> CancelInvocationDecision {
    match resolution {
        CancelResolution::NewlyReserved(receipt) => CancelInvocationDecision::Accepted {
            disposition: CancelReservationDisposition::NewlyReserved,
            receipt,
        },
        CancelResolution::ExistingExact(receipt) => CancelInvocationDecision::Accepted {
            disposition: CancelReservationDisposition::ExistingExact,
            receipt,
        },
        CancelResolution::ExistingWinner(winner) => match *winner {
            ReceiptState::DirectTerminalUnacked(receipt) => {
                CancelInvocationDecision::ExistingDirectTerminal(receipt)
            }
            state => CancelInvocationDecision::Rejected(ReceiptDecisionRejection::new(state)),
        },
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CancelledDirectTerminalIntent {
    reservation: ReservedReceipt,
    terminal: V5CanonicalTerminal,
}

impl CancelledDirectTerminalIntent {
    pub(crate) fn reservation(&self) -> &ReservedReceipt {
        &self.reservation
    }

    pub(crate) fn terminal(&self) -> &V5CanonicalTerminal {
        &self.terminal
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum CancelReservedSubmitDecision {
    PublishCancelledDirect(CancelledDirectTerminalIntent),
    ExistingDirectTerminal(DirectTerminalUnackedReceipt),
    Rejected(ReceiptDecisionRejection),
}

pub(crate) fn decide_cancel_reserved_submit(
    outcome: ReserveOutcome,
) -> Result<CancelReservedSubmitDecision, CanonicalTerminalError> {
    match outcome.into_state() {
        ReceiptState::Reserved(reservation) if reservation.cancel_requested() => {
            let terminal = canonical_v5_terminal(&ReceiptTerminalOutcome::Cancelled)?;
            Ok(CancelReservedSubmitDecision::PublishCancelledDirect(
                CancelledDirectTerminalIntent {
                    reservation,
                    terminal,
                },
            ))
        }
        ReceiptState::DirectTerminalUnacked(receipt) => Ok(
            CancelReservedSubmitDecision::ExistingDirectTerminal(receipt),
        ),
        state => Ok(CancelReservedSubmitDecision::Rejected(
            ReceiptDecisionRejection::new(state),
        )),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CancelReservedExpiryIntent {
    receipt: CancelReservedReceipt,
    observed_at_epoch_ms: u64,
}

impl CancelReservedExpiryIntent {
    pub(crate) fn key(&self) -> &ReceiptKey {
        self.receipt.key()
    }

    pub(crate) fn expected_version(&self) -> ReceiptVersion {
        self.receipt.record_version()
    }

    pub(crate) fn expected_mutation_sequence(&self) -> u64 {
        self.receipt.mutation_sequence()
    }

    pub(crate) fn observed_at_epoch_ms(&self) -> u64 {
        self.observed_at_epoch_ms
    }

    pub(crate) fn into_receipt(self) -> CancelReservedReceipt {
        self.receipt
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum CancelReservedRecoveryDecision {
    Current(Box<ReceiptState>),
    Expire(CancelReservedExpiryIntent),
}

pub(crate) fn classify_recovered_receipt(
    state: ReceiptState,
    observed_at_epoch_ms: u64,
) -> CancelReservedRecoveryDecision {
    match state {
        ReceiptState::CancelReserved(receipt)
            if observed_at_epoch_ms >= receipt.expires_at_epoch_ms() =>
        {
            CancelReservedRecoveryDecision::Expire(CancelReservedExpiryIntent {
                receipt,
                observed_at_epoch_ms,
            })
        }
        state => CancelReservedRecoveryDecision::Current(Box::new(state)),
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum CancelReservedExpiryDecision {
    Expired,
    Current(Box<ReceiptState>),
}

pub(crate) fn classify_cancel_reserved_expiry_outcome(
    outcome: CancelExpiryOutcome,
) -> CancelReservedExpiryDecision {
    match outcome {
        CancelExpiryOutcome::Expired | CancelExpiryOutcome::Missing => {
            CancelReservedExpiryDecision::Expired
        }
        CancelExpiryOutcome::NotDue(receipt) => {
            CancelReservedExpiryDecision::Current(Box::new(ReceiptState::CancelReserved(receipt)))
        }
        CancelExpiryOutcome::ExistingWinner(winner) => {
            CancelReservedExpiryDecision::Current(winner)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        classify_cancel_reserved_expiry_outcome, classify_recovered_receipt,
        decide_cancel_reserved_submit, decide_cancel_resolution, CancelInvocationDecision,
        CancelReservationDisposition, CancelReservedExpiryDecision, CancelReservedRecoveryDecision,
        CancelReservedSubmitDecision,
    };
    use crate::application::receipt_ledger::{
        canonical_v5_terminal, receipt_key_digest, request_scope_hash, CancelExpiryOutcome,
        CancelReservedReceipt, CancelResolution, CoreIdentityDigest, DirectTerminalUnackedReceipt,
        OriginalCutoffDescriptor, ReceiptKey, ReceiptRecordHeader, ReceiptState, ReceiptStateKind,
        ReceiptTerminalOutcome, ReceiptVersion, RequestIdentity, ReserveOutcome, ReservedPhase,
        ReservedReceipt, V5ToolIdentity, MAX_RECEIPT_ENTITLEMENT_BYTES,
    };
    use crate::domain::invocation::{InvocationId, NormalizedArgumentsHash, TaskId};

    fn receipt_key() -> ReceiptKey {
        ReceiptKey::new(
            InvocationId::new(),
            TaskId::new(),
            RequestIdentity::new(
                CoreIdentityDigest::from_sha256([0x55; 32]),
                V5ToolIdentity::View,
                NormalizedArgumentsHash::from_sha256([0x11; 32]),
                request_scope_hash("workspace-a").expect("valid request scope"),
            ),
        )
    }

    fn cancel_reserved() -> CancelReservedReceipt {
        CancelReservedReceipt::new(receipt_key(), ReceiptVersion::initial(), 1, 512, 1_000)
            .expect("valid cancellation reservation")
    }

    fn reserved(cancel_requested: bool) -> ReservedReceipt {
        let key = receipt_key();
        ReservedReceipt::new(
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
            cancel_requested,
            MAX_RECEIPT_ENTITLEMENT_BYTES - 512,
        )
    }

    fn direct_terminal() -> DirectTerminalUnackedReceipt {
        let key = receipt_key();
        DirectTerminalUnackedReceipt::new(
            ReceiptRecordHeader::new(
                key.clone(),
                receipt_key_digest(&key),
                ReceiptVersion::new(2).expect("nonzero version"),
                2,
                700,
            ),
            OriginalCutoffDescriptor::new(1_000, 7_000).expect("valid cutoff"),
            2_000,
            canonical_v5_terminal(&ReceiptTerminalOutcome::Cancelled)
                .expect("cancelled terminal is canonical"),
            MAX_RECEIPT_ENTITLEMENT_BYTES - 700,
        )
    }

    #[test]
    fn cancel_resolution_preserves_new_vs_duplicate_acceptance_without_runtime_policy() {
        let receipt = cancel_reserved();
        let newly_reserved =
            decide_cancel_resolution(CancelResolution::NewlyReserved(receipt.clone()));
        let existing_exact =
            decide_cancel_resolution(CancelResolution::ExistingExact(receipt.clone()));

        assert_eq!(
            newly_reserved,
            CancelInvocationDecision::Accepted {
                disposition: CancelReservationDisposition::NewlyReserved,
                receipt: receipt.clone(),
            }
        );
        assert_eq!(
            existing_exact,
            CancelInvocationDecision::Accepted {
                disposition: CancelReservationDisposition::ExistingExact,
                receipt,
            }
        );
    }

    #[test]
    fn cancel_resolution_returns_an_exact_terminal_or_typed_state_rejection() {
        let terminal = direct_terminal();
        assert_eq!(
            decide_cancel_resolution(CancelResolution::ExistingWinner(Box::new(
                ReceiptState::DirectTerminalUnacked(terminal.clone()),
            ))),
            CancelInvocationDecision::ExistingDirectTerminal(terminal)
        );

        let reserved = ReceiptState::Reserved(reserved(false));
        let decision =
            decide_cancel_resolution(CancelResolution::ExistingWinner(Box::new(reserved.clone())));
        let CancelInvocationDecision::Rejected(rejection) = decision else {
            panic!("a nonterminal winner must be rejected by the CR0 decision model");
        };
        assert_eq!(rejection.state_kind(), ReceiptStateKind::ReservedUnbound);
        assert_eq!(rejection.into_state(), reserved);
    }

    #[test]
    fn converted_cancel_reservation_produces_only_a_canonical_cancelled_direct_intent() {
        for outcome in [
            ReserveOutcome::Created(reserved(true)),
            ReserveOutcome::ExistingExact(ReceiptState::Reserved(reserved(true))),
        ] {
            let decision =
                decide_cancel_reserved_submit(outcome).expect("fixed Cancelled is canonical");
            let CancelReservedSubmitDecision::PublishCancelledDirect(intent) = decision else {
                panic!("a converted cancellation reservation must bypass the callback");
            };

            assert!(intent.reservation().cancel_requested());
            assert_eq!(
                intent.terminal().outcome(),
                &ReceiptTerminalOutcome::Cancelled
            );
            assert_eq!(intent.terminal().payload(), br#"{"status":"cancelled"}"#);
            assert_eq!(
                intent.terminal().digest().as_str(),
                "f2d0423d2613a0d09397b750542e4542f7653d78ebd5e0448f1326d09145d9ae"
            );
        }
    }

    #[test]
    fn converted_submit_returns_an_existing_direct_terminal_and_rejects_non_cancelled_work() {
        let terminal = direct_terminal();
        assert_eq!(
            decide_cancel_reserved_submit(ReserveOutcome::ExistingExact(
                ReceiptState::DirectTerminalUnacked(terminal.clone()),
            ))
            .expect("existing terminal needs no canonicalization"),
            CancelReservedSubmitDecision::ExistingDirectTerminal(terminal)
        );

        let non_cancelled = ReceiptState::Reserved(reserved(false));
        let decision =
            decide_cancel_reserved_submit(ReserveOutcome::ExistingExact(non_cancelled.clone()))
                .expect("rejection needs no canonicalization");
        let CancelReservedSubmitDecision::Rejected(rejection) = decision else {
            panic!("CR0 must not expose a callback path for an ordinary reservation");
        };
        assert_eq!(rejection.state_kind(), ReceiptStateKind::ReservedUnbound);
        assert_eq!(rejection.into_state(), non_cancelled);
    }

    #[test]
    fn recovered_cancel_reservation_is_current_before_expiry_and_requests_exact_expiry_at_boundary()
    {
        let receipt = cancel_reserved();
        let state = ReceiptState::CancelReserved(receipt.clone());
        assert_eq!(
            classify_recovered_receipt(state.clone(), receipt.expires_at_epoch_ms() - 1),
            CancelReservedRecoveryDecision::Current(Box::new(state))
        );

        let decision =
            classify_recovered_receipt(ReceiptState::CancelReserved(receipt.clone()), 8_125);
        let CancelReservedRecoveryDecision::Expire(intent) = decision else {
            panic!("the exact absolute expiry must request a witnessed deletion");
        };
        assert_eq!(intent.key(), receipt.key());
        assert_eq!(intent.expected_version(), receipt.record_version());
        assert_eq!(
            intent.expected_mutation_sequence(),
            receipt.mutation_sequence()
        );
        assert_eq!(intent.observed_at_epoch_ms(), 8_125);
        assert_eq!(intent.into_receipt(), receipt);
    }

    #[test]
    fn cancel_expiry_result_closes_missing_and_preserves_not_due_or_racing_winner() {
        assert_eq!(
            classify_cancel_reserved_expiry_outcome(CancelExpiryOutcome::Expired),
            CancelReservedExpiryDecision::Expired
        );
        assert_eq!(
            classify_cancel_reserved_expiry_outcome(CancelExpiryOutcome::Missing),
            CancelReservedExpiryDecision::Expired
        );

        let not_due = cancel_reserved();
        assert_eq!(
            classify_cancel_reserved_expiry_outcome(CancelExpiryOutcome::NotDue(not_due.clone())),
            CancelReservedExpiryDecision::Current(Box::new(ReceiptState::CancelReserved(not_due)))
        );

        let terminal = ReceiptState::DirectTerminalUnacked(direct_terminal());
        assert_eq!(
            classify_cancel_reserved_expiry_outcome(CancelExpiryOutcome::ExistingWinner(Box::new(
                terminal.clone()
            ),)),
            CancelReservedExpiryDecision::Current(Box::new(terminal))
        );
    }

    #[test]
    fn recovery_classification_leaves_non_cancel_receipt_states_exactly_unchanged() {
        let state = ReceiptState::DirectTerminalUnacked(direct_terminal());
        assert_eq!(
            classify_recovered_receipt(state.clone(), u64::MAX),
            CancelReservedRecoveryDecision::Current(Box::new(state))
        );
    }
}
