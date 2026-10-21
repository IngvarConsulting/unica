use super::hooks::V5RuntimeHooks;
use crate::invocation::InvocationResponseDeadline;
use crate::invocation_store::EpochMillisClock;
use crate::receipt_ledger::{CoreIdentityDigest, ReceiptKey, ReceiptKeyDigest};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use unica_execution_contract::invocation::{
    DomainResult, InvocationFailure, SafeIdentityHash, TaskId,
};
use unica_work_control::cancellation::CancellationToken;

/// Scheduling evidence established by the existing product preparation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionClass {
    InlineCandidate,
    KnownLong(KnownLongReason),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KnownLongReason {
    MissingEngine,
    ColdIndex,
    ProviderStartup,
    OccupiedWriteLease,
    ExternalProcess,
}

#[derive(Debug)]
pub enum V5CanonicalPrepareError {
    Direct(Box<DomainResult>),
    Rejected(Box<DomainResult>),
    WorkspaceRegistryFailed,
}

/// Facts read by the receipt owner; the product request and its arguments stay owned once.
pub trait InvocationRequestFacts: Clone + Send + Sync + 'static {
    fn reserved_task_id(&self) -> TaskId;
    fn response_budget_ms(&self) -> u64;
    fn workspace_hint(&self) -> &str;
}

pub trait PreparedInvocation: Send + 'static {
    fn execution_class(&self) -> &ExecutionClass;
    fn execute(self, cancellation: CancellationToken) -> Result<DomainResult, InvocationFailure>;
}

pub trait BoundInvocation: Send + 'static {
    type Prepared: PreparedInvocation;
    fn response_deadline(&self) -> Option<InvocationResponseDeadline>;
    fn workspace_identity_hash(&self) -> &SafeIdentityHash;
    fn prepare(self) -> Result<Self::Prepared, Box<DomainResult>>;
}

/// The one composition binding to the existing product admission and operation owners.
pub trait InvocationHost: Send + Sync + 'static {
    /// Largest supported Body response in bytes, independent of the current client/deadline.
    const MAX_SUPPORTED_BODY_RESPONSE_BYTES: usize;

    type Request: InvocationRequestFacts;
    type Bound: BoundInvocation<Prepared = Self::Prepared>;
    type Prepared: PreparedInvocation;

    fn has_retained_workspace_work(&self) -> bool;
    fn capture_response_deadline(&self, response_budget_ms: u64) -> InvocationResponseDeadline;
    fn bind(
        &self,
        invocation: Self::Request,
        response_deadline: InvocationResponseDeadline,
        admission_cancellation: &CancellationToken,
    ) -> Result<Self::Bound, V5CanonicalPrepareError>;

    fn discover_join_workspace_root(&self, workspace_hint: &str) -> Result<PathBuf, String>;
    fn discover_join_source_set_paths(&self, root: &Path) -> Result<Vec<String>, String>;
}

/// Strict decoding remains deferred until after the original response deadline is captured.
/// Its actual producer owns the original frame and exact Daemon core identity.
pub trait DecodedSubmit<R: InvocationRequestFacts> {
    type Error;
    fn reserved_task_id(&self) -> Option<TaskId>;
    fn response_budget_ms(&self) -> Option<u64>;
    fn into_strict_submit(
        self,
        expected_core: &CoreIdentityDigest,
    ) -> Result<StrictSubmit<R>, Self::Error>;
}

/// The existing strict conversion's immutable request and complete reserve identity.
#[derive(Debug)]
pub struct StrictSubmit<R: InvocationRequestFacts> {
    invocation: R,
    receipt_key: ReceiptKey,
    receipt_key_digest: ReceiptKeyDigest,
}

impl<R: InvocationRequestFacts> StrictSubmit<R> {
    pub fn new(
        invocation: R,
        receipt_key: ReceiptKey,
        receipt_key_digest: ReceiptKeyDigest,
    ) -> Self {
        Self {
            invocation,
            receipt_key,
            receipt_key_digest,
        }
    }

    pub fn invocation(&self) -> &R {
        &self.invocation
    }

    pub fn response_budget_ms(&self) -> u64 {
        self.invocation.response_budget_ms()
    }

    pub fn receipt_key(&self) -> &ReceiptKey {
        &self.receipt_key
    }

    pub fn receipt_key_digest(&self) -> &ReceiptKeyDigest {
        &self.receipt_key_digest
    }

    pub fn into_parts(self) -> (ReceiptKey, u64) {
        (self.receipt_key, self.invocation.response_budget_ms())
    }
}

/// All producers are required; the owner does not manufacture a product host or clock.
pub struct ReceiptRuntimeConfig<H: InvocationHost> {
    pub(crate) core_identity_digest: CoreIdentityDigest,
    pub(crate) host: Arc<dyn Fn() -> Arc<H> + Send + Sync>,
    pub(crate) epoch_clock: Arc<dyn EpochMillisClock>,
    pub(crate) hooks: Arc<dyn V5RuntimeHooks<H>>,
    pub(crate) reconcile_startup: bool,
}

impl<H: InvocationHost> ReceiptRuntimeConfig<H> {
    pub fn new(
        core_identity_digest: CoreIdentityDigest,
        host: Arc<dyn Fn() -> Arc<H> + Send + Sync>,
        epoch_clock: Arc<dyn EpochMillisClock>,
        hooks: Arc<dyn V5RuntimeHooks<H>>,
        reconcile_startup: bool,
    ) -> Self {
        Self {
            core_identity_digest,
            host,
            epoch_clock,
            hooks,
            reconcile_startup,
        }
    }
}
