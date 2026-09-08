//! Instrumentation points of the protocol-v5 runtime.
//!
//! The runtime reports what it does and asks before a few decisions through
//! one object. Production installs [`NoHooks`], whose every method is a no-op;
//! the ReceiptLedger contract harness installs its own implementation under
//! the `receipt-ledger-test-support` feature. The runtime itself never
//! branches on that feature: every event, pause point and injected decision
//! is a production type, and the closed lists below name exactly where the
//! runtime can be observed or paused.

use super::V5ReceiptRuntime;
use crate::application::invocation_store_v5::V5StoredInvocationRecord;
use crate::application::receipt_ledger::{
    CommittedDirectPublication, ReceiptLedgerError, TaskBoundReceipt, TaskHandoffActorBoundReceipt,
    TaskPromisedActorBoundReceipt, TaskPromisedUnboundReceipt, TaskTerminalBoundReceipt,
    TaskTerminalReceiptBackedReceipt, V5CanonicalTerminal,
};
use crate::domain::invocation::{DomainResult, InvocationId, SafeIdentityHash};
use serde::{Deserialize, Serialize};
use std::any::Any;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// One observable step of the runtime, in the order a request meets them.
/// A few steps are raised only by the observer's own probes and leases
/// (gates, listener and actor counts, capacity probes), so production never
/// constructs them; they stay in the one closed vocabulary the contract
/// harness reads.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum V5ReceiptRuntimeEventKind {
    StrictEnvelopeParsed,
    V5ReceiptRuntimeEntered,
    V5ExecutorEntered,
    CanonicalV13ServiceEntered,
    ReceiptReserved,
    ValidationEntered,
    AdmissionEntered,
    ActorBoundCommitted,
    ReceiptBegunCommitted,
    PrepareEntered,
    ExecuteEntered,
    UnboundPromiseCommitted,
    BoundHandoffCommitted,
    BoundHandoffTerminalStaged,
    TaskBoundCommitted,
    TaskLinkCapacityReserved,
    TaskLinkCapacityRejected,
    TaskStoreCreateAttempted,
    TaskStoreCapacityInvariantViolation,
    TaskStoreCreated,
    TaskLinkReservationConverted,
    FalseCancelObservationReached,
    TaskStoreWorkingReadback,
    TaskStoreTerminalCommitted,
    TaskStoreTerminalReadback,
    TaskTerminalBoundCommitted,
    TokenSignalled,
    MarkReservedBegunBlocked,
    CancelCommitBlocked,
    TaskStoreReadbackBeforeBind,
    CancelCommitted,
    OperationCompleted,
    LeaseReleased,
    ListenerPublished,
    ListenerClosed,
    CancelReservationConverted,
    ResultSerialized,
    ReceiptTerminalCommitted,
    FinalResultProjected,
    AcknowledgementCommitted,
}

/// The four callbacks of an invocation whose entry the harness counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum V5Stage {
    Validation,
    Admission,
    Prepare,
    Execute,
}

/// Where the runtime can be held by an observer. A pause is a no-op unless
/// the installed hooks hold that point.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum V5PausePoint {
    ValidationEntered,
    AdmissionEntered,
    ActorBound,
    BeforePrepare,
    PrepareEntered,
    BeforeTaskStoreCreate,
    AfterWorkingReadback,
    AfterFalseCancelObservation,
    BeforeReceiptBegun,
    BeforeTaskTerminalReceipt,
    AfterCancelReservationConvertedBeforeTerminal,
    AfterTaskStoreReadbackBeforeTaskBound,
    BeforeMarkReservedBegunGateAcquire,
    BeforeCancelGateAcquire,
    AfterTaskStoreTerminalBeforeLifecycleLinkTerminal,
    BeforeRetirementSnapshot,
}

/// An injected workspace admission failure, in place of the real bind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum V5AdmissionRejection {
    Invalid,
    Capacity,
    RegistryFailed,
}

/// A durable-store fault the hooks may inject exactly once.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum V5StoreFaultPoint {
    AfterTerminalPayloadRenameBeforeDirectorySync,
    AfterTaskCreateRenameBeforeDirectorySync,
}

/// Everything the runtime reports or asks. Every method has a no-op default:
/// an implementation overrides only what it observes or decides.
#[allow(unused_variables)]
pub(crate) trait V5RuntimeHooks: Send + Sync {
    /// The concrete observer, for the harness that installed it.
    #[allow(dead_code)]
    fn as_any(&self) -> &dyn Any;

    // --- observation ---

    fn event(&self, event: V5ReceiptRuntimeEventKind, epoch_ms: u64) {}

    fn stage_entered(&self, stage: V5Stage) {}

    fn task_store_create_attempted(&self) {}

    fn restart_requested(&self) {}

    /// The process stops admitting and dies. `grace` is the elapsed grace of
    /// the watchdog that fired, if one did.
    fn forced_process_exit(&self, grace: Option<Duration>) {}

    /// Held while the listener is published; dropping it closes the count.
    fn listener_lease(&self) -> Option<Box<dyn Any + Send>> {
        None
    }

    /// Held while a submit session owns an actor; dropping it releases it.
    fn actor_lease(&self) -> Option<Box<dyn Any + Send>> {
        None
    }

    fn runtime_opened(&self, runtime: &Arc<V5ReceiptRuntime>) {}

    fn direct_publication(
        &self,
        publication: &CommittedDirectPublication,
        response_kind: &'static str,
        origin: &'static str,
        oversized_candidate: Option<&DomainResult>,
    ) {
    }

    fn receipt_backed_terminal(
        &self,
        committed: &TaskTerminalReceiptBackedReceipt,
    ) -> Result<(), ReceiptLedgerError> {
        Ok(())
    }

    fn actor_workspace_identity(&self, identity: &SafeIdentityHash) {}

    fn callback_invocation_id(&self, invocation_id: InvocationId) {}

    fn bound_task(&self, record: &V5StoredInvocationRecord, bound: &TaskBoundReceipt) {}

    fn terminal_bound_task(
        &self,
        record: &V5StoredInvocationRecord,
        link: &TaskTerminalBoundReceipt,
    ) {
    }

    fn promised_actor_binding(
        &self,
        promised: &TaskPromisedUnboundReceipt,
        actor_promised: &TaskPromisedActorBoundReceipt,
        bound: &TaskBoundReceipt,
    ) {
    }

    fn bound_task_start_authorization(
        &self,
        authorized: &TaskBoundReceipt,
        record: &V5StoredInvocationRecord,
        begun: &TaskBoundReceipt,
    ) {
    }

    fn staged_terminal_preparation(
        &self,
        staged: &TaskHandoffActorBoundReceipt,
    ) -> Result<(), ReceiptLedgerError> {
        Ok(())
    }

    fn staged_terminal_publication(
        &self,
        handoff: &TaskHandoffActorBoundReceipt,
        provisional: &V5StoredInvocationRecord,
        terminal_record: &V5StoredInvocationRecord,
        link: &TaskTerminalBoundReceipt,
    ) -> Result<(), ReceiptLedgerError> {
        Ok(())
    }

    fn bound_terminal_publication(
        &self,
        bound: &TaskBoundReceipt,
        provisional: &V5StoredInvocationRecord,
        terminal_record: &V5StoredInvocationRecord,
        link: &TaskTerminalBoundReceipt,
        terminal: &V5CanonicalTerminal,
        epoch_ms: u64,
    ) -> Result<(), ReceiptLedgerError> {
        Ok(())
    }

    // --- synchronization ---

    /// Holds the runtime at `point` until the observer releases it or the
    /// deadline passes. Production never holds.
    fn pause(&self, point: V5PausePoint, deadline: Instant) -> Result<(), ReceiptLedgerError> {
        Ok(())
    }

    /// Whether an observer holds `point`: some commits run under a bulk
    /// budget while a pause is installed there.
    fn holds(&self, point: V5PausePoint) -> bool {
        false
    }

    /// The deadline a commit runs under at a held point: the observer may
    /// widen it while it inspects the durable state between two steps.
    fn commit_deadline_at(&self, point: V5PausePoint, deadline: Instant) -> Instant {
        deadline
    }

    /// Whether the observer already simulated the death of this process.
    fn process_exited(&self) -> bool {
        false
    }

    /// Whether anyone observes at all: an observer may promote a receipt while
    /// the runtime is paused, so the runtime re-reads its durable state after
    /// each pause only when one is installed.
    fn observing(&self) -> bool {
        false
    }

    /// Whether a fail-stopped runtime releases its authority instead of
    /// keeping it until process death: the contract harness runs the daemon
    /// in a thread and joins it as its process-death boundary.
    fn releases_authority_on_fail_stop(&self) -> bool {
        false
    }

    fn acquire_lifecycle_gate(
        &self,
        label: &'static str,
        deadline: Instant,
    ) -> Result<(), ReceiptLedgerError> {
        Ok(())
    }

    fn release_lifecycle_gate(&self, label: &'static str) {}

    fn wait_for_gate_cancel(&self, deadline: Instant) -> Result<(), ReceiptLedgerError> {
        Ok(())
    }

    fn release_pre_actor_pauses(&self) {}

    // --- injected decisions ---

    fn validation_rejects(&self) -> bool {
        false
    }

    fn admission_rejection(&self) -> Option<V5AdmissionRejection> {
        None
    }

    fn prepare_rejects(&self) -> bool {
        false
    }

    fn crash_after_side_effect(&self) -> bool {
        false
    }

    fn store_fault(&self, point: V5StoreFaultPoint) -> bool {
        false
    }

    fn submit_response_disconnect(&self) -> bool {
        false
    }

    fn ack_response_disconnect(&self) -> bool {
        false
    }

    /// A wider operation and response deadline for the session while the
    /// observer configures a precomputed terminal.
    fn session_deadline_override(&self) -> Option<Instant> {
        None
    }
}

/// Production: nothing observes and nothing is injected.
pub(crate) struct NoHooks;

impl V5RuntimeHooks for NoHooks {
    fn as_any(&self) -> &dyn Any {
        self
    }
}
