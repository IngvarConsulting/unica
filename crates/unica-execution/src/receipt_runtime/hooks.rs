//! Instrumentation points of the protocol-v5 runtime.
//!
//! The runtime reports what it does and asks before a few decisions through
//! one object. Production installs [`NoHooks`], which uses the normal defaults;
//! the ReceiptLedger contract harness installs its own implementation under
//! the `receipt-ledger-test-support` feature. The runtime itself never
//! branches on that feature: every event, pause point and injected decision
//! is a production type, and the closed lists below name exactly where the
//! runtime can be observed or paused.

use super::ports::InvocationHost;
use super::V5ReceiptRuntime;
use crate::invocation_store_v5::V5StoredInvocationRecord;
use crate::receipt_ledger::{
    CommittedDirectPublication, ReceiptLedgerError, TaskBoundReceipt, TaskHandoffActorBoundReceipt,
    TaskPromisedActorBoundReceipt, TaskPromisedUnboundReceipt, TaskTerminalBoundReceipt,
    TaskTerminalReceiptBackedReceipt, V5CanonicalTerminal,
};
use serde::{Deserialize, Serialize};
use std::any::Any;
use std::sync::Arc;
use std::time::{Duration, Instant};
use unica_execution_contract::invocation::{DomainResult, InvocationId, SafeIdentityHash};

/// One observable step of the runtime, in the order a request meets them.
/// A few steps are raised only by the observer's own probes and leases
/// (gates, listener and actor counts, capacity probes), so production never
/// constructs them; they stay in the one closed vocabulary the contract
/// harness reads.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum V5ReceiptRuntimeEventKind {
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
pub enum V5Stage {
    Validation,
    Admission,
    Prepare,
    Execute,
}

/// The owners one idle iteration of the listener reads. The daemon stays
/// while any of them holds and exits after its grace once none does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct V5ListenerOwners {
    /// An authenticated client session holds its owner lease.
    pub client_leases: bool,
    /// A connection is still in its handshake.
    pub handshakes: bool,
    /// A Task or an inline attempt has registered its cancellation.
    pub task_cancellations: bool,
    /// Workspace work keeps an actor: a running producer, a needed lease or a
    /// saved plan.
    pub retained_workspace_work: bool,
}

/// Where the runtime can be held by an observer. A pause is a no-op unless
/// the installed hooks hold that point.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum V5PausePoint {
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
    /// A submit holds its key's lock, has found no Task link for the key and
    /// has not reserved it yet.
    BeforeSubmitReservation,
    /// A cancel holds its key's lock, has found no Task link for the key and
    /// has not written its ledger cancel yet.
    BeforeCancelReservation,
    /// A cancel has read the bound Task it is about to mark and has not
    /// written the mark yet.
    BeforeTaskCancelWrite,
    /// A begun attempt has passed its early look for a Task the session
    /// handler materialized and has not claimed the cutoff yet.
    BeforeCutoffClaim,
}

/// An injected workspace admission failure, in place of the real bind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum V5AdmissionRejection {
    Invalid,
    Capacity,
    RegistryFailed,
}

/// A durable-store fault the hooks may inject exactly once.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum V5StoreFaultPoint {
    AfterTerminalPayloadRenameBeforeDirectorySync,
    AfterTaskCreateRenameBeforeDirectorySync,
}

/// One step of retiring an expired terminal Task; the hooks may fail any of
/// them for one record to show that the failure stays with that record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum V5RetirementStep {
    Begin,
    Authorize,
    Delete,
    Finalize,
}

/// Everything the runtime reports or asks. Every method has a production default:
/// an implementation overrides only what it observes or decides.
#[allow(unused_variables)]
pub trait V5RuntimeHooks<H: InvocationHost>: Send + Sync {
    /// The concrete observer, for the harness that installed it.
    #[allow(dead_code)]
    fn as_any(&self) -> &dyn Any;

    /// Starts the actual connection handler; a failed spawn drops its owned
    /// transport and admission slot without running the handler.
    fn spawn_connection_handler(
        &self,
        handler: Box<dyn FnOnce() + Send + 'static>,
    ) -> std::io::Result<std::thread::JoinHandle<()>> {
        std::thread::Builder::new()
            .name("unica-daemon-connection".into())
            .spawn(handler)
    }

    // --- observation ---

    /// Observes a real authenticated Task wait before its result is awaited.
    fn wait_task_received(&self, task_id: unica_execution_contract::invocation::TaskId) {}

    fn event(&self, event: V5ReceiptRuntimeEventKind, epoch_ms: u64) {}

    /// Observes a real accepted transport before owner admission.
    fn before_owner_handshake(&self, peer_port: u16) {}

    /// Observes the owner-registry read in listener idle maintenance.
    fn after_idle_owner_leases_read(&self, empty: bool) {}

    fn listener_ownership_observed(&self, no_active_work: bool) {}

    /// Whether the listener should read every owner for
    /// [`Self::listener_owners_observed`] in each idle iteration.
    fn observes_listener_owners(&self) -> bool {
        false
    }

    /// Observes which owners keep the listener alive in one idle iteration.
    fn listener_owners_observed(&self, owners: V5ListenerOwners) {}

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

    fn runtime_opened(&self, runtime: &Arc<V5ReceiptRuntime<H>>) {}

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

    /// Fails `step` of retiring the expired Task `task_id` before it touches
    /// either store, as a store failure that would otherwise fail-stop.
    fn retirement_step_fault(
        &self,
        step: V5RetirementStep,
        task_id: unica_execution_contract::invocation::TaskId,
    ) -> bool {
        false
    }

    fn submit_response_disconnect(&self) -> bool {
        false
    }

    fn ack_response_disconnect(&self) -> bool {
        false
    }

    /// Invoked after an exact receipt recovery is answered and before writing
    /// the answer. Tests lose it to close the frontend recovery window.
    fn recover_response_disconnect(&self) -> bool {
        false
    }

    /// Invoked only after the durable exact Cancel has succeeded and before
    /// writing its answer. Tests may delay or lose that committed answer.
    fn cancel_response_disconnect(&self) -> bool {
        false
    }

    /// A wider operation and response deadline for the session while the
    /// observer configures a precomputed terminal.
    fn session_deadline_override(&self) -> Option<Instant> {
        None
    }
}

/// Production: nothing observes and nothing is injected.
pub struct NoHooks;

impl<H: InvocationHost> V5RuntimeHooks<H> for NoHooks {
    fn as_any(&self) -> &dyn Any {
        self
    }
}
