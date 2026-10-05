use crate::application::receipt_ledger::{
    canonical_v5_terminal, CancelReservedReceipt, CancelResolution, CanonicalTerminalError,
    DirectTerminalUnackedReceipt, ReceiptState, ReceiptStateKind, ReceiptTerminalOutcome,
    ReserveOutcome, ReservedReceipt, V5CanonicalTerminal,
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
    ExecuteReserved(ReservedReceipt),
    PublishCancelledDirect(CancelledDirectTerminalIntent),
    ExistingDirectTerminal(DirectTerminalUnackedReceipt),
    Rejected(ReceiptDecisionRejection),
}

pub(crate) fn decide_cancel_reserved_submit(
    outcome: ReserveOutcome,
) -> Result<CancelReservedSubmitDecision, CanonicalTerminalError> {
    match outcome {
        ReserveOutcome::Created(reservation) if reservation.cancel_requested() => {
            let terminal = canonical_v5_terminal(&ReceiptTerminalOutcome::Cancelled)?;
            Ok(CancelReservedSubmitDecision::PublishCancelledDirect(
                CancelledDirectTerminalIntent {
                    reservation,
                    terminal,
                },
            ))
        }
        ReserveOutcome::Created(reservation) => {
            Ok(CancelReservedSubmitDecision::ExecuteReserved(reservation))
        }
        ReserveOutcome::ExistingExact(ReceiptState::Reserved(reservation))
            if reservation.cancel_requested() =>
        {
            let terminal = canonical_v5_terminal(&ReceiptTerminalOutcome::Cancelled)?;
            Ok(CancelReservedSubmitDecision::PublishCancelledDirect(
                CancelledDirectTerminalIntent {
                    reservation,
                    terminal,
                },
            ))
        }
        ReserveOutcome::ExistingExact(ReceiptState::DirectTerminalUnacked(receipt)) => Ok(
            CancelReservedSubmitDecision::ExistingDirectTerminal(receipt),
        ),
        ReserveOutcome::ExistingExact(state) => Ok(CancelReservedSubmitDecision::Rejected(
            ReceiptDecisionRejection::new(state),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        decide_cancel_reserved_submit, decide_cancel_resolution, CancelInvocationDecision,
        CancelReservationDisposition, CancelReservedSubmitDecision,
    };
    use crate::application::receipt_ledger::{
        canonical_v5_terminal, receipt_key_digest, request_scope_hash, CancelReservedReceipt,
        CancelResolution, CoreIdentityDigest, DirectTerminalUnackedReceipt,
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
            panic!("a duplicate ordinary reservation must not expose a callback path");
        };
        assert_eq!(rejection.state_kind(), ReceiptStateKind::ReservedUnbound);
        assert_eq!(rejection.into_state(), non_cancelled);
    }
}
