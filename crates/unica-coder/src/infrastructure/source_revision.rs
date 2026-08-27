use crate::domain::cancellation::CancellationToken;
use crate::domain::code_intelligence::ProviderDeadline;
use crate::domain::source_revision::{
    SourceRevision, SourceRevisionMachine, SourceRevisionState, SourceRevisionTrustLoss,
};
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::deadline_lock::{
    DeadlineLock, DeadlineLockError, DeadlineLockErrorKind, Recover,
};
#[cfg(test)]
use crate::infrastructure::platform::filesystem::supports_retained_root_replacement_test;
use crate::infrastructure::platform::filesystem::{
    host_directory_component_names_equivalent, is_nofollow_link_error, FileIdentity,
    RetainedChildCapability, RetainedDirectoryCapability,
};
use crate::infrastructure::platform::source_revision_fence::{
    deferred_platform_fence, platform_fence, FenceCapability, FenceOutcome, SourceRevisionFence,
};
use crate::infrastructure::revision_artifact_policy::{
    RevisionArtifactDisposition, RevisionArtifactPolicy,
};
use crate::infrastructure::workspace_actor::ActorRevisionServiceAuthority;
// The corpus scan and the platform fence must prune the same directory: a
// fence that reports what the manifest ignores can never converge.
use crate::infrastructure::native_operations::apply::{StagedApplyChange, StagedFileState};
use crate::infrastructure::source_roots::GENERATED_DIR_NAME;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fmt;
use std::fs;
use std::io::{ErrorKind, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
#[cfg(test)]
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

#[cfg(test)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum RetainedScanTestMutationPoint {
    ScanStart,
    AfterDirectoryEnumeration,
    BeforeDirectoryRecursion,
    BeforeFileHash,
    AfterFileHash,
}

#[cfg(test)]
thread_local! {
    static RETAINED_SCAN_TEST_MUTATION: std::cell::RefCell<
        Option<RetainedScanTestMutation>,
    > = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
struct RetainedScanTestMutation {
    point: RetainedScanTestMutationPoint,
    repeat: bool,
    action: Box<dyn FnMut()>,
}

#[cfg(test)]
pub(crate) struct RetainedScanTestMutationGuard;

#[cfg(test)]
impl Drop for RetainedScanTestMutationGuard {
    fn drop(&mut self) {
        RETAINED_SCAN_TEST_MUTATION.with(|slot| slot.borrow_mut().take());
    }
}

#[cfg(test)]
pub(crate) fn set_retained_scan_test_mutation(
    point: RetainedScanTestMutationPoint,
    mutation: impl FnOnce() + 'static,
) -> RetainedScanTestMutationGuard {
    let mut mutation = Some(mutation);
    RETAINED_SCAN_TEST_MUTATION.with(|slot| {
        *slot.borrow_mut() = Some(RetainedScanTestMutation {
            point,
            repeat: false,
            action: Box::new(move || mutation.take().expect("one-shot mutation runs once")()),
        });
    });
    RetainedScanTestMutationGuard
}

#[cfg(test)]
pub(crate) fn set_repeating_retained_scan_test_mutation(
    point: RetainedScanTestMutationPoint,
    mutation: impl FnMut() + 'static,
) -> RetainedScanTestMutationGuard {
    RETAINED_SCAN_TEST_MUTATION.with(|slot| {
        *slot.borrow_mut() = Some(RetainedScanTestMutation {
            point,
            repeat: true,
            action: Box::new(mutation),
        });
    });
    RetainedScanTestMutationGuard
}

#[cfg(test)]
fn run_retained_scan_test_mutation(point: RetainedScanTestMutationPoint) {
    RETAINED_SCAN_TEST_MUTATION.with(|slot| {
        let pending = slot.borrow_mut().take();
        match pending {
            Some(mut pending) if pending.point == point => {
                (pending.action)();
                if pending.repeat {
                    *slot.borrow_mut() = Some(pending);
                }
            }
            Some(pending) => *slot.borrow_mut() = Some(pending),
            None => {}
        }
    });
}

const MAX_SOURCE_DEPTH: usize = 64;
const REVISION_RECORD_SCHEMA_VERSION: u32 = 2;
const RETAINED_HASH_CHUNK_BYTES: usize = 64 * 1024;
const MAX_RETAINED_SOURCE_ENTRIES: usize = 1_000_000;
const MAX_RETAINED_SOURCE_FILE_BYTES: u64 = 256 * 1024 * 1024;
const MAX_RETAINED_SOURCE_TOTAL_BYTES: u64 = 16 * 1024 * 1024 * 1024;

#[cfg(test)]
thread_local! {
    static REVISION_SCAN_LIMITS_OVERRIDE: std::cell::Cell<Option<RetainedScanLimits>> = const {
        std::cell::Cell::new(None)
    };
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WorkspaceStateScope {
    /// v0.12 compatibility namespace: only canonical workspace/source paths
    /// participate in persisted state identity.
    LegacyPhysical,
    /// v0.13 actor namespace: the digest covers the complete structural actor
    /// identity and is bounded to lowercase SHA-256 text.
    Scoped(ScopedStateDigest),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ScopedStateDigest(String);

impl WorkspaceStateScope {
    fn record_value(&self) -> Option<&str> {
        match self {
            Self::LegacyPhysical => None,
            Self::Scoped(digest) => Some(&digest.0),
        }
    }

    pub(crate) fn scoped_sha256(digest: String) -> Result<Self, String> {
        if digest.len() != 64
            || !digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(
                "workspace actor state scope must be a lowercase SHA-256 digest".to_string(),
            );
        }
        Ok(Self::Scoped(ScopedStateDigest(digest)))
    }

    pub(crate) fn scoped_digest(&self) -> Option<&str> {
        self.record_value()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum ManifestEntryKind {
    Directory = 1,
    Content = 2,
    Presence = 3,
}

#[derive(Clone, PartialEq, Eq)]
struct SourceEntryDigest {
    kind: ManifestEntryKind,
    digest: [u8; 32],
    /// Bounded corpus accounting metadata. It deliberately does not
    /// participate in `digest_source_manifest`, so the legacy revision digest
    /// remains byte-for-byte stable while incremental updates can enforce the
    /// same aggregate limit as full captures.
    content_bytes: u64,
}

#[cfg(test)]
#[derive(Default)]
struct ReconcileEverySnapshotFence {
    at_reconcile_boundary: AtomicBool,
}

#[cfg(test)]
impl SourceRevisionFence for ReconcileEverySnapshotFence {
    fn capability(&self) -> FenceCapability {
        FenceCapability::ProvenFast
    }

    fn flush(
        &self,
        _deadline: ProviderDeadline,
        _cancellation: &CancellationToken,
    ) -> Result<FenceOutcome, String> {
        if self.at_reconcile_boundary.fetch_xor(true, Ordering::AcqRel) {
            Ok(FenceOutcome::Proven {
                changed_paths: Vec::new(),
            })
        } else {
            Ok(FenceOutcome::TrustLost(SourceRevisionTrustLoss::WatcherGap))
        }
    }
}

type SourceManifest = BTreeMap<PathBuf, SourceEntryDigest>;

#[derive(Clone, PartialEq, Eq)]
struct RetainedManifestCapture {
    manifest: SourceManifest,
    identities: BTreeMap<PathBuf, (ManifestEntryKind, FileIdentity)>,
    namespace_stable: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ManifestProvenance {
    Ambient(FileIdentity),
    Retained(FileIdentity),
}

#[derive(Debug, Clone, Copy)]
struct RetainedScanLimits {
    max_entries: usize,
    max_file_bytes: u64,
    max_total_bytes: u64,
}

impl RetainedScanLimits {
    const PRODUCTION: Self = Self::new(
        MAX_RETAINED_SOURCE_ENTRIES,
        MAX_RETAINED_SOURCE_FILE_BYTES,
        MAX_RETAINED_SOURCE_TOTAL_BYTES,
    );

    const fn new(max_entries: usize, max_file_bytes: u64, max_total_bytes: u64) -> Self {
        Self {
            max_entries,
            max_file_bytes,
            max_total_bytes,
        }
    }
}

fn revision_scan_limits() -> RetainedScanLimits {
    #[cfg(test)]
    if let Some(limits) = REVISION_SCAN_LIMITS_OVERRIDE.with(std::cell::Cell::get) {
        return limits;
    }
    RetainedScanLimits::PRODUCTION
}

#[cfg(test)]
struct RevisionScanLimitsGuard(Option<RetainedScanLimits>);

#[cfg(test)]
impl Drop for RevisionScanLimitsGuard {
    fn drop(&mut self) {
        REVISION_SCAN_LIMITS_OVERRIDE.with(|slot| slot.set(self.0));
    }
}

#[cfg(test)]
fn set_revision_scan_limits_for_test(limits: RetainedScanLimits) -> RevisionScanLimitsGuard {
    let previous = REVISION_SCAN_LIMITS_OVERRIDE.with(|slot| slot.replace(Some(limits)));
    RevisionScanLimitsGuard(previous)
}

#[derive(Debug)]
struct RetainedScanState {
    entries: usize,
    verification_entries: usize,
    total_bytes: u64,
    namespace_stable: bool,
}

impl Default for RetainedScanState {
    fn default() -> Self {
        Self {
            entries: 0,
            verification_entries: 0,
            total_bytes: 0,
            namespace_stable: true,
        }
    }
}

#[derive(Clone, Copy)]
struct RetainedScanContext<'a> {
    deadline: ProviderDeadline,
    cancellation: &'a CancellationToken,
    limits: RetainedScanLimits,
    artifact_policy: RevisionArtifactPolicy,
}

#[derive(Clone, Copy)]
struct AmbientScanContext<'a> {
    root: &'a Path,
    artifact_policy: RevisionArtifactPolicy,
    limits: RetainedScanLimits,
    should_stop: &'a (dyn Fn() -> bool + Sync),
}

#[derive(Debug, Clone)]
pub(crate) struct RetainedRevisionLease {
    revision: SourceRevision,
    root_identity: FileIdentity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RetainedRevisionErrorKind {
    Cancelled,
    Deadline,
    ConcurrentRevision,
    ContainmentIdentity,
    Provider,
    Invariant,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RetainedRevisionError {
    kind: RetainedRevisionErrorKind,
    message: String,
}

impl RetainedRevisionError {
    fn new(kind: RetainedRevisionErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub(crate) fn kind(&self) -> RetainedRevisionErrorKind {
        self.kind
    }

    fn from_deadline_lock(error: DeadlineLockError) -> Self {
        let kind = match error.kind() {
            DeadlineLockErrorKind::Cancelled => RetainedRevisionErrorKind::Cancelled,
            DeadlineLockErrorKind::Deadline => RetainedRevisionErrorKind::Deadline,
            DeadlineLockErrorKind::Poisoned => RetainedRevisionErrorKind::Invariant,
        };
        Self::new(kind, error.to_string())
    }
}

impl fmt::Display for RetainedRevisionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.message.fmt(formatter)
    }
}

impl std::error::Error for RetainedRevisionError {}

pub(crate) struct PreparedRevisionReconciliation {
    service: Arc<SourceRevisionService>,
    root: Arc<RetainedDirectoryCapability>,
    root_identity: FileIdentity,
    expected_machine: SourceRevisionMachine,
    projected_manifest: SourceManifest,
    candidate: SourceRevision,
    record_path: PathBuf,
    record_bytes: Vec<u8>,
}

pub(crate) struct ActiveRevisionReconciliation<'a> {
    prepared: &'a PreparedRevisionReconciliation,
    _operation: MutexGuard<'a, ()>,
}

impl PreparedRevisionReconciliation {
    pub(crate) fn record_path(&self) -> &Path {
        &self.record_path
    }

    pub(crate) fn record_bytes(&self) -> &[u8] {
        &self.record_bytes
    }

    pub(crate) fn activate(
        &self,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> Result<ActiveRevisionReconciliation<'_>, RetainedRevisionError> {
        let operation = self
            .service
            .operation
            .acquire_before(
                deadline,
                cancellation,
                "prepared revision reconciliation wait",
            )
            .map_err(RetainedRevisionError::from_deadline_lock)?;
        if *self
            .service
            .machine
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            != self.expected_machine
        {
            return Err(RetainedRevisionError::new(
                RetainedRevisionErrorKind::ConcurrentRevision,
                "source revision machine changed after apply planning",
            ));
        }
        Ok(ActiveRevisionReconciliation {
            prepared: self,
            _operation: operation,
        })
    }
}

impl ActiveRevisionReconciliation<'_> {
    pub(crate) fn validate_published_source(
        &self,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> Result<(), RetainedRevisionError> {
        let root = &self.prepared.root;
        if root.identity() != self.prepared.root_identity {
            return Err(RetainedRevisionError::new(
                RetainedRevisionErrorKind::ContainmentIdentity,
                "revision participant belongs to another retained source root",
            ));
        }
        root.validate_named_identity().map_err(|error| {
            RetainedRevisionError::new(
                RetainedRevisionErrorKind::ContainmentIdentity,
                format!("retained source identity changed during revision reconciliation: {error}"),
            )
        })?;
        let first =
            self.prepared
                .service
                .capture_retained_manifest_typed(root, deadline, cancellation)?;
        let second =
            self.prepared
                .service
                .capture_retained_manifest_typed(root, deadline, cancellation)?;
        if !first.namespace_stable
            || !second.namespace_stable
            || first.manifest != second.manifest
            || first.identities != second.identities
            || second.manifest != self.prepared.projected_manifest
        {
            return Err(RetainedRevisionError::new(
                RetainedRevisionErrorKind::ConcurrentRevision,
                "temporarily published source does not match revision candidate",
            ));
        }
        let digest = digest_source_manifest(&second.manifest).map_err(|error| {
            RetainedRevisionError::new(RetainedRevisionErrorKind::Invariant, error)
        })?;
        if digest != self.prepared.candidate.digest {
            return Err(RetainedRevisionError::new(
                RetainedRevisionErrorKind::ConcurrentRevision,
                "temporarily published source digest does not match revision candidate",
            ));
        }
        Ok(())
    }

    pub(crate) fn install(self) -> Result<SourceRevision, RetainedRevisionError> {
        let mut machine = self
            .prepared
            .service
            .machine
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if !machine.install_candidate_if_unchanged(
            &self.prepared.expected_machine,
            self.prepared.candidate.clone(),
        ) {
            return Err(RetainedRevisionError::new(
                RetainedRevisionErrorKind::ConcurrentRevision,
                "source revision trust epoch changed during apply publication",
            ));
        }
        *self
            .prepared
            .service
            .manifest
            .lock()
            .unwrap_or_else(|error| error.into_inner()) =
            Some(self.prepared.projected_manifest.clone());
        *self
            .prepared
            .service
            .manifest_provenance
            .lock()
            .unwrap_or_else(|error| error.into_inner()) =
            Some(ManifestProvenance::Retained(self.prepared.root_identity));
        Ok(self.prepared.candidate.clone())
    }
}

impl RetainedRevisionLease {
    pub(crate) fn revision_identity(&self) -> String {
        format!(
            "{}:{}:{}",
            self.revision.algorithm, self.revision.generation, self.revision.digest
        )
    }
}
type SourceManifestScanner =
    dyn Fn(&Path, &(dyn Fn() -> bool + Sync)) -> Result<SourceManifest, String> + Send + Sync;
type SourceFileReader = dyn Fn(&Path) -> Result<Box<dyn Read + Send>, String> + Send + Sync;

pub(crate) struct SourceRevisionService {
    workspace_root: PathBuf,
    source_root: PathBuf,
    source_root_identity: FileIdentity,
    record_path: PathBuf,
    state_scope: WorkspaceStateScope,
    artifact_policy: RevisionArtifactPolicy,
    machine: Mutex<SourceRevisionMachine>,
    manifest: Mutex<Option<SourceManifest>>,
    manifest_provenance: Mutex<Option<ManifestProvenance>>,
    operation: DeadlineLock<Recover>,
    fence: Arc<dyn SourceRevisionFence>,
    scanner: Arc<SourceManifestScanner>,
    file_reader: Arc<SourceFileReader>,
    #[cfg(test)]
    retained_scans: AtomicUsize,
}

impl fmt::Debug for SourceRevisionService {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SourceRevisionService")
            .field("workspace_root", &self.workspace_root)
            .field("source_root", &self.source_root)
            .field("record_path", &self.record_path)
            .field("state_scope", &self.state_scope)
            .field("fence", &self.fence.capability())
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SourceRevisionRecord {
    schema_version: u32,
    workspace_root: String,
    source_root: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    state_scope: Option<String>,
    revision: SourceRevision,
}

impl SourceRevisionService {
    pub(crate) fn new(context: &WorkspaceContext, source_root: &Path) -> Result<Self, String> {
        let canonical_source_root = fs::canonicalize(source_root)
            .map_err(|error| format!("source revision root cannot be normalized: {error}"))?;
        let fence = platform_fence(&canonical_source_root, &context.cache_root)?;
        Self::with_fence(
            context,
            &canonical_source_root,
            WorkspaceStateScope::LegacyPhysical,
            RevisionArtifactPolicy::legacy_v12(),
            fence,
        )
    }

    pub(super) fn new_actor(
        context: &WorkspaceContext,
        authority: ActorRevisionServiceAuthority,
    ) -> Result<Self, String> {
        let artifact_policy = RevisionArtifactPolicy::from_authenticated_actor(&authority)?;
        let state_scope = authority.state_scope().clone();
        let source_root = authority.source_root();
        let canonical_source_root = fs::canonicalize(source_root)
            .map_err(|error| format!("source revision root cannot be normalized: {error}"))?;
        let fence = deferred_platform_fence(&canonical_source_root, &context.cache_root);
        Self::with_fence(
            context,
            &canonical_source_root,
            state_scope,
            artifact_policy,
            fence,
        )
    }

    #[cfg(test)]
    pub(crate) fn new_reconciling_for_test(
        context: &WorkspaceContext,
        source_root: &Path,
    ) -> Result<Self, String> {
        Self::with_fence(
            context,
            source_root,
            WorkspaceStateScope::LegacyPhysical,
            RevisionArtifactPolicy::legacy_v12(),
            Arc::new(ReconcileEverySnapshotFence::default()),
        )
    }

    #[cfg(test)]
    pub(crate) fn new_with_fence_for_test(
        context: &WorkspaceContext,
        source_root: &Path,
        state_scope: WorkspaceStateScope,
        fence: Arc<dyn SourceRevisionFence>,
    ) -> Result<Self, String> {
        Self::with_fence(
            context,
            source_root,
            state_scope,
            RevisionArtifactPolicy::legacy_v12(),
            fence,
        )
    }

    fn with_fence(
        context: &WorkspaceContext,
        source_root: &Path,
        state_scope: WorkspaceStateScope,
        artifact_policy: RevisionArtifactPolicy,
        fence: Arc<dyn SourceRevisionFence>,
    ) -> Result<Self, String> {
        let workspace_root = fs::canonicalize(&context.workspace_root)
            .map_err(|error| format!("workspace revision root cannot be normalized: {error}"))?;
        let source_root = fs::canonicalize(source_root)
            .map_err(|error| format!("source revision root cannot be normalized: {error}"))?;
        let source_root_identity = RetainedDirectoryCapability::open(&source_root)
            .map_err(|error| format!("source revision root cannot be retained: {error}"))?
            .identity();
        let mut identity = Sha256::new();
        if let WorkspaceStateScope::Scoped(digest) = &state_scope {
            identity.update(b"unica-source-revision-state-v1\0");
            identity.update((digest.0.len() as u64).to_le_bytes());
            identity.update(digest.0.as_bytes());
        }
        update_identity_path(&mut identity, &workspace_root);
        identity.update([0]);
        update_identity_path(&mut identity, &source_root);
        let identity = format!("{:x}", identity.finalize());
        let record_path = context
            .cache_root
            .join("source-revisions")
            .join(format!("{identity}.json"));
        let machine = load_revision_record(
            &record_path,
            &workspace_root,
            &source_root,
            state_scope.record_value(),
        )
        .and_then(|revision| SourceRevisionMachine::from_revision(revision).ok())
        .unwrap_or_default();
        Ok(Self {
            workspace_root,
            source_root,
            source_root_identity,
            record_path,
            state_scope,
            artifact_policy,
            machine: Mutex::new(machine),
            manifest: Mutex::new(None),
            manifest_provenance: Mutex::new(None),
            operation: DeadlineLock::default(),
            fence,
            scanner: Arc::new(move |root, should_stop| {
                scan_source_manifest_with_policy(root, artifact_policy, should_stop)
            }),
            file_reader: Arc::new(read_source_file),
            #[cfg(test)]
            retained_scans: AtomicUsize::new(0),
        })
    }

    pub(crate) fn snapshot(
        &self,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> Result<SourceRevision, String> {
        let _operation = self
            .operation
            .acquire_before(deadline, cancellation, "source revision operation wait")
            .map_err(|error| error.to_string())?;
        let ambient_identity = self.ambient_root_identity()?;
        if self.fence.capability() == FenceCapability::Unsupported {
            self.machine
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .lose_trust(SourceRevisionTrustLoss::UnsupportedFence);
            return Err(
                "source revision fence is unsupported; freshness cannot be proven".to_string(),
            );
        }
        let fence_outcome = self.fence.flush(deadline, cancellation).inspect_err(|_| {
            self.machine
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .lose_trust(SourceRevisionTrustLoss::ReconcileFailed);
        })?;
        let needs_reconcile = match fence_outcome {
            FenceOutcome::Proven { changed_paths } => {
                let (trusted, trust_loss_epoch) = {
                    let machine = self
                        .machine
                        .lock()
                        .unwrap_or_else(|error| error.into_inner());
                    (
                        matches!(machine.state(), SourceRevisionState::Trusted(_)),
                        machine.trust_loss_epoch(),
                    )
                };
                let ambient_manifest_matches = self
                    .manifest_provenance
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .is_some_and(|provenance| {
                        provenance == ManifestProvenance::Ambient(ambient_identity)
                    });
                if changed_paths.is_empty() {
                    !(trusted && ambient_manifest_matches)
                } else {
                    !(trusted
                        && ambient_manifest_matches
                        && self.apply_incremental(
                            changed_paths,
                            trust_loss_epoch,
                            ambient_identity,
                            deadline,
                            cancellation,
                        )?)
                }
            }
            FenceOutcome::TrustLost(reason) => {
                self.machine
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .lose_trust(reason);
                true
            }
        };
        if needs_reconcile {
            self.reconcile(ambient_identity, deadline, cancellation)?;
        }
        if self.ambient_root_identity()? != ambient_identity {
            self.lose_incremental_trust();
            return Err("ambient source revision identity changed during snapshot".to_string());
        }
        self.trusted_snapshot()
    }

    fn ambient_root_identity(&self) -> Result<FileIdentity, String> {
        RetainedDirectoryCapability::open(&self.source_root)
            .map(|root| root.identity())
            .map_err(|error| format!("ambient source revision root cannot be retained: {error}"))
    }

    /// Computes and publishes a revision from the same descriptor-relative
    /// authority used by the V13 reader. No lexical source path or watcher
    /// snapshot participates in this operation.
    pub(crate) fn snapshot_retained(
        &self,
        root: &RetainedDirectoryCapability,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> Result<SourceRevision, String> {
        let _operation = self
            .operation
            .acquire_before(
                deadline,
                cancellation,
                "retained source revision operation wait",
            )
            .map_err(|error| error.to_string())?;
        if root.identity() != self.source_root_identity {
            return Err(
                "retained source revision capability has a different actor identity".to_string(),
            );
        }
        root.validate_named_identity().map_err(|error| {
            format!("retained source revision identity changed after admission: {error}")
        })?;
        let needs_reconcile = if self.fence.capability() == FenceCapability::Unsupported {
            true
        } else {
            match self.fence.flush(deadline, cancellation).inspect_err(|_| {
                self.machine
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .lose_trust(SourceRevisionTrustLoss::ReconcileFailed);
            })? {
                FenceOutcome::Proven { changed_paths } if changed_paths.is_empty() => {
                    let trusted = matches!(
                        self.machine
                            .lock()
                            .unwrap_or_else(|error| error.into_inner())
                            .state(),
                        SourceRevisionState::Trusted(_)
                    );
                    let retained_manifest_matches = self
                        .manifest_provenance
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .is_some_and(|provenance| {
                            provenance == ManifestProvenance::Retained(root.identity())
                        });
                    !(trusted && retained_manifest_matches)
                }
                FenceOutcome::Proven { .. } => {
                    self.machine
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .lose_trust(SourceRevisionTrustLoss::WatcherGap);
                    true
                }
                FenceOutcome::TrustLost(reason) => {
                    self.machine
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .lose_trust(reason);
                    true
                }
            }
        };
        if !needs_reconcile {
            return self.trusted_snapshot();
        }
        self.reconcile_retained(root, deadline, cancellation)?;
        self.trusted_snapshot()
    }

    pub(crate) fn begin_retained_operation(
        &self,
        root: &RetainedDirectoryCapability,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> Result<RetainedRevisionLease, String> {
        let revision = self.snapshot_retained(root, deadline, cancellation)?;
        Ok(RetainedRevisionLease {
            revision,
            root_identity: root.identity(),
        })
    }

    /// Observes one exact retained revision without flushing the platform
    /// fence, persisting a record or advancing the in-memory revision machine.
    /// Two equal retained captures bind both bytes and namespace identity.
    pub(crate) fn observe_retained_operation(
        &self,
        root: &RetainedDirectoryCapability,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> Result<RetainedRevisionLease, String> {
        self.observe_retained_operation_typed(root, deadline, cancellation)
            .map_err(|error| error.to_string())
    }

    fn observe_retained_operation_typed(
        &self,
        root: &RetainedDirectoryCapability,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> Result<RetainedRevisionLease, RetainedRevisionError> {
        let _operation = self
            .operation
            .acquire_before(
                deadline,
                cancellation,
                "retained source revision observation wait",
            )
            .map_err(RetainedRevisionError::from_deadline_lock)?;
        if root.identity() != self.source_root_identity {
            return Err(RetainedRevisionError::new(
                RetainedRevisionErrorKind::ContainmentIdentity,
                "retained source revision capability has a different actor identity",
            ));
        }
        root.validate_named_identity().map_err(|error| {
            RetainedRevisionError::new(
                RetainedRevisionErrorKind::ContainmentIdentity,
                format!("retained source revision identity changed during observation: {error}"),
            )
        })?;
        let first = self.capture_retained_manifest_typed(root, deadline, cancellation)?;
        let second = self.capture_retained_manifest_typed(root, deadline, cancellation)?;
        if !first.namespace_stable
            || !second.namespace_stable
            || first.manifest != second.manifest
            || first.identities != second.identities
        {
            return Err(RetainedRevisionError::new(
                RetainedRevisionErrorKind::ConcurrentRevision,
                "retained source revision did not stabilize during observation",
            ));
        }
        let digest = digest_source_manifest(&second.manifest).map_err(|error| {
            RetainedRevisionError::new(RetainedRevisionErrorKind::Invariant, error)
        })?;
        let revision = self
            .machine
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .candidate_for_digest(digest)
            .map_err(|error| {
                RetainedRevisionError::new(RetainedRevisionErrorKind::Invariant, error)
            })?;
        Ok(RetainedRevisionLease {
            revision,
            root_identity: root.identity(),
        })
    }

    pub(crate) fn confirm_retained_observation_typed(
        &self,
        root: &RetainedDirectoryCapability,
        lease: &RetainedRevisionLease,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> Result<(), RetainedRevisionError> {
        if root.identity() != lease.root_identity {
            return Err(RetainedRevisionError::new(
                RetainedRevisionErrorKind::ContainmentIdentity,
                "retained revision lease belongs to another source identity",
            ));
        }
        let current = self.observe_retained_operation_typed(root, deadline, cancellation)?;
        if current.revision != lease.revision {
            return Err(RetainedRevisionError::new(
                RetainedRevisionErrorKind::ConcurrentRevision,
                "retained source revision changed during logical operation",
            ));
        }
        Ok(())
    }

    pub(crate) fn prepare_retained_apply_reconciliation(
        self: &Arc<Self>,
        root: &Arc<RetainedDirectoryCapability>,
        lease: &RetainedRevisionLease,
        changes: &[StagedApplyChange],
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> Result<PreparedRevisionReconciliation, RetainedRevisionError> {
        let _operation = self
            .operation
            .acquire_before(deadline, cancellation, "prepared revision planning wait")
            .map_err(RetainedRevisionError::from_deadline_lock)?;
        if root.identity() != self.source_root_identity || root.identity() != lease.root_identity {
            return Err(RetainedRevisionError::new(
                RetainedRevisionErrorKind::ContainmentIdentity,
                "revision planning belongs to another retained source root",
            ));
        }
        let first = self.capture_retained_manifest_typed(root, deadline, cancellation)?;
        let second = self.capture_retained_manifest_typed(root, deadline, cancellation)?;
        if !first.namespace_stable
            || !second.namespace_stable
            || first.manifest != second.manifest
            || first.identities != second.identities
        {
            return Err(RetainedRevisionError::new(
                RetainedRevisionErrorKind::ConcurrentRevision,
                "retained source revision did not stabilize during apply planning",
            ));
        }
        let expected_machine = self
            .machine
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        let observed = expected_machine
            .candidate_for_digest(digest_source_manifest(&second.manifest).map_err(|error| {
                RetainedRevisionError::new(RetainedRevisionErrorKind::Invariant, error)
            })?)
            .map_err(|error| {
                RetainedRevisionError::new(RetainedRevisionErrorKind::Invariant, error)
            })?;
        if observed != lease.revision {
            return Err(RetainedRevisionError::new(
                RetainedRevisionErrorKind::ConcurrentRevision,
                "retained source revision changed during apply planning",
            ));
        }
        let mut projected_manifest = second.manifest;
        for change in changes {
            if self.artifact_policy.classify(&change.relative_path)
                != RevisionArtifactDisposition::Content
            {
                return Err(RetainedRevisionError::new(
                    RetainedRevisionErrorKind::Invariant,
                    format!(
                        "staged source path is outside the closed revision artifact policy: {}",
                        change.relative_path.display()
                    ),
                ));
            }
            match &change.current {
                StagedFileState::Absent => {
                    projected_manifest.remove(&change.relative_path);
                }
                StagedFileState::Bytes(bytes) => {
                    let mut digest = Sha256::new();
                    digest.update(bytes);
                    projected_manifest.insert(
                        change.relative_path.clone(),
                        SourceEntryDigest {
                            kind: ManifestEntryKind::Content,
                            digest: digest.finalize().into(),
                            content_bytes: u64::try_from(bytes.len())
                                .expect("staged source content length fits u64"),
                        },
                    );
                    let mut parent = change.relative_path.parent();
                    while let Some(relative) = parent {
                        if relative.as_os_str().is_empty() {
                            break;
                        }
                        projected_manifest.entry(relative.to_path_buf()).or_insert(
                            SourceEntryDigest {
                                kind: ManifestEntryKind::Directory,
                                digest: [0; 32],
                                content_bytes: 0,
                            },
                        );
                        parent = relative.parent();
                    }
                }
            }
        }
        let candidate =
            expected_machine
                .candidate_for_digest(digest_source_manifest(&projected_manifest).map_err(
                    |error| RetainedRevisionError::new(RetainedRevisionErrorKind::Invariant, error),
                )?)
                .map_err(|error| {
                    RetainedRevisionError::new(RetainedRevisionErrorKind::Invariant, error)
                })?;
        let record_bytes = revision_record_bytes(
            &self.workspace_root,
            &self.source_root,
            self.state_scope.record_value(),
            &candidate,
        )
        .map_err(|error| RetainedRevisionError::new(RetainedRevisionErrorKind::Invariant, error))?;
        Ok(PreparedRevisionReconciliation {
            service: Arc::clone(self),
            root: Arc::clone(root),
            root_identity: root.identity(),
            expected_machine,
            projected_manifest,
            candidate,
            record_path: self.record_path.clone(),
            record_bytes,
        })
    }

    pub(crate) fn confirm_retained_operation(
        &self,
        root: &RetainedDirectoryCapability,
        lease: &RetainedRevisionLease,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> Result<(), String> {
        if root.identity() != lease.root_identity {
            return Err("retained revision lease belongs to another source identity".to_string());
        }
        let current = self.snapshot_retained(root, deadline, cancellation)?;
        if current != lease.revision {
            return Err("retained source revision changed during logical operation".to_string());
        }
        Ok(())
    }

    fn reconcile_retained(
        &self,
        root: &RetainedDirectoryCapability,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> Result<(), String> {
        for _ in 0..3 {
            let trust_loss_epoch = {
                let mut machine = self
                    .machine
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                let trust_loss_epoch = machine.trust_loss_epoch();
                machine.begin_reconcile();
                trust_loss_epoch
            };
            let first = self
                .capture_retained_manifest(root, deadline, cancellation)
                .inspect_err(|_| self.lose_incremental_trust())?;
            if !first.namespace_stable {
                continue;
            }
            let capture = if self.fence.capability() == FenceCapability::Unsupported {
                let second = self
                    .capture_retained_manifest(root, deadline, cancellation)
                    .inspect_err(|_| self.lose_incremental_trust())?;
                if !second.namespace_stable
                    || first.manifest != second.manifest
                    || first.identities != second.identities
                {
                    continue;
                }
                second
            } else {
                first
            };
            if self.fence.capability() != FenceCapability::Unsupported {
                match self.fence.flush(deadline, cancellation).inspect_err(|_| {
                    self.machine
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .lose_trust(SourceRevisionTrustLoss::ReconcileFailed);
                })? {
                    FenceOutcome::Proven { changed_paths } if changed_paths.is_empty() => {}
                    FenceOutcome::Proven { .. } => continue,
                    FenceOutcome::TrustLost(reason) => {
                        self.machine
                            .lock()
                            .unwrap_or_else(|error| error.into_inner())
                            .lose_trust(reason);
                        continue;
                    }
                }
            }
            let digest = digest_source_manifest(&capture.manifest)?;
            if self.publish_revision_with_authority(
                &capture.manifest,
                digest,
                trust_loss_epoch,
                ManifestProvenance::Retained(root.identity()),
            )? {
                return Ok(());
            }
        }
        Err("retained source revision did not stabilize during reconcile".to_string())
    }

    fn capture_retained_manifest(
        &self,
        root: &RetainedDirectoryCapability,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> Result<RetainedManifestCapture, String> {
        self.capture_retained_manifest_typed(root, deadline, cancellation)
            .map_err(|error| error.to_string())
    }

    fn capture_retained_manifest_typed(
        &self,
        root: &RetainedDirectoryCapability,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> Result<RetainedManifestCapture, RetainedRevisionError> {
        #[cfg(test)]
        self.retained_scans.fetch_add(1, Ordering::Relaxed);
        capture_retained_source_manifest_with_limits(
            root,
            deadline,
            cancellation,
            revision_scan_limits(),
            self.artifact_policy,
        )
    }

    #[cfg(test)]
    pub(crate) fn retained_scan_count(&self) -> usize {
        self.retained_scans.load(Ordering::Relaxed)
    }

    #[cfg(test)]
    pub(crate) fn fence_capability_for_test(&self) -> FenceCapability {
        self.fence.capability()
    }

    #[cfg(test)]
    pub(crate) fn machine_state_for_test(&self) -> SourceRevisionMachine {
        self.machine
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    fn trusted_snapshot(&self) -> Result<SourceRevision, String> {
        let machine = self
            .machine
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        match machine.state() {
            SourceRevisionState::Trusted(revision) => Ok(revision.clone()),
            _ => Err("source revision remains untrusted after reconcile".to_string()),
        }
    }

    pub(crate) fn mark_dirty(&self) {
        self.machine
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .lose_trust(SourceRevisionTrustLoss::WatcherGap);
    }

    #[cfg(test)]
    pub(crate) fn hold_operation_for_test(&self) -> std::sync::MutexGuard<'_, ()> {
        self.operation.hold_for_test()
    }

    fn apply_incremental(
        &self,
        mut changed_paths: Vec<PathBuf>,
        expected_trust_loss_epoch: u64,
        ambient_identity: FileIdentity,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> Result<bool, String> {
        let Some(mut manifest) = self
            .manifest
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
        else {
            return Ok(false);
        };
        let mut total_bytes = manifest.values().try_fold(0_u64, |total, entry| {
            total
                .checked_add(entry.content_bytes)
                .ok_or_else(|| "source revision aggregate byte count overflowed".to_string())
        })?;
        for _ in 0..3 {
            {
                let mut machine = self
                    .machine
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                if machine.trust_loss_epoch() != expected_trust_loss_epoch {
                    return Ok(false);
                }
                machine.begin_reconcile();
            }
            for relative_path in changed_paths {
                if cancellation.is_cancelled() || deadline.remaining().is_zero() {
                    self.machine
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .lose_trust(SourceRevisionTrustLoss::ReconcileFailed);
                    return Err("source revision reconcile cancelled".to_string());
                }
                if !self.apply_changed_path(
                    &mut manifest,
                    &relative_path,
                    revision_scan_limits(),
                    &mut total_bytes,
                    deadline,
                    cancellation,
                )? {
                    return Ok(false);
                }
            }
            match self.fence.flush(deadline, cancellation).inspect_err(|_| {
                self.machine
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .lose_trust(SourceRevisionTrustLoss::ReconcileFailed);
            })? {
                FenceOutcome::Proven {
                    changed_paths: additional,
                } if !additional.is_empty() => {
                    changed_paths = additional;
                }
                FenceOutcome::Proven { .. } => {
                    let digest = digest_source_manifest(&manifest)?;
                    return self.publish_revision(
                        &manifest,
                        digest,
                        expected_trust_loss_epoch,
                        ambient_identity,
                    );
                }
                FenceOutcome::TrustLost(reason) => {
                    self.machine
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .lose_trust(reason);
                    return Ok(false);
                }
            }
        }
        self.machine
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .lose_trust(SourceRevisionTrustLoss::ReconcileFailed);
        Ok(false)
    }

    fn apply_changed_path(
        &self,
        manifest: &mut SourceManifest,
        relative_path: &Path,
        limits: RetainedScanLimits,
        total_bytes: &mut u64,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> Result<bool, String> {
        retained_revision_checkpoint(deadline, cancellation).map_err(|error| error.to_string())?;
        if relative_path.is_absolute()
            || relative_path.components().any(|component| {
                matches!(
                    component,
                    std::path::Component::ParentDir
                        | std::path::Component::RootDir
                        | std::path::Component::Prefix(_)
                )
            })
        {
            self.lose_incremental_trust();
            return Err("source revision event escaped its root".to_string());
        }
        // The corpus scan prunes the generated directory; an incremental
        // event under it must stay just as invisible, or the incremental
        // digest would diverge from the digest a full scan produces.
        for component in relative_path.components() {
            if host_directory_component_names_equivalent(
                &self.source_root,
                component.as_os_str(),
                OsStr::new(GENERATED_DIR_NAME),
            )
            .map_err(|error| {
                format!("source revision component identity cannot be proven: {error}")
            })? {
                remove_manifest_entry(manifest, relative_path, total_bytes)?;
                return Ok(true);
            }
        }
        let path = self.source_root.join(relative_path);
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == ErrorKind::NotFound => {
                remove_manifest_entry(manifest, relative_path, total_bytes)?;
                return Ok(true);
            }
            Err(error) => {
                self.lose_incremental_trust();
                return Err(format!("source revision file cannot be inspected: {error}"));
            }
        };
        if metadata.file_type().is_symlink() {
            if self.artifact_policy.classify(relative_path) != RevisionArtifactDisposition::Ignored
            {
                self.lose_incremental_trust();
                return Err(format!(
                    "source revision corpus contains an indexed symbolic link: {}",
                    relative_path.display()
                ));
            }
            remove_manifest_entry(manifest, relative_path, total_bytes)?;
            return Ok(true);
        }
        if metadata.is_dir() {
            return Ok(false);
        }
        if !metadata.is_file()
            || self.artifact_policy.classify(relative_path) == RevisionArtifactDisposition::Ignored
        {
            remove_manifest_entry(manifest, relative_path, total_bytes)?;
            return Ok(true);
        }
        let canonical = fs::canonicalize(&path)
            .map_err(|error| format!("source revision file cannot be normalized: {error}"))?;
        if canonical != path {
            return Ok(false);
        }
        match self.artifact_policy.classify(relative_path) {
            RevisionArtifactDisposition::Content => {
                let previous_bytes = manifest
                    .get(relative_path)
                    .map_or(0, |entry| entry.content_bytes);
                *total_bytes = total_bytes.checked_sub(previous_bytes).ok_or_else(|| {
                    "source revision aggregate byte accounting underflowed".to_string()
                })?;
                let mut reader =
                    (self.file_reader)(&path).inspect_err(|_| self.lose_incremental_trust())?;
                retained_revision_checkpoint(deadline, cancellation)
                    .inspect_err(|_| self.lose_incremental_trust())
                    .map_err(|error| error.to_string())?;
                let (digest, content_bytes) = hash_bounded_source_reader(
                    reader.as_mut(),
                    relative_path,
                    limits,
                    total_bytes,
                    &mut || retained_revision_checkpoint(deadline, cancellation),
                )
                .inspect_err(|_| self.lose_incremental_trust())
                .map_err(|error| error.to_string())?;
                manifest.insert(
                    relative_path.to_path_buf(),
                    SourceEntryDigest {
                        kind: ManifestEntryKind::Content,
                        digest,
                        content_bytes,
                    },
                );
            }
            RevisionArtifactDisposition::Presence => {
                remove_manifest_entry(manifest, relative_path, total_bytes)?;
                manifest.insert(
                    relative_path.to_path_buf(),
                    SourceEntryDigest {
                        kind: ManifestEntryKind::Presence,
                        digest: [0; 32],
                        content_bytes: 0,
                    },
                );
            }
            RevisionArtifactDisposition::Ignored => unreachable!("ignored path returned above"),
        }
        Ok(true)
    }

    fn lose_incremental_trust(&self) {
        self.machine
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .lose_trust(SourceRevisionTrustLoss::ReconcileFailed);
    }

    fn reconcile(
        &self,
        ambient_identity: FileIdentity,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> Result<(), String> {
        for _ in 0..3 {
            let trust_loss_epoch = {
                let mut machine = self
                    .machine
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                let trust_loss_epoch = machine.trust_loss_epoch();
                machine.begin_reconcile();
                trust_loss_epoch
            };
            let manifest = (self.scanner)(&self.source_root, &|| {
                cancellation.is_cancelled() || deadline.remaining().is_zero()
            })
            .inspect_err(|_| {
                self.machine
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .lose_trust(SourceRevisionTrustLoss::ReconcileFailed);
            })?;
            if self.ambient_root_identity()? != ambient_identity {
                self.lose_incremental_trust();
                return Err("ambient source revision identity changed during reconcile".to_string());
            }
            let fence_outcome = self.fence.flush(deadline, cancellation).inspect_err(|_| {
                self.machine
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .lose_trust(SourceRevisionTrustLoss::ReconcileFailed);
            })?;
            match fence_outcome {
                FenceOutcome::Proven { changed_paths } if !changed_paths.is_empty() => continue,
                FenceOutcome::TrustLost(reason) => {
                    self.machine
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .lose_trust(reason);
                    continue;
                }
                FenceOutcome::Proven { .. } => {}
            }
            let digest = digest_source_manifest(&manifest)?;
            if self.publish_revision(&manifest, digest, trust_loss_epoch, ambient_identity)? {
                return Ok(());
            }
        }
        Err("source revision did not stabilize during reconcile".to_string())
    }

    fn publish_revision(
        &self,
        manifest: &SourceManifest,
        digest: String,
        trust_loss_epoch: u64,
        ambient_identity: FileIdentity,
    ) -> Result<bool, String> {
        self.publish_revision_with_authority(
            manifest,
            digest,
            trust_loss_epoch,
            ManifestProvenance::Ambient(ambient_identity),
        )
    }

    fn publish_revision_with_authority(
        &self,
        manifest: &SourceManifest,
        digest: String,
        trust_loss_epoch: u64,
        provenance: ManifestProvenance,
    ) -> Result<bool, String> {
        let Some(revision) = self
            .machine
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .finish_reconcile_if_trust_unchanged(digest, trust_loss_epoch)?
        else {
            return Ok(false);
        };
        persist_revision_record(
            &self.record_path,
            &self.workspace_root,
            &self.source_root,
            self.state_scope.record_value(),
            &revision,
        )
        .inspect_err(|_| {
            self.machine
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .lose_trust(SourceRevisionTrustLoss::ReconcileFailed);
        })?;
        *self
            .manifest
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(manifest.clone());
        *self
            .manifest_provenance
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(provenance);
        Ok(true)
    }
}

fn update_identity_path(identity: &mut Sha256, path: &Path) {
    let bytes = path.as_os_str().as_encoded_bytes();
    identity.update((bytes.len() as u64).to_le_bytes());
    identity.update(bytes);
}

fn remove_manifest_entry(
    manifest: &mut SourceManifest,
    relative_path: &Path,
    total_bytes: &mut u64,
) -> Result<(), String> {
    let removed_bytes = manifest
        .remove(relative_path)
        .map_or(0, |entry| entry.content_bytes);
    *total_bytes = total_bytes
        .checked_sub(removed_bytes)
        .ok_or_else(|| "source revision aggregate byte accounting underflowed".to_string())?;
    Ok(())
}

fn load_revision_record(
    path: &Path,
    workspace_root: &Path,
    source_root: &Path,
    state_scope: Option<&str>,
) -> Option<SourceRevision> {
    let record: SourceRevisionRecord = serde_json::from_slice(&fs::read(path).ok()?).ok()?;
    (record.schema_version == REVISION_RECORD_SCHEMA_VERSION
        && Path::new(&record.workspace_root) == workspace_root
        && Path::new(&record.source_root) == source_root
        && record.state_scope.as_deref() == state_scope)
        .then_some(record.revision)
}

fn persist_revision_record(
    path: &Path,
    workspace_root: &Path,
    source_root: &Path,
    state_scope: Option<&str>,
    revision: &SourceRevision,
) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "source revision record has no parent".to_string())?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("source revision record directory cannot be created: {error}"))?;
    let bytes = revision_record_bytes(workspace_root, source_root, state_scope, revision)?;
    let temporary = path.with_extension(format!("tmp-{}", uuid::Uuid::new_v4()));
    fs::write(&temporary, bytes)
        .and_then(|_| fs::rename(&temporary, path))
        .map_err(|error| format!("source revision record cannot be published: {error}"))
}

fn revision_record_bytes(
    workspace_root: &Path,
    source_root: &Path,
    state_scope: Option<&str>,
    revision: &SourceRevision,
) -> Result<Vec<u8>, String> {
    let record = SourceRevisionRecord {
        schema_version: REVISION_RECORD_SCHEMA_VERSION,
        workspace_root: workspace_root.to_string_lossy().into_owned(),
        source_root: source_root.to_string_lossy().into_owned(),
        state_scope: state_scope.map(str::to_string),
        revision: revision.clone(),
    };
    serde_json::to_vec(&record)
        .map_err(|error| format!("source revision record cannot be serialized: {error}"))
}

#[cfg(test)]
pub(crate) fn seed_observed_revision_record_for_test(
    service: &SourceRevisionService,
    root: &RetainedDirectoryCapability,
) -> Result<String, String> {
    let capture = capture_retained_source_manifest_with_limits(
        root,
        ProviderDeadline::from_budget(std::time::Duration::from_secs(5)),
        &CancellationToken::new(),
        RetainedScanLimits::PRODUCTION,
        RevisionArtifactPolicy::legacy_v12(),
    )
    .map_err(|error| error.to_string())?;
    let revision = service
        .machine
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .candidate_for_digest(digest_source_manifest(&capture.manifest)?)?;
    persist_revision_record(
        &service.record_path,
        &service.workspace_root,
        &service.source_root,
        service.state_scope.record_value(),
        &revision,
    )?;
    Ok(format!(
        "{}:{}:{}",
        revision.algorithm, revision.generation, revision.digest
    ))
}

#[cfg(test)]
fn scan_retained_source_manifest_with_limits(
    root: &RetainedDirectoryCapability,
    deadline: ProviderDeadline,
    cancellation: &CancellationToken,
    limits: RetainedScanLimits,
) -> Result<SourceManifest, String> {
    capture_retained_source_manifest_with_limits(
        root,
        deadline,
        cancellation,
        limits,
        RevisionArtifactPolicy::legacy_v12(),
    )
    .map_err(|error| error.to_string())
    .map(|capture| capture.manifest)
}

fn capture_retained_source_manifest_with_limits(
    root: &RetainedDirectoryCapability,
    deadline: ProviderDeadline,
    cancellation: &CancellationToken,
    limits: RetainedScanLimits,
    artifact_policy: RevisionArtifactPolicy,
) -> Result<RetainedManifestCapture, RetainedRevisionError> {
    retained_revision_checkpoint(deadline, cancellation)?;
    let root_stable_before = root.validate_named_identity().is_ok();
    #[cfg(test)]
    run_retained_scan_test_mutation(RetainedScanTestMutationPoint::ScanStart);
    let mut manifest = SourceManifest::new();
    let mut identities = BTreeMap::new();
    let mut state = RetainedScanState::default();
    let context = RetainedScanContext {
        deadline,
        cancellation,
        limits,
        artifact_policy,
    };
    scan_retained_directory(
        root,
        Path::new(""),
        0,
        context,
        &mut state,
        &mut manifest,
        &mut identities,
    )?;
    retained_revision_checkpoint(deadline, cancellation)?;
    let root_stable_after = root.validate_named_identity().is_ok();
    Ok(RetainedManifestCapture {
        manifest,
        identities,
        namespace_stable: root_stable_before && root_stable_after && state.namespace_stable,
    })
}

fn scan_retained_directory(
    directory: &RetainedDirectoryCapability,
    relative_directory: &Path,
    depth: usize,
    context: RetainedScanContext<'_>,
    state: &mut RetainedScanState,
    manifest: &mut SourceManifest,
    identities: &mut BTreeMap<PathBuf, (ManifestEntryKind, FileIdentity)>,
) -> Result<(), RetainedRevisionError> {
    retained_revision_checkpoint(context.deadline, context.cancellation)?;
    if depth > MAX_SOURCE_DEPTH {
        return Err(RetainedRevisionError::new(
            RetainedRevisionErrorKind::Provider,
            "source revision corpus exceeds maximum depth",
        ));
    }
    let remaining_entries = context.limits.max_entries.saturating_sub(state.entries);
    let names = directory
        .read_immediate_names_bounded(remaining_entries, || {
            retained_revision_checkpoint(context.deadline, context.cancellation)
                .map_err(std::io::Error::other)
        })
        .map_err(|error| {
            if context.cancellation.is_cancelled() {
                RetainedRevisionError::new(
                    RetainedRevisionErrorKind::Cancelled,
                    "retained source revision reconcile cancelled",
                )
            } else if context.deadline.remaining().is_zero() {
                RetainedRevisionError::new(
                    RetainedRevisionErrorKind::Deadline,
                    "retained source revision deadline exceeded",
                )
            } else if error.kind() == ErrorKind::InvalidData {
                RetainedRevisionError::new(
                    RetainedRevisionErrorKind::Provider,
                    format!(
                        "retained source revision entry limit {} exceeded",
                        context.limits.max_entries
                    ),
                )
            } else {
                RetainedRevisionError::new(
                    RetainedRevisionErrorKind::Provider,
                    format!("retained source revision directory cannot be read: {error}"),
                )
            }
        })?;
    #[cfg(test)]
    run_retained_scan_test_mutation(RetainedScanTestMutationPoint::AfterDirectoryEnumeration);
    let child_name_comparator = directory.child_name_comparator().map_err(|error| {
        RetainedRevisionError::new(
            RetainedRevisionErrorKind::Provider,
            format!("retained child-name policy cannot be proven: {error}"),
        )
    })?;
    for name in &names {
        retained_revision_checkpoint(context.deadline, context.cancellation)?;
        state.entries = state
            .entries
            .checked_add(1)
            .filter(|entries| *entries <= context.limits.max_entries)
            .ok_or_else(|| {
                RetainedRevisionError::new(
                    RetainedRevisionErrorKind::Provider,
                    format!(
                        "retained source revision entry limit {} exceeded",
                        context.limits.max_entries
                    ),
                )
            })?;
        if child_name_comparator
            .names_equivalent(name, OsStr::new(GENERATED_DIR_NAME))
            .map_err(|error| {
                RetainedRevisionError::new(
                    RetainedRevisionErrorKind::Provider,
                    format!("retained generated-directory identity cannot be proven: {error}"),
                )
            })?
        {
            continue;
        }
        let relative = relative_directory.join(name);
        let child = match directory.retain_immediate_child_nofollow(name) {
            Ok(child) => child,
            Err(error) if is_nofollow_link_error(&error) => {
                if context.artifact_policy.classify(&relative)
                    != RevisionArtifactDisposition::Ignored
                {
                    return Err(RetainedRevisionError::new(
                        RetainedRevisionErrorKind::ContainmentIdentity,
                        format!(
                            "retained source revision corpus contains an indexed symbolic link: {}",
                            relative.display()
                        ),
                    ));
                }
                continue;
            }
            Err(error) => {
                return Err(RetainedRevisionError::new(
                    RetainedRevisionErrorKind::Provider,
                    format!(
                        "retained source revision entry cannot be opened: {}: {error}",
                        relative.display()
                    ),
                ));
            }
        };
        match child {
            RetainedChildCapability::Directory(child) => {
                manifest.insert(
                    relative.clone(),
                    SourceEntryDigest {
                        kind: ManifestEntryKind::Directory,
                        digest: [0; 32],
                        content_bytes: 0,
                    },
                );
                identities.insert(
                    relative.clone(),
                    (ManifestEntryKind::Directory, child.identity()),
                );
                #[cfg(test)]
                run_retained_scan_test_mutation(
                    RetainedScanTestMutationPoint::BeforeDirectoryRecursion,
                );
                scan_retained_directory(
                    &child,
                    &relative,
                    depth + 1,
                    context,
                    state,
                    manifest,
                    identities,
                )?;
                if child.validate_named_identity().is_err() {
                    state.namespace_stable = false;
                }
            }
            RetainedChildCapability::RegularFile(file)
                if context.artifact_policy.classify(&relative)
                    != RevisionArtifactDisposition::Ignored =>
            {
                let disposition = context.artifact_policy.classify(&relative);
                let (kind, digest, content_bytes) = match disposition {
                    RevisionArtifactDisposition::Content => {
                        #[cfg(test)]
                        run_retained_scan_test_mutation(
                            RetainedScanTestMutationPoint::BeforeFileHash,
                        );
                        let (digest, content_bytes) =
                            hash_retained_source_file(&file, &relative, context, state)?;
                        (ManifestEntryKind::Content, digest, content_bytes)
                    }
                    RevisionArtifactDisposition::Presence => {
                        (ManifestEntryKind::Presence, [0; 32], 0)
                    }
                    RevisionArtifactDisposition::Ignored => unreachable!("guarded match arm"),
                };
                identities.insert(relative.clone(), (kind, file.identity()));
                manifest.insert(
                    relative,
                    SourceEntryDigest {
                        kind,
                        digest,
                        content_bytes,
                    },
                );
                if file.validate_named_identity().is_err() {
                    state.namespace_stable = false;
                }
            }
            RetainedChildCapability::ReparsePoint
                if context.artifact_policy.classify(&relative)
                    != RevisionArtifactDisposition::Ignored =>
            {
                return Err(RetainedRevisionError::new(
                    RetainedRevisionErrorKind::ContainmentIdentity,
                    format!(
                        "retained source revision corpus contains an indexed reparse point: {}",
                        relative.display()
                    ),
                ));
            }
            RetainedChildCapability::RegularFile(_)
            | RetainedChildCapability::ReparsePoint
            | RetainedChildCapability::Unsupported => {}
        }
    }
    retained_revision_checkpoint(context.deadline, context.cancellation)?;
    let verification_names = directory
        .read_immediate_names_bounded(context.limits.max_entries, || {
            retained_revision_checkpoint(context.deadline, context.cancellation)
                .map_err(std::io::Error::other)
        })
        .map_err(|error| {
            if context.cancellation.is_cancelled() {
                RetainedRevisionError::new(
                    RetainedRevisionErrorKind::Cancelled,
                    "retained source revision reconcile cancelled",
                )
            } else if context.deadline.remaining().is_zero() {
                RetainedRevisionError::new(
                    RetainedRevisionErrorKind::Deadline,
                    "retained source revision deadline exceeded",
                )
            } else if error.kind() == ErrorKind::InvalidData {
                RetainedRevisionError::new(
                    RetainedRevisionErrorKind::Provider,
                    format!(
                        "retained source revision verification entry limit {} exceeded",
                        context.limits.max_entries
                    ),
                )
            } else {
                RetainedRevisionError::new(
                    RetainedRevisionErrorKind::Provider,
                    format!("retained source revision directory cannot be verified: {error}"),
                )
            }
        })?;
    state.verification_entries = state
        .verification_entries
        .checked_add(verification_names.len())
        .filter(|entries| *entries <= context.limits.max_entries)
        .ok_or_else(|| {
            RetainedRevisionError::new(
                RetainedRevisionErrorKind::Provider,
                format!(
                    "retained source revision verification entry limit {} exceeded",
                    context.limits.max_entries
                ),
            )
        })?;
    if verification_names != names || directory.validate_named_identity().is_err() {
        state.namespace_stable = false;
    }
    Ok(())
}

fn hash_retained_source_file(
    file: &crate::infrastructure::platform::filesystem::RetainedRegularFileCapability,
    relative: &Path,
    context: RetainedScanContext<'_>,
    state: &mut RetainedScanState,
) -> Result<([u8; 32], u64), RetainedRevisionError> {
    hash_retained_source_file_with_checkpoint(file, relative, context.limits, state, &mut || {
        retained_revision_checkpoint(context.deadline, context.cancellation)
    })
}

fn hash_retained_source_file_with_checkpoint(
    file: &crate::infrastructure::platform::filesystem::RetainedRegularFileCapability,
    relative: &Path,
    limits: RetainedScanLimits,
    state: &mut RetainedScanState,
    checkpoint: &mut dyn FnMut() -> Result<(), RetainedRevisionError>,
) -> Result<([u8; 32], u64), RetainedRevisionError> {
    let mut reader = file.try_clone_file().map_err(|error| {
        RetainedRevisionError::new(
            RetainedRevisionErrorKind::Provider,
            format!(
                "retained source revision file cannot be cloned: {}: {error}",
                relative.display()
            ),
        )
    })?;
    reader.seek(SeekFrom::Start(0)).map_err(|error| {
        RetainedRevisionError::new(
            RetainedRevisionErrorKind::Provider,
            format!(
                "retained source revision file cannot be rewound: {}: {error}",
                relative.display()
            ),
        )
    })?;
    let result = hash_bounded_source_reader(
        &mut reader,
        relative,
        limits,
        &mut state.total_bytes,
        checkpoint,
    )?;
    #[cfg(test)]
    run_retained_scan_test_mutation(RetainedScanTestMutationPoint::AfterFileHash);
    Ok(result)
}

/// Hashes one Content artifact through the common revision budget. Every
/// producer of a source manifest uses this routine, so ambient, retained and
/// incremental captures cannot silently acquire different byte/cancellation
/// semantics.
fn hash_bounded_source_reader(
    reader: &mut dyn Read,
    relative: &Path,
    limits: RetainedScanLimits,
    total_bytes: &mut u64,
    checkpoint: &mut dyn FnMut() -> Result<(), RetainedRevisionError>,
) -> Result<([u8; 32], u64), RetainedRevisionError> {
    let mut digest = Sha256::new();
    let mut file_bytes = 0_u64;
    let mut chunk = [0_u8; RETAINED_HASH_CHUNK_BYTES];
    loop {
        checkpoint()?;
        let read = reader.read(&mut chunk).map_err(|error| {
            RetainedRevisionError::new(
                RetainedRevisionErrorKind::Provider,
                format!(
                    "source revision file cannot be read: {}: {error}",
                    relative.display()
                ),
            )
        })?;
        if read == 0 {
            break;
        }
        let read = u64::try_from(read).expect("source revision chunk length fits u64");
        file_bytes = file_bytes.checked_add(read).ok_or_else(|| {
            RetainedRevisionError::new(
                RetainedRevisionErrorKind::Invariant,
                "source revision file byte count overflowed",
            )
        })?;
        if file_bytes > limits.max_file_bytes {
            return Err(RetainedRevisionError::new(
                RetainedRevisionErrorKind::Provider,
                format!(
                    "source revision file byte limit {} exceeded: {}",
                    limits.max_file_bytes,
                    relative.display()
                ),
            ));
        }
        *total_bytes = total_bytes.checked_add(read).ok_or_else(|| {
            RetainedRevisionError::new(
                RetainedRevisionErrorKind::Invariant,
                "source revision aggregate byte count overflowed",
            )
        })?;
        if *total_bytes > limits.max_total_bytes {
            return Err(RetainedRevisionError::new(
                RetainedRevisionErrorKind::Provider,
                format!(
                    "source revision aggregate byte limit {} exceeded",
                    limits.max_total_bytes
                ),
            ));
        }
        digest.update(&chunk[..usize::try_from(read).expect("chunk length fits usize")]);
    }
    Ok((digest.finalize().into(), file_bytes))
}

fn retained_revision_checkpoint(
    deadline: ProviderDeadline,
    cancellation: &CancellationToken,
) -> Result<(), RetainedRevisionError> {
    if cancellation.is_cancelled() {
        Err(RetainedRevisionError::new(
            RetainedRevisionErrorKind::Cancelled,
            "retained source revision reconcile cancelled",
        ))
    } else if deadline.remaining().is_zero() {
        Err(RetainedRevisionError::new(
            RetainedRevisionErrorKind::Deadline,
            "retained source revision deadline exceeded",
        ))
    } else {
        Ok(())
    }
}

#[cfg(test)]
fn scan_source_digest(
    source_root: &Path,
    should_stop: &(dyn Fn() -> bool + Sync),
) -> Result<String, String> {
    let manifest = scan_source_manifest(source_root, should_stop)?;
    digest_source_manifest(&manifest)
}

#[cfg(test)]
fn scan_source_manifest(
    source_root: &Path,
    should_stop: &(dyn Fn() -> bool + Sync),
) -> Result<SourceManifest, String> {
    scan_source_manifest_with_policy(
        source_root,
        RevisionArtifactPolicy::legacy_v12(),
        should_stop,
    )
}

fn scan_source_manifest_with_policy(
    source_root: &Path,
    artifact_policy: RevisionArtifactPolicy,
    should_stop: &(dyn Fn() -> bool + Sync),
) -> Result<SourceManifest, String> {
    if should_stop() {
        return Err("source revision reconcile cancelled".to_string());
    }
    let mut manifest = BTreeMap::new();
    let mut state = RetainedScanState::default();
    let context = AmbientScanContext {
        root: source_root,
        artifact_policy,
        limits: revision_scan_limits(),
        should_stop,
    };
    scan_directory(source_root, 0, context, &mut state, &mut manifest)?;
    Ok(manifest)
}

fn digest_source_manifest(manifest: &SourceManifest) -> Result<String, String> {
    let mut corpus = Sha256::new();
    corpus.update(b"unica-source-sha256-v1\0");
    for (relative, entry) in manifest {
        let path = relative.as_os_str().as_encoded_bytes();
        corpus.update([entry.kind as u8]);
        corpus.update((path.len() as u64).to_le_bytes());
        corpus.update(path);
        corpus.update(entry.digest);
    }
    Ok(format!("{:x}", corpus.finalize()))
}

fn read_source_file(path: &Path) -> Result<Box<dyn Read + Send>, String> {
    fs::File::open(path)
        .map(|file| Box::new(file) as Box<dyn Read + Send>)
        .map_err(|error| format!("source revision file cannot be opened: {error}"))
}

fn scan_directory(
    directory: &Path,
    depth: usize,
    context: AmbientScanContext<'_>,
    state: &mut RetainedScanState,
    manifest: &mut SourceManifest,
) -> Result<(), String> {
    if (context.should_stop)() {
        return Err("source revision reconcile cancelled".to_string());
    }
    if depth > MAX_SOURCE_DEPTH {
        return Err("source revision corpus exceeds maximum depth".to_string());
    }
    let mut children = Vec::new();
    for child in fs::read_dir(directory)
        .map_err(|error| format!("source revision directory cannot be read: {error}"))?
    {
        if (context.should_stop)() {
            return Err("source revision reconcile cancelled".to_string());
        }
        state.entries = state
            .entries
            .checked_add(1)
            .filter(|entries| *entries <= context.limits.max_entries)
            .ok_or_else(|| {
                format!(
                    "source revision entry limit {} exceeded",
                    context.limits.max_entries
                )
            })?;
        children
            .push(child.map_err(|error| format!("source revision entry cannot be read: {error}"))?);
    }
    children.sort_by_key(fs::DirEntry::file_name);
    for child in children {
        if (context.should_stop)() {
            return Err("source revision reconcile cancelled".to_string());
        }
        let file_type = child
            .file_type()
            .map_err(|error| format!("source revision entry type cannot be read: {error}"))?;
        let path = child.path();
        let relative = path
            .strip_prefix(context.root)
            .map_err(|_| "source revision entry escaped its root".to_string())?
            .to_path_buf();
        if file_type.is_symlink() {
            if context.artifact_policy.classify(&relative) != RevisionArtifactDisposition::Ignored {
                return Err(format!(
                    "source revision corpus contains an indexed symbolic link: {}",
                    relative.display()
                ));
            }
            continue;
        }
        if file_type.is_dir() {
            if host_directory_component_names_equivalent(
                directory,
                &child.file_name(),
                OsStr::new(GENERATED_DIR_NAME),
            )
            .map_err(|error| format!("generated-directory identity cannot be proven: {error}"))?
            {
                continue;
            }
            manifest.insert(
                relative,
                SourceEntryDigest {
                    kind: ManifestEntryKind::Directory,
                    digest: [0; 32],
                    content_bytes: 0,
                },
            );
            scan_directory(&path, depth + 1, context, state, manifest)?;
        } else if file_type.is_file() {
            match context.artifact_policy.classify(&relative) {
                RevisionArtifactDisposition::Content => {
                    let mut reader = read_source_file(&path)?;
                    let (digest, content_bytes) = hash_bounded_source_reader(
                        reader.as_mut(),
                        &relative,
                        context.limits,
                        &mut state.total_bytes,
                        &mut || {
                            if (context.should_stop)() {
                                Err(RetainedRevisionError::new(
                                    RetainedRevisionErrorKind::Cancelled,
                                    "source revision reconcile cancelled",
                                ))
                            } else {
                                Ok(())
                            }
                        },
                    )
                    .map_err(|error| error.to_string())?;
                    manifest.insert(
                        relative,
                        SourceEntryDigest {
                            kind: ManifestEntryKind::Content,
                            digest,
                            content_bytes,
                        },
                    );
                }
                RevisionArtifactDisposition::Presence => {
                    manifest.insert(
                        relative,
                        SourceEntryDigest {
                            kind: ManifestEntryKind::Presence,
                            digest: [0; 32],
                            content_bytes: 0,
                        },
                    );
                }
                RevisionArtifactDisposition::Ignored => {}
            }
        }
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::infrastructure::platform::filesystem::{
        retained_directory_case_policy_queries_for_test,
        set_retained_directory_case_policy_for_test, RetainedDirectoryCapability,
    };
    use crate::infrastructure::platform::testing::{
        create_dir_symlink_for_test, create_file_symlink_for_test,
    };
    use std::collections::VecDeque;
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc;
    use std::thread;
    use tempfile::tempdir;

    fn platform_actor_policy() -> RevisionArtifactPolicy {
        RevisionArtifactPolicy::platform_xml_for_test(
            crate::domain::project_sources::SourceSetKind::Configuration,
        )
    }

    fn actor_scan_test_service(
        workspace: &Path,
        source: &Path,
        fence: Arc<dyn SourceRevisionFence>,
        file_reader: Arc<SourceFileReader>,
    ) -> SourceRevisionService {
        let source = std::fs::canonicalize(source).unwrap();
        let policy = platform_actor_policy();
        SourceRevisionService {
            workspace_root: std::fs::canonicalize(workspace).unwrap(),
            source_root_identity: RetainedDirectoryCapability::open(&source)
                .unwrap()
                .identity(),
            source_root: source,
            record_path: workspace.join("revision-test.json"),
            state_scope: WorkspaceStateScope::scoped_sha256("a".repeat(64)).unwrap(),
            artifact_policy: policy,
            machine: Mutex::new(SourceRevisionMachine::default()),
            manifest: Mutex::new(None),
            manifest_provenance: Mutex::new(None),
            operation: DeadlineLock::default(),
            fence,
            scanner: Arc::new(move |root, should_stop| {
                scan_source_manifest_with_policy(root, policy, should_stop)
            }),
            file_reader,
            retained_scans: AtomicUsize::new(0),
        }
    }

    fn proven_scan_fence(changed: Option<PathBuf>) -> Arc<dyn SourceRevisionFence> {
        let mut outcomes = VecDeque::from([
            FenceOutcome::Proven {
                changed_paths: Vec::new(),
            },
            FenceOutcome::Proven {
                changed_paths: Vec::new(),
            },
        ]);
        if let Some(changed) = changed {
            outcomes.push_back(FenceOutcome::Proven {
                changed_paths: vec![changed],
            });
            outcomes.push_back(FenceOutcome::Proven {
                changed_paths: Vec::new(),
            });
        }
        Arc::new(ScriptedFence {
            outcomes: Mutex::new(outcomes),
        })
    }

    #[test]
    fn actor_revision_ambient_targeted_content_uses_retained_bounds_and_checkpoints() {
        let mut failures = Vec::new();
        for (label, first, second, limits) in [
            (
                "per-file",
                b"12345".as_slice(),
                None,
                RetainedScanLimits::new(64, 4, 64),
            ),
            (
                "aggregate",
                b"1234".as_slice(),
                Some(b"5678".as_slice()),
                RetainedScanLimits::new(64, 4, 7),
            ),
        ] {
            let workspace = tempdir().unwrap();
            let source = workspace.path().join("src");
            let first_path = source.join("XDTOPackages/First/Ext/Package.bin");
            std::fs::create_dir_all(first_path.parent().unwrap()).unwrap();
            std::fs::write(&first_path, first).unwrap();
            if let Some(second) = second {
                let second_path = source.join("XDTOPackages/Second/Ext/Package.bin");
                std::fs::create_dir_all(second_path.parent().unwrap()).unwrap();
                std::fs::write(second_path, second).unwrap();
            }
            let service = actor_scan_test_service(
                workspace.path(),
                &source,
                proven_scan_fence(None),
                Arc::new(read_source_file),
            );
            let _limits = set_revision_scan_limits_for_test(limits);
            if service
                .snapshot(
                    ProviderDeadline::from_budget(std::time::Duration::from_secs(5)),
                    &CancellationToken::new(),
                )
                .is_ok()
            {
                failures.push(format!("{label} ambient scan returned success"));
            }
        }

        let workspace = tempdir().unwrap();
        let source = workspace.path().join("src");
        let package = source.join("XDTOPackages/First/Ext/Package.bin");
        std::fs::create_dir_all(package.parent().unwrap()).unwrap();
        std::fs::write(&package, b"1234").unwrap();
        std::fs::write(source.join("scratch.bin"), vec![0_u8; 128]).unwrap();
        let service = actor_scan_test_service(
            workspace.path(),
            &source,
            proven_scan_fence(None),
            Arc::new(read_source_file),
        );
        let _limits = set_revision_scan_limits_for_test(RetainedScanLimits::new(64, 4, 4));
        if let Err(error) = service.snapshot(
            ProviderDeadline::from_budget(std::time::Duration::from_secs(5)),
            &CancellationToken::new(),
        ) {
            failures.push(format!(
                "ignored huge binary consumed ambient bytes: {error}"
            ));
        }

        {
            let workspace = tempdir().unwrap();
            let source = workspace.path().join("src");
            std::fs::create_dir_all(&source).unwrap();
            std::fs::write(
                source.join("Configuration.xml"),
                vec![7_u8; RETAINED_HASH_CHUNK_BYTES * 2],
            )
            .unwrap();
            let checkpoints = AtomicUsize::new(0);
            let cancellation = CancellationToken::new();
            let cancel_during_content = cancellation.clone();
            let result =
                scan_source_manifest_with_policy(&source, platform_actor_policy(), &|| {
                    if checkpoints.fetch_add(1, Ordering::AcqRel) >= 4 {
                        cancel_during_content.cancel();
                    }
                    cancellation.is_cancelled()
                });
            if result.is_ok() {
                failures.push("cancellation did not checkpoint during ambient content".to_string());
            }
        }
        {
            let workspace = tempdir().unwrap();
            let source = workspace.path().join("src");
            std::fs::create_dir_all(&source).unwrap();
            std::fs::write(
                source.join("Configuration.xml"),
                vec![7_u8; RETAINED_HASH_CHUNK_BYTES * 2],
            )
            .unwrap();
            let checkpoints = AtomicUsize::new(0);
            let deadline = ProviderDeadline::from_budget(std::time::Duration::from_millis(25));
            let result =
                scan_source_manifest_with_policy(&source, platform_actor_policy(), &|| {
                    if checkpoints.fetch_add(1, Ordering::AcqRel) == 4 {
                        std::thread::sleep(std::time::Duration::from_millis(50));
                    }
                    deadline.remaining().is_zero()
                });
            if result.is_ok() {
                failures.push("deadline did not checkpoint during ambient content".to_string());
            }
        }

        assert!(
            failures.is_empty(),
            "ambient targeted content bypassed retained semantics: {failures:?}"
        );
    }

    #[test]
    fn actor_revision_incremental_targeted_content_uses_retained_bounds_and_checkpoints() {
        let mut failures = Vec::new();
        for (label, initial, sibling, changed, limits) in [
            (
                "per-file",
                b"1234".as_slice(),
                None,
                b"12345".as_slice(),
                RetainedScanLimits::new(64, 4, 64),
            ),
            (
                "aggregate",
                b"123".as_slice(),
                Some(b"456".as_slice()),
                b"789".as_slice(),
                RetainedScanLimits::new(64, 4, 5),
            ),
        ] {
            let workspace = tempdir().unwrap();
            let source = workspace.path().join("src");
            let changed_relative = PathBuf::from("XDTOPackages/First/Ext/Package.bin");
            let changed_path = source.join(&changed_relative);
            std::fs::create_dir_all(changed_path.parent().unwrap()).unwrap();
            std::fs::write(&changed_path, initial).unwrap();
            if let Some(sibling) = sibling {
                let sibling_path = source.join("XDTOPackages/Second/Ext/Package.bin");
                std::fs::create_dir_all(sibling_path.parent().unwrap()).unwrap();
                std::fs::write(sibling_path, sibling).unwrap();
            }
            let service = actor_scan_test_service(
                workspace.path(),
                &source,
                proven_scan_fence(Some(changed_relative)),
                Arc::new(read_source_file),
            );
            service
                .snapshot(
                    ProviderDeadline::from_budget(std::time::Duration::from_secs(5)),
                    &CancellationToken::new(),
                )
                .unwrap();
            std::fs::write(&changed_path, changed).unwrap();
            let _limits = set_revision_scan_limits_for_test(limits);

            if service
                .snapshot(
                    ProviderDeadline::from_budget(std::time::Duration::from_secs(5)),
                    &CancellationToken::new(),
                )
                .is_ok()
            {
                failures.push(format!("{label} incremental scan returned success"));
            }
        }

        let workspace = tempdir().unwrap();
        let source = workspace.path().join("src");
        let changed_relative = PathBuf::from("XDTOPackages/First/Ext/Package.bin");
        let changed_path = source.join(&changed_relative);
        std::fs::create_dir_all(changed_path.parent().unwrap()).unwrap();
        std::fs::write(&changed_path, b"1234").unwrap();
        std::fs::write(source.join("scratch.bin"), vec![0_u8; 128]).unwrap();
        let service = actor_scan_test_service(
            workspace.path(),
            &source,
            proven_scan_fence(Some(changed_relative)),
            Arc::new(read_source_file),
        );
        service
            .snapshot(
                ProviderDeadline::from_budget(std::time::Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        std::fs::write(&changed_path, b"5678").unwrap();
        let _limits = set_revision_scan_limits_for_test(RetainedScanLimits::new(64, 4, 4));
        if let Err(error) = service.snapshot(
            ProviderDeadline::from_budget(std::time::Duration::from_secs(5)),
            &CancellationToken::new(),
        ) {
            failures.push(format!(
                "ignored huge binary consumed incremental bytes: {error}"
            ));
        }

        assert!(
            failures.is_empty(),
            "incremental targeted content bypassed retained bounds: {failures:?}"
        );
    }

    #[test]
    fn actor_revision_incremental_targeted_content_honors_mid_read_stop() {
        let mut failures = Vec::new();
        {
            let workspace = tempdir().unwrap();
            let source = workspace.path().join("src");
            let changed_relative = PathBuf::from("XDTOPackages/First/Ext/Package.bin");
            let changed_path = source.join(&changed_relative);
            std::fs::create_dir_all(changed_path.parent().unwrap()).unwrap();
            std::fs::write(&changed_path, vec![1_u8; RETAINED_HASH_CHUNK_BYTES * 2]).unwrap();
            let (started_tx, started_rx) = mpsc::channel();
            let (release_tx, release_rx) = mpsc::channel();
            let release_rx = Mutex::new(release_rx);
            let service = Arc::new(actor_scan_test_service(
                workspace.path(),
                &source,
                proven_scan_fence(Some(changed_relative)),
                Arc::new(move |path| {
                    started_tx.send(()).unwrap();
                    release_rx.lock().unwrap().recv().unwrap();
                    read_source_file(path)
                }),
            ));
            service
                .snapshot(
                    ProviderDeadline::from_budget(std::time::Duration::from_secs(5)),
                    &CancellationToken::new(),
                )
                .unwrap();
            std::fs::write(&changed_path, vec![2_u8; RETAINED_HASH_CHUNK_BYTES * 2]).unwrap();
            let cancellation = CancellationToken::new();
            let worker_cancellation = cancellation.clone();
            let worker_service = Arc::clone(&service);
            let worker = thread::spawn(move || {
                worker_service.snapshot(
                    ProviderDeadline::from_budget(std::time::Duration::from_secs(5)),
                    &worker_cancellation,
                )
            });
            started_rx
                .recv_timeout(std::time::Duration::from_secs(1))
                .unwrap();
            cancellation.cancel();
            release_tx.send(()).unwrap();
            if worker.join().unwrap().is_ok() {
                failures.push("incremental cancellation returned success".to_string());
            }
        }
        {
            let workspace = tempdir().unwrap();
            let source = workspace.path().join("src");
            let changed_relative = PathBuf::from("XDTOPackages/First/Ext/Package.bin");
            let changed_path = source.join(&changed_relative);
            std::fs::create_dir_all(changed_path.parent().unwrap()).unwrap();
            std::fs::write(&changed_path, vec![1_u8; RETAINED_HASH_CHUNK_BYTES * 2]).unwrap();
            let service = actor_scan_test_service(
                workspace.path(),
                &source,
                proven_scan_fence(Some(changed_relative)),
                Arc::new(|path| {
                    std::thread::sleep(std::time::Duration::from_millis(150));
                    read_source_file(path)
                }),
            );
            service
                .snapshot(
                    ProviderDeadline::from_budget(std::time::Duration::from_secs(5)),
                    &CancellationToken::new(),
                )
                .unwrap();
            std::fs::write(&changed_path, vec![2_u8; RETAINED_HASH_CHUNK_BYTES * 2]).unwrap();
            if service
                .snapshot(
                    ProviderDeadline::from_budget(std::time::Duration::from_millis(100)),
                    &CancellationToken::new(),
                )
                .is_ok()
            {
                failures.push("incremental deadline returned success".to_string());
            }
        }

        assert!(
            failures.is_empty(),
            "incremental targeted content skipped mid-read stop: {failures:?}"
        );
    }

    #[test]
    pub(crate) fn actor_revision_ignores_huge_unrelated_binary_while_bounding_targeted_resource() {
        let workspace = tempdir().unwrap();
        let source = workspace.path().join("src");
        let package = source.join("XDTOPackages/Sample/Ext/Package.bin");
        std::fs::create_dir_all(package.parent().unwrap()).unwrap();
        std::fs::write(source.join("scratch.bin"), vec![0_u8; 128]).unwrap();
        std::fs::write(&package, b"tiny").unwrap();
        let root =
            RetainedDirectoryCapability::open(&std::fs::canonicalize(&source).unwrap()).unwrap();

        let capture = capture_retained_source_manifest_with_limits(
            &root,
            ProviderDeadline::from_budget(std::time::Duration::from_secs(5)),
            &CancellationToken::new(),
            RetainedScanLimits::new(32, 8, 8),
            platform_actor_policy(),
        )
        .unwrap();

        assert!(
            capture
                .manifest
                .contains_key(Path::new("XDTOPackages/Sample/Ext/Package.bin")),
            "targeted Package.bin was omitted from the bounded actor manifest"
        );
        assert!(!capture.manifest.contains_key(Path::new("scratch.bin")));
    }

    #[test]
    pub(crate) fn actor_revision_targeted_resources_honor_cancellation_deadline_and_limits() {
        fn fixture() -> (tempfile::TempDir, RetainedDirectoryCapability) {
            let workspace = tempdir().unwrap();
            let package = workspace
                .path()
                .join("src/XDTOPackages/Sample/Ext/Package.bin");
            std::fs::create_dir_all(package.parent().unwrap()).unwrap();
            std::fs::write(&package, vec![7_u8; RETAINED_HASH_CHUNK_BYTES * 2]).unwrap();
            let root = RetainedDirectoryCapability::open(
                &std::fs::canonicalize(workspace.path().join("src")).unwrap(),
            )
            .unwrap();
            (workspace, root)
        }

        let mut failures = Vec::new();
        {
            let (_workspace, root) = fixture();
            let cancellation = CancellationToken::new();
            let cancel = cancellation.clone();
            let _guard = set_retained_scan_test_mutation(
                RetainedScanTestMutationPoint::BeforeFileHash,
                move || cancel.cancel(),
            );
            match capture_retained_source_manifest_with_limits(
                &root,
                ProviderDeadline::from_budget(std::time::Duration::from_secs(5)),
                &cancellation,
                RetainedScanLimits::PRODUCTION,
                platform_actor_policy(),
            ) {
                Err(error) if error.kind() == RetainedRevisionErrorKind::Cancelled => {}
                Err(error) => {
                    failures.push(format!("cancellation returned {:?}: {error}", error.kind()))
                }
                Ok(_) => failures.push("cancellation returned success".to_string()),
            }
        }
        {
            let (_workspace, root) = fixture();
            let _guard = set_retained_scan_test_mutation(
                RetainedScanTestMutationPoint::BeforeFileHash,
                || std::thread::sleep(std::time::Duration::from_millis(150)),
            );
            match capture_retained_source_manifest_with_limits(
                &root,
                ProviderDeadline::from_budget(std::time::Duration::from_millis(100)),
                &CancellationToken::new(),
                RetainedScanLimits::PRODUCTION,
                platform_actor_policy(),
            ) {
                Err(error) if error.kind() == RetainedRevisionErrorKind::Deadline => {}
                Err(error) => {
                    failures.push(format!("deadline returned {:?}: {error}", error.kind()))
                }
                Ok(_) => failures.push("deadline returned success".to_string()),
            }
        }
        for (label, limits, expected_fragment) in [
            (
                "per-file",
                RetainedScanLimits::new(32, 8, u64::MAX),
                "file byte limit 8 exceeded",
            ),
            (
                "aggregate",
                RetainedScanLimits::new(32, u64::MAX, 8),
                "aggregate byte limit 8 exceeded",
            ),
        ] {
            let (_workspace, root) = fixture();
            match capture_retained_source_manifest_with_limits(
                &root,
                ProviderDeadline::from_budget(std::time::Duration::from_secs(5)),
                &CancellationToken::new(),
                limits,
                platform_actor_policy(),
            ) {
                Err(error)
                    if error.kind() == RetainedRevisionErrorKind::Provider
                        && error.to_string().contains(expected_fragment) => {}
                Err(error) => {
                    failures.push(format!("{label} returned {:?}: {error}", error.kind()))
                }
                Ok(_) => failures.push(format!("{label} returned success")),
            }
        }

        assert!(
            failures.is_empty(),
            "targeted resource bounds were skipped: {failures:?}"
        );
    }

    #[test]
    pub(crate) fn projected_revision_artifacts_equal_retained_postpublication_capture() {
        crate::infrastructure::workspace_actor::tests::actor_revision_platform_resource_projection_matches_live_capture();
        crate::infrastructure::workspace_actor::tests::actor_revision_unknown_staged_artifact_is_rejected_before_publication();
        crate::infrastructure::workspace_actor::tests::actor_revision_late_failure_rolls_back_targeted_resource_without_receipt();
        actor_revision_projection_uses_capture_byte_limits_and_final_batch_accounting();
    }

    #[test]
    fn actor_revision_projection_uses_capture_byte_limits_and_final_batch_accounting() {
        use crate::infrastructure::native_operations::apply::StagedChangeKind;

        fn change(relative: &str, original: &[u8], current: StagedFileState) -> StagedApplyChange {
            StagedApplyChange {
                relative_path: PathBuf::from(relative),
                kind: match &current {
                    StagedFileState::Bytes(_) => StagedChangeKind::Replace,
                    StagedFileState::Absent => StagedChangeKind::Remove,
                },
                original: StagedFileState::Bytes(original.to_vec()),
                current,
            }
        }

        fn fixture(
            label: &str,
            files: &[(&str, &[u8])],
        ) -> (
            tempfile::TempDir,
            Arc<SourceRevisionService>,
            Arc<RetainedDirectoryCapability>,
        ) {
            let workspace = tempfile::Builder::new().prefix(label).tempdir().unwrap();
            let source = workspace.path().join("src");
            for (relative, bytes) in files {
                let path = source.join(relative);
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(path, bytes).unwrap();
            }
            let service = Arc::new(actor_scan_test_service(
                workspace.path(),
                &source,
                Arc::new(UnsupportedFence),
                Arc::new(read_source_file),
            ));
            let root = Arc::new(
                RetainedDirectoryCapability::open(&std::fs::canonicalize(source).unwrap()).unwrap(),
            );
            (workspace, service, root)
        }

        fn lease(
            service: &SourceRevisionService,
            root: &RetainedDirectoryCapability,
        ) -> RetainedRevisionLease {
            service
                .observe_retained_operation(
                    root,
                    ProviderDeadline::from_budget(std::time::Duration::from_secs(5)),
                    &CancellationToken::new(),
                )
                .unwrap()
        }

        let mut failures = Vec::new();

        {
            let (_workspace, service, root) = fixture(
                "projection-file-bound",
                &[("XDTOPackages/A/Ext/Package.bin", b"1234")],
            );
            let _limits = set_revision_scan_limits_for_test(RetainedScanLimits::new(64, 4, 64));
            let admitted = lease(&service, &root);
            let machine_before = service.machine_state_for_test();
            let source_before =
                std::fs::read(root.path().join("XDTOPackages/A/Ext/Package.bin")).unwrap();
            match service.prepare_retained_apply_reconciliation(
                &root,
                &admitted,
                &[change(
                    "XDTOPackages/A/Ext/Package.bin",
                    b"1234",
                    StagedFileState::Bytes(b"12345".to_vec()),
                )],
                ProviderDeadline::from_budget(std::time::Duration::from_secs(5)),
                &CancellationToken::new(),
            ) {
                Err(error)
                    if error.kind() == RetainedRevisionErrorKind::Provider
                        && error.to_string().contains("file byte limit 4 exceeded") => {}
                Err(error) => {
                    failures.push(format!("per-file returned {:?}: {error}", error.kind()))
                }
                Ok(_) => failures.push(
                    "per-file projection returned a publishable candidate/receipt authority"
                        .to_string(),
                ),
            }
            assert_eq!(
                std::fs::read(root.path().join("XDTOPackages/A/Ext/Package.bin")).unwrap(),
                source_before,
                "failed projection published source bytes"
            );
            assert_eq!(service.machine_state_for_test(), machine_before);
            assert!(
                !service.record_path.exists(),
                "failed projection wrote a revision record"
            );
        }

        {
            let (_workspace, service, root) = fixture(
                "projection-aggregate-bound",
                &[
                    ("XDTOPackages/A/Ext/Package.bin", b"1234"),
                    ("XDTOPackages/B/Ext/Package.bin", b"5678"),
                ],
            );
            let _limits = set_revision_scan_limits_for_test(RetainedScanLimits::new(64, 8, 8));
            let admitted = lease(&service, &root);
            match service.prepare_retained_apply_reconciliation(
                &root,
                &admitted,
                &[change(
                    "XDTOPackages/A/Ext/Package.bin",
                    b"1234",
                    StagedFileState::Bytes(b"12345".to_vec()),
                )],
                ProviderDeadline::from_budget(std::time::Duration::from_secs(5)),
                &CancellationToken::new(),
            ) {
                Err(error)
                    if error.kind() == RetainedRevisionErrorKind::Provider
                        && error
                            .to_string()
                            .contains("aggregate byte limit 8 exceeded") => {}
                Err(error) => {
                    failures.push(format!("aggregate returned {:?}: {error}", error.kind()))
                }
                Ok(_) => failures.push(
                    "aggregate projection returned a candidate no bounded capture can accept"
                        .to_string(),
                ),
            }
        }

        {
            let (_workspace, service, root) = fixture(
                "projection-final-batch",
                &[
                    ("XDTOPackages/A/Ext/Package.bin", b"1234"),
                    ("XDTOPackages/B/Ext/Package.bin", b"5678"),
                ],
            );
            let _limits = set_revision_scan_limits_for_test(RetainedScanLimits::new(64, 5, 8));
            let admitted = lease(&service, &root);
            let prepared = service.prepare_retained_apply_reconciliation(
                &root,
                &admitted,
                &[
                    change(
                        "XDTOPackages/A/Ext/Package.bin",
                        b"1234",
                        StagedFileState::Bytes(b"12345".to_vec()),
                    ),
                    change(
                        "XDTOPackages/B/Ext/Package.bin",
                        b"5678",
                        StagedFileState::Absent,
                    ),
                ],
                ProviderDeadline::from_budget(std::time::Duration::from_secs(5)),
                &CancellationToken::new(),
            );
            match prepared {
                Ok(prepared) => {
                    std::fs::write(root.path().join("XDTOPackages/A/Ext/Package.bin"), b"12345")
                        .unwrap();
                    std::fs::remove_file(root.path().join("XDTOPackages/B/Ext/Package.bin"))
                        .unwrap();
                    match prepared.activate(
                        ProviderDeadline::from_budget(std::time::Duration::from_secs(5)),
                        &CancellationToken::new(),
                    ) {
                        Ok(active) => {
                            if let Err(error) = active.validate_published_source(
                                ProviderDeadline::from_budget(std::time::Duration::from_secs(5)),
                                &CancellationToken::new(),
                            ) {
                                failures.push(format!(
                                    "final bounded batch digest diverged from capture: {error}"
                                ));
                            }
                        }
                        Err(error) => failures
                            .push(format!("final bounded batch could not activate: {error}")),
                    }
                }
                Err(error) => failures.push(format!(
                    "removal did not free aggregate bytes before replacement: {error}"
                )),
            }
        }

        assert!(
            failures.is_empty(),
            "projected content bypassed bounded capture semantics: {failures:?}"
        );
    }

    #[test]
    pub(crate) fn actor_revision_artifact_policy_contract_is_complete() {
        crate::infrastructure::revision_artifact_policy::tests::platform_xml_revision_artifact_profile_is_closed_and_legacy_is_unchanged();
        crate::infrastructure::revision_artifact_policy::tests::actor_revision_policy_has_no_raw_issuer_or_scoped_service_bypass();
        projected_revision_artifacts_equal_retained_postpublication_capture();
        crate::infrastructure::workspace_actor::tests::actor_revision_lookalike_resource_is_rejected_before_publication();
        crate::infrastructure::workspace_actor::tests::actor_revision_external_resource_drift_rotates_subsequent_admission();
        crate::infrastructure::workspace_actor::tests::actor_revision_policy_migrates_old_scoped_record_once_then_is_restart_stable();
        actor_revision_ignores_huge_unrelated_binary_while_bounding_targeted_resource();
        actor_revision_targeted_resources_honor_cancellation_deadline_and_limits();
        actor_revision_ambient_targeted_content_uses_retained_bounds_and_checkpoints();
        actor_revision_incremental_targeted_content_uses_retained_bounds_and_checkpoints();
        actor_revision_incremental_targeted_content_honors_mid_read_stop();
        crate::infrastructure::workspace_actor::tests::actor_revision_platform_commit_preserves_legacy_and_surface_contracts();
    }

    struct UnsupportedFence;

    impl SourceRevisionFence for UnsupportedFence {
        fn capability(&self) -> FenceCapability {
            FenceCapability::Unsupported
        }

        fn flush(
            &self,
            _deadline: ProviderDeadline,
            _cancellation: &CancellationToken,
        ) -> Result<FenceOutcome, String> {
            Ok(FenceOutcome::TrustLost(
                SourceRevisionTrustLoss::UnsupportedFence,
            ))
        }
    }

    struct ProvenCleanFence;

    impl SourceRevisionFence for ProvenCleanFence {
        fn capability(&self) -> FenceCapability {
            FenceCapability::ProvenFast
        }

        fn flush(
            &self,
            _deadline: ProviderDeadline,
            _cancellation: &CancellationToken,
        ) -> Result<FenceOutcome, String> {
            Ok(FenceOutcome::Proven {
                changed_paths: Vec::new(),
            })
        }
    }

    struct CountingProvenFence {
        calls: Arc<AtomicUsize>,
    }

    impl SourceRevisionFence for CountingProvenFence {
        fn capability(&self) -> FenceCapability {
            FenceCapability::ProvenFast
        }

        fn flush(
            &self,
            _deadline: ProviderDeadline,
            _cancellation: &CancellationToken,
        ) -> Result<FenceOutcome, String> {
            self.calls.fetch_add(1, Ordering::AcqRel);
            Ok(FenceOutcome::Proven {
                changed_paths: Vec::new(),
            })
        }
    }

    fn counting_legacy_service() -> (
        tempfile::TempDir,
        Arc<SourceRevisionService>,
        Arc<AtomicUsize>,
    ) {
        let workspace = tempdir().unwrap();
        let source_root = workspace.path().join("src");
        fs::create_dir_all(&source_root).unwrap();
        fs::write(source_root.join("Module.bsl"), "Процедура A()\n").unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let service = Arc::new(SourceRevisionService {
            workspace_root: fs::canonicalize(workspace.path()).unwrap(),
            source_root: fs::canonicalize(&source_root).unwrap(),
            source_root_identity: RetainedDirectoryCapability::open(
                &fs::canonicalize(&source_root).unwrap(),
            )
            .unwrap()
            .identity(),
            record_path: workspace.path().join("revision.json"),
            state_scope: WorkspaceStateScope::LegacyPhysical,
            artifact_policy: RevisionArtifactPolicy::legacy_v12(),
            machine: Mutex::new(SourceRevisionMachine::default()),
            manifest: Mutex::new(None),
            manifest_provenance: Mutex::new(None),
            operation: DeadlineLock::default(),
            fence: Arc::new(CountingProvenFence {
                calls: Arc::clone(&calls),
            }),
            scanner: Arc::new(scan_source_manifest),
            file_reader: Arc::new(read_source_file),
            retained_scans: AtomicUsize::new(0),
        });
        (workspace, service, calls)
    }

    #[test]
    fn source_revision_cancellation_bounds_contended_operation_lane() {
        let (_workspace, service, calls) = counting_legacy_service();
        let owner = service.operation.hold_for_test();
        let cancellation = CancellationToken::new();
        let waiter_cancellation = cancellation.clone();
        let waiter_service = Arc::clone(&service);
        let (started_tx, started_rx) = mpsc::channel();
        let (result_tx, result_rx) = mpsc::channel();
        let waiter = thread::spawn(move || {
            started_tx.send(()).unwrap();
            result_tx
                .send(waiter_service.snapshot(
                    ProviderDeadline::from_budget(std::time::Duration::from_secs(5)),
                    &waiter_cancellation,
                ))
                .unwrap();
        });
        started_rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap();
        assert!(
            result_rx
                .recv_timeout(std::time::Duration::from_millis(50))
                .is_err(),
            "snapshot unexpectedly crossed the held operation lane"
        );
        cancellation.cancel();

        let result_before_release = result_rx.recv_timeout(std::time::Duration::from_millis(250));
        let returned_before_release = result_before_release.is_ok();
        drop(owner);
        let result = result_before_release.unwrap_or_else(|_| {
            result_rx
                .recv_timeout(std::time::Duration::from_secs(2))
                .expect("snapshot must finish after the operation lane is released")
        });
        waiter.join().unwrap();

        assert!(
            returned_before_release,
            "cancellation did not stop the held source-revision operation wait"
        );
        assert!(result.unwrap_err().starts_with("cancelled:"));
        assert_eq!(
            calls.load(Ordering::Acquire),
            0,
            "cancelled operation executed the revision fence"
        );
    }

    #[test]
    fn source_revision_deadline_bounds_contended_operation_without_execution() {
        for (label, budget) in [
            ("expired", std::time::Duration::ZERO),
            ("elapsed", std::time::Duration::from_millis(40)),
        ] {
            let (_workspace, service, calls) = counting_legacy_service();
            let owner = service.operation.hold_for_test();
            let waiter_service = Arc::clone(&service);
            let (result_tx, result_rx) = mpsc::channel();
            let waiter = thread::spawn(move || {
                result_tx
                    .send(waiter_service.snapshot(
                        ProviderDeadline::from_budget(budget),
                        &CancellationToken::new(),
                    ))
                    .unwrap();
            });

            let result_before_release =
                result_rx.recv_timeout(std::time::Duration::from_millis(250));
            let returned_before_release = result_before_release.is_ok();
            drop(owner);
            let result = result_before_release.unwrap_or_else(|_| {
                result_rx
                    .recv_timeout(std::time::Duration::from_secs(2))
                    .expect("snapshot must finish after the operation lane is released")
            });
            waiter.join().unwrap();

            assert!(
                returned_before_release,
                "{label} deadline did not bound the source-revision operation wait"
            );
            assert!(result.unwrap_err().contains("deadline exceeded"));
            assert_eq!(
                calls.load(Ordering::Acquire),
                0,
                "timed-out operation executed the revision fence"
            );
        }
    }

    #[test]
    fn legacy_source_revision_wait_succeeds_when_released_before_deadline() {
        let (_workspace, service, calls) = counting_legacy_service();
        let owner = service.operation.hold_for_test();
        let waiter_service = Arc::clone(&service);
        let (started_tx, started_rx) = mpsc::channel();
        let (result_tx, result_rx) = mpsc::channel();
        let waiter = thread::spawn(move || {
            started_tx.send(()).unwrap();
            result_tx
                .send(waiter_service.snapshot(
                    ProviderDeadline::from_budget(std::time::Duration::from_secs(5)),
                    &CancellationToken::new(),
                ))
                .unwrap();
        });
        started_rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap();
        assert!(
            result_rx
                .recv_timeout(std::time::Duration::from_millis(50))
                .is_err(),
            "snapshot unexpectedly crossed the held operation lane"
        );
        drop(owner);

        let revision = result_rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap()
            .unwrap();
        waiter.join().unwrap();

        assert_eq!(revision.generation, 1);
        assert_eq!(calls.load(Ordering::Acquire), 2);
    }

    #[test]
    fn legacy_source_revision_recovers_a_poisoned_operation_lane() {
        let (_workspace, service, calls) = counting_legacy_service();
        let legacy_record_path = service.record_path.clone();
        let poison_service = Arc::clone(&service);
        let poison = thread::spawn(move || {
            let _operation = poison_service.hold_operation_for_test();
            panic!("poison source revision operation lane");
        });
        assert!(poison.join().is_err());

        let revision = service
            .snapshot(
                ProviderDeadline::from_budget(std::time::Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();

        assert_eq!(revision.generation, 1);
        assert_eq!(calls.load(Ordering::Acquire), 2);
        assert!(matches!(
            service.state_scope,
            WorkspaceStateScope::LegacyPhysical
        ));
        assert_eq!(service.record_path, legacy_record_path);
    }

    struct FailOnceFence {
        calls: AtomicUsize,
    }

    impl SourceRevisionFence for FailOnceFence {
        fn capability(&self) -> FenceCapability {
            FenceCapability::ProvenFast
        }

        fn flush(
            &self,
            _deadline: ProviderDeadline,
            _cancellation: &CancellationToken,
        ) -> Result<FenceOutcome, String> {
            if self.calls.fetch_add(1, Ordering::AcqRel) == 0 {
                Err("synthetic fence failure".to_string())
            } else {
                Ok(FenceOutcome::Proven {
                    changed_paths: Vec::new(),
                })
            }
        }
    }

    struct ScriptedFence {
        outcomes: Mutex<VecDeque<FenceOutcome>>,
    }

    impl SourceRevisionFence for ScriptedFence {
        fn capability(&self) -> FenceCapability {
            FenceCapability::ProvenFast
        }

        fn flush(
            &self,
            _deadline: ProviderDeadline,
            _cancellation: &CancellationToken,
        ) -> Result<FenceOutcome, String> {
            Ok(self
                .outcomes
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .pop_front()
                .unwrap_or(FenceOutcome::Proven {
                    changed_paths: Vec::new(),
                }))
        }
    }

    #[test]
    fn unsupported_fence_never_promotes_repeated_scans_to_trusted() {
        let workspace = tempdir().unwrap();
        let source_root = workspace.path().join("src");
        fs::create_dir_all(&source_root).unwrap();
        fs::write(source_root.join("Module.bsl"), "Процедура A()\n").unwrap();
        let full_scans = Arc::new(AtomicUsize::new(0));
        let service = SourceRevisionService {
            workspace_root: fs::canonicalize(workspace.path()).unwrap(),
            source_root: fs::canonicalize(&source_root).unwrap(),
            source_root_identity: RetainedDirectoryCapability::open(
                &fs::canonicalize(&source_root).unwrap(),
            )
            .unwrap()
            .identity(),
            record_path: workspace.path().join("revision.json"),
            state_scope: WorkspaceStateScope::LegacyPhysical,
            artifact_policy: RevisionArtifactPolicy::legacy_v12(),
            machine: Mutex::new(SourceRevisionMachine::default()),
            manifest: Mutex::new(None),
            manifest_provenance: Mutex::new(None),
            operation: DeadlineLock::default(),
            fence: Arc::new(UnsupportedFence),
            scanner: Arc::new({
                let full_scans = Arc::clone(&full_scans);
                move |root, should_stop| {
                    full_scans.fetch_add(1, Ordering::AcqRel);
                    scan_source_manifest(root, should_stop)
                }
            }),
            file_reader: Arc::new(read_source_file),
            retained_scans: AtomicUsize::new(0),
        };

        let error = service
            .snapshot(
                ProviderDeadline::from_budget(std::time::Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .expect_err("an unsupported fence cannot prove a trusted revision");

        assert!(error.contains("unsupported"), "{error}");
        assert_eq!(full_scans.load(Ordering::Acquire), 0);
    }

    #[test]
    fn revision_record_identity_includes_workspace_and_source_roots() {
        let sandbox = tempdir().unwrap();
        let workspace_a = sandbox.path().join("workspace-a");
        let workspace_b = sandbox.path().join("workspace-b");
        let source_root = sandbox.path().join("shared-source");
        let cache_root = sandbox.path().join("shared-cache");
        for path in [&workspace_a, &workspace_b, &source_root, &cache_root] {
            fs::create_dir_all(path).unwrap();
        }
        let context = |workspace_root: &Path| WorkspaceContext {
            cwd: workspace_root.to_path_buf(),
            workspace_root: workspace_root.to_path_buf(),
            cache_root: cache_root.clone(),
            workspace_epoch: 0,
        };

        let first = SourceRevisionService::new(&context(&workspace_a), &source_root).unwrap();
        let second = SourceRevisionService::new(&context(&workspace_b), &source_root).unwrap();

        assert_ne!(
            first.record_path, second.record_path,
            "a shared cache must not alias revision records from different workspaces"
        );
    }

    #[test]
    fn failed_revision_record_publication_does_not_leave_a_trusted_snapshot() {
        let workspace = tempdir().unwrap();
        let source_root = workspace.path().join("src");
        fs::create_dir_all(&source_root).unwrap();
        fs::write(source_root.join("Module.bsl"), "Процедура A()\n").unwrap();
        let record_parent = workspace.path().join("record-parent");
        fs::write(&record_parent, "not a directory").unwrap();
        let service = SourceRevisionService {
            workspace_root: fs::canonicalize(workspace.path()).unwrap(),
            source_root: fs::canonicalize(&source_root).unwrap(),
            source_root_identity: RetainedDirectoryCapability::open(
                &fs::canonicalize(&source_root).unwrap(),
            )
            .unwrap()
            .identity(),
            record_path: record_parent.join("revision.json"),
            state_scope: WorkspaceStateScope::LegacyPhysical,
            artifact_policy: RevisionArtifactPolicy::legacy_v12(),
            machine: Mutex::new(SourceRevisionMachine::default()),
            manifest: Mutex::new(None),
            manifest_provenance: Mutex::new(None),
            operation: DeadlineLock::default(),
            fence: Arc::new(ProvenCleanFence),
            scanner: Arc::new(scan_source_manifest),
            file_reader: Arc::new(read_source_file),
            retained_scans: AtomicUsize::new(0),
        };
        let snapshot = || {
            service.snapshot(
                ProviderDeadline::from_budget(std::time::Duration::from_secs(5)),
                &CancellationToken::new(),
            )
        };

        let first_error = snapshot().expect_err("publishing the revision must fail");
        assert!(first_error.contains("cannot be created"), "{first_error}");
        let second_error =
            snapshot().expect_err("a revision that was not published must remain untrusted");
        assert!(second_error.contains("cannot be created"), "{second_error}");
    }

    #[test]
    fn failed_fence_does_not_leave_the_previous_revision_trusted() {
        let workspace = tempdir().unwrap();
        let source_root = workspace.path().join("src");
        fs::create_dir_all(&source_root).unwrap();
        let module = source_root.join("Module.bsl");
        fs::write(&module, "Процедура A()\n").unwrap();
        let initial_digest = scan_source_digest(&source_root, &|| false).unwrap();
        let mut machine = SourceRevisionMachine::default();
        let initial_revision = machine.finish_reconcile(initial_digest).unwrap();
        let service = SourceRevisionService {
            workspace_root: fs::canonicalize(workspace.path()).unwrap(),
            source_root: fs::canonicalize(&source_root).unwrap(),
            source_root_identity: RetainedDirectoryCapability::open(
                &fs::canonicalize(&source_root).unwrap(),
            )
            .unwrap()
            .identity(),
            record_path: workspace.path().join("revision.json"),
            state_scope: WorkspaceStateScope::LegacyPhysical,
            artifact_policy: RevisionArtifactPolicy::legacy_v12(),
            machine: Mutex::new(machine),
            manifest: Mutex::new(None),
            manifest_provenance: Mutex::new(None),
            operation: DeadlineLock::default(),
            fence: Arc::new(FailOnceFence {
                calls: AtomicUsize::new(0),
            }),
            scanner: Arc::new(scan_source_manifest),
            file_reader: Arc::new(read_source_file),
            retained_scans: AtomicUsize::new(0),
        };

        let first_error = service
            .snapshot(
                ProviderDeadline::from_budget(std::time::Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .expect_err("the failed fence must reject the snapshot");
        assert_eq!(first_error, "synthetic fence failure");
        fs::write(&module, "Процедура B()\n").unwrap();

        let recovered = service
            .snapshot(
                ProviderDeadline::from_budget(std::time::Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        assert!(recovered.generation > initial_revision.generation);
        assert_ne!(recovered.digest, initial_revision.digest);
    }

    #[test]
    fn generated_directory_event_does_not_perturb_the_incremental_digest() {
        let workspace = tempdir().unwrap();
        let source_root = workspace.path().join("src");
        fs::create_dir_all(&source_root).unwrap();
        fs::write(source_root.join("Module.bsl"), "Процедура A()\n").unwrap();
        // A cache file with a source extension inside the pruned directory:
        // the full scan never records it, so the incremental path must not
        // record it either.
        fs::create_dir_all(source_root.join(".build/unica")).unwrap();
        fs::write(source_root.join(".build/unica/state.yaml"), "a: 1\n").unwrap();
        let full_scans = Arc::new(AtomicUsize::new(0));
        let incremental_reads = Arc::new(AtomicUsize::new(0));
        let service = SourceRevisionService {
            workspace_root: fs::canonicalize(workspace.path()).unwrap(),
            source_root: fs::canonicalize(&source_root).unwrap(),
            source_root_identity: RetainedDirectoryCapability::open(
                &fs::canonicalize(&source_root).unwrap(),
            )
            .unwrap()
            .identity(),
            record_path: workspace.path().join("revision.json"),
            state_scope: WorkspaceStateScope::LegacyPhysical,
            artifact_policy: RevisionArtifactPolicy::legacy_v12(),
            machine: Mutex::new(SourceRevisionMachine::default()),
            manifest: Mutex::new(None),
            manifest_provenance: Mutex::new(None),
            operation: DeadlineLock::default(),
            fence: Arc::new(ScriptedFence {
                outcomes: Mutex::new(VecDeque::from([
                    FenceOutcome::Proven {
                        changed_paths: Vec::new(),
                    },
                    FenceOutcome::Proven {
                        changed_paths: Vec::new(),
                    },
                    FenceOutcome::Proven {
                        changed_paths: vec![PathBuf::from(".build/unica/state.yaml")],
                    },
                ])),
            }),
            scanner: Arc::new({
                let full_scans = Arc::clone(&full_scans);
                move |root, should_stop| {
                    full_scans.fetch_add(1, Ordering::AcqRel);
                    scan_source_manifest(root, should_stop)
                }
            }),
            file_reader: Arc::new({
                let incremental_reads = Arc::clone(&incremental_reads);
                move |path| {
                    incremental_reads.fetch_add(1, Ordering::AcqRel);
                    read_source_file(path)
                }
            }),
            retained_scans: AtomicUsize::new(0),
        };
        let cancellation = CancellationToken::new();
        let snapshot = || {
            service
                .snapshot(
                    ProviderDeadline::from_budget(std::time::Duration::from_secs(60)),
                    &cancellation,
                )
                .unwrap()
        };

        let cold = snapshot();
        let scans_after_cold = full_scans.load(Ordering::Acquire);

        fs::write(source_root.join(".build/unica/state.yaml"), "a: 2\n").unwrap();
        let after_cache_write = snapshot();
        assert_eq!(
            after_cache_write.digest, cold.digest,
            "a generated-directory event must not change the corpus digest"
        );
        assert_eq!(
            full_scans.load(Ordering::Acquire),
            scans_after_cold,
            "a generated-directory event must not trigger a full rescan"
        );
        assert_eq!(
            incremental_reads.load(Ordering::Acquire),
            0,
            "a generated-directory file must not be read"
        );
    }

    #[test]
    fn external_file_edit_does_not_repeat_the_full_corpus_scan() {
        let workspace = tempdir().unwrap();
        let source_root = workspace.path().join("src");
        fs::create_dir_all(&source_root).unwrap();
        fs::write(source_root.join("Module.bsl"), "Процедура A()\n").unwrap();
        fs::write(source_root.join("Unchanged.bsl"), "Процедура B()\n").unwrap();
        let full_scans = Arc::new(AtomicUsize::new(0));
        let incremental_reads = Arc::new(AtomicUsize::new(0));
        let service = SourceRevisionService {
            workspace_root: fs::canonicalize(workspace.path()).unwrap(),
            source_root: fs::canonicalize(&source_root).unwrap(),
            source_root_identity: RetainedDirectoryCapability::open(
                &fs::canonicalize(&source_root).unwrap(),
            )
            .unwrap()
            .identity(),
            record_path: workspace.path().join("revision.json"),
            state_scope: WorkspaceStateScope::LegacyPhysical,
            artifact_policy: RevisionArtifactPolicy::legacy_v12(),
            machine: Mutex::new(SourceRevisionMachine::default()),
            manifest: Mutex::new(None),
            manifest_provenance: Mutex::new(None),
            operation: DeadlineLock::default(),
            fence: Arc::new(ScriptedFence {
                outcomes: Mutex::new(VecDeque::from([
                    FenceOutcome::Proven {
                        changed_paths: Vec::new(),
                    },
                    FenceOutcome::Proven {
                        changed_paths: Vec::new(),
                    },
                    FenceOutcome::Proven {
                        changed_paths: Vec::new(),
                    },
                    FenceOutcome::Proven {
                        changed_paths: vec![PathBuf::from("Module.bsl")],
                    },
                    FenceOutcome::Proven {
                        changed_paths: Vec::new(),
                    },
                    FenceOutcome::Proven {
                        changed_paths: vec![PathBuf::from("Module.bsl")],
                    },
                    FenceOutcome::Proven {
                        changed_paths: Vec::new(),
                    },
                ])),
            }),
            scanner: Arc::new({
                let full_scans = Arc::clone(&full_scans);
                move |root, should_stop| {
                    full_scans.fetch_add(1, Ordering::AcqRel);
                    scan_source_manifest(root, should_stop)
                }
            }),
            file_reader: Arc::new({
                let incremental_reads = Arc::clone(&incremental_reads);
                move |path| {
                    incremental_reads.fetch_add(1, Ordering::AcqRel);
                    read_source_file(path)
                }
            }),
            retained_scans: AtomicUsize::new(0),
        };
        let cancellation = CancellationToken::new();
        let snapshot = || {
            service
                .snapshot(
                    ProviderDeadline::from_budget(std::time::Duration::from_secs(5)),
                    &cancellation,
                )
                .unwrap()
        };

        let scans_before = full_scans.load(Ordering::Acquire);
        let first = snapshot();
        let scans_after_cold = full_scans.load(Ordering::Acquire);
        assert!(scans_after_cold > scans_before);
        let warm = snapshot();
        assert_eq!(warm, first);
        assert_eq!(
            full_scans.load(Ordering::Acquire),
            scans_after_cold,
            "a warm trusted snapshot must not walk the source tree"
        );

        fs::write(source_root.join("Module.bsl"), "Процедура B()\n").unwrap();
        let changed = snapshot();
        assert!(changed.generation > first.generation);
        assert_eq!(
            full_scans.load(Ordering::Acquire),
            scans_after_cold,
            "a precise external file event must not reread the whole corpus"
        );
        assert_eq!(
            incremental_reads.load(Ordering::Acquire),
            1,
            "only the changed file must be read"
        );

        fs::remove_file(source_root.join("Module.bsl")).unwrap();
        let removed = snapshot();
        assert!(removed.generation > changed.generation);
        assert_eq!(
            full_scans.load(Ordering::Acquire),
            scans_after_cold,
            "a precise file removal must update the manifest without a full scan"
        );
        assert_eq!(
            incremental_reads.load(Ordering::Acquire),
            1,
            "a removed file must not be read"
        );
    }

    #[test]
    fn concurrent_mark_dirty_rejects_incremental_publication_and_reconciles_again() {
        let workspace = tempdir().unwrap();
        let source_root = workspace.path().join("src");
        fs::create_dir_all(&source_root).unwrap();
        let module = source_root.join("Module.bsl");
        fs::write(&module, "Процедура A()\n").unwrap();
        let full_scans = Arc::new(AtomicUsize::new(0));
        let (read_started_tx, read_started_rx) = mpsc::channel();
        let (read_release_tx, read_release_rx) = mpsc::channel();
        let read_release_rx = Mutex::new(read_release_rx);
        let service = Arc::new(SourceRevisionService {
            workspace_root: fs::canonicalize(workspace.path()).unwrap(),
            source_root: fs::canonicalize(&source_root).unwrap(),
            source_root_identity: RetainedDirectoryCapability::open(
                &fs::canonicalize(&source_root).unwrap(),
            )
            .unwrap()
            .identity(),
            record_path: workspace.path().join("revision.json"),
            state_scope: WorkspaceStateScope::LegacyPhysical,
            artifact_policy: RevisionArtifactPolicy::legacy_v12(),
            machine: Mutex::new(SourceRevisionMachine::default()),
            manifest: Mutex::new(None),
            manifest_provenance: Mutex::new(None),
            operation: DeadlineLock::default(),
            fence: Arc::new(ScriptedFence {
                outcomes: Mutex::new(VecDeque::from([
                    FenceOutcome::Proven {
                        changed_paths: Vec::new(),
                    },
                    FenceOutcome::Proven {
                        changed_paths: Vec::new(),
                    },
                    FenceOutcome::Proven {
                        changed_paths: vec![PathBuf::from("Module.bsl")],
                    },
                    FenceOutcome::Proven {
                        changed_paths: Vec::new(),
                    },
                    FenceOutcome::Proven {
                        changed_paths: Vec::new(),
                    },
                ])),
            }),
            scanner: Arc::new({
                let full_scans = Arc::clone(&full_scans);
                move |root, should_stop| {
                    full_scans.fetch_add(1, Ordering::AcqRel);
                    scan_source_manifest(root, should_stop)
                }
            }),
            file_reader: Arc::new(move |path| {
                read_started_tx.send(()).unwrap();
                read_release_rx.lock().unwrap().recv().unwrap();
                read_source_file(path)
            }),
            retained_scans: AtomicUsize::new(0),
        });
        let initial = service
            .snapshot(
                ProviderDeadline::from_budget(std::time::Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        assert_eq!(full_scans.load(Ordering::Acquire), 1);
        fs::write(&module, "Процедура B()\n").unwrap();

        let worker_service = Arc::clone(&service);
        let worker = thread::spawn(move || {
            worker_service.snapshot(
                ProviderDeadline::from_budget(std::time::Duration::from_secs(5)),
                &CancellationToken::new(),
            )
        });
        read_started_rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .unwrap();
        service.mark_dirty();
        read_release_tx.send(()).unwrap();
        let changed = worker.join().unwrap().unwrap();

        assert!(changed.generation > initial.generation);
        assert_eq!(
            full_scans.load(Ordering::Acquire),
            2,
            "a watcher gap during incremental update must force a full reconcile"
        );
    }

    #[test]
    fn corpus_digest_tracks_content_and_path_but_ignores_generated_cache() {
        let first = tempdir().unwrap();
        let second = tempdir().unwrap();
        for root in [first.path(), second.path()] {
            fs::create_dir_all(root.join("CommonModules/Sales/Ext")).unwrap();
            fs::write(
                root.join("CommonModules/Sales/Ext/Module.bsl"),
                "Процедура A()\n",
            )
            .unwrap();
        }
        let baseline = scan_source_digest(first.path(), &|| false).unwrap();
        assert_eq!(
            baseline,
            scan_source_digest(second.path(), &|| false).unwrap()
        );

        fs::create_dir_all(first.path().join(".build")).unwrap();
        fs::write(first.path().join(".build/cache.db"), "private").unwrap();
        assert_eq!(
            baseline,
            scan_source_digest(first.path(), &|| false).unwrap()
        );

        fs::write(
            first.path().join("CommonModules/Sales/Ext/Module.bsl"),
            "Процедура B()\n",
        )
        .unwrap();
        assert_ne!(
            baseline,
            scan_source_digest(first.path(), &|| false).unwrap()
        );
    }

    #[test]
    fn corpus_digest_ignores_platform_equivalent_generated_directory_name() {
        let root = tempdir().unwrap();
        let canonical = fs::canonicalize(root.path()).unwrap();
        if crate::infrastructure::platform::filesystem::host_filesystem_case_sensitive(&canonical)
            .unwrap()
        {
            return;
        }
        fs::write(root.path().join("Module.bsl"), "Процедура A()\n").unwrap();
        let baseline = scan_source_digest(&canonical, &|| false).unwrap();
        fs::create_dir_all(root.path().join(".BUILD/unica")).unwrap();
        fs::write(
            root.path().join(".BUILD/unica/GeneratedModule.bsl"),
            "Процедура Cache()\n",
        )
        .unwrap();

        assert_eq!(baseline, scan_source_digest(&canonical, &|| false).unwrap());
    }

    #[test]
    fn corpus_digest_accepts_a_non_utf8_relative_path() {
        let Some(path) =
            crate::infrastructure::platform::testing::non_utf8_relative_path_for_test()
        else {
            return;
        };
        let mut manifest = SourceManifest::new();
        manifest.insert(
            path,
            SourceEntryDigest {
                kind: ManifestEntryKind::Content,
                digest: [7; 32],
                content_bytes: 1,
            },
        );

        assert!(
            digest_source_manifest(&manifest).is_ok(),
            "a valid filesystem name must not disable source revisions"
        );
    }

    #[test]
    fn corpus_digest_ignores_a_symlink_outside_the_rlm_corpus() {
        let workspace = tempdir().unwrap();
        let source_root = workspace.path().join("src");
        fs::create_dir_all(&source_root).unwrap();
        fs::write(source_root.join("Module.bsl"), "Процедура A()\n").unwrap();
        let baseline = scan_source_digest(&source_root, &|| false).unwrap();
        let outside = workspace.path().join("README.txt");
        fs::write(&outside, "not indexed by RLM").unwrap();
        let Some(link) = create_file_symlink_for_test(&outside, source_root.join("README.txt"))
        else {
            return;
        };
        link.unwrap();

        assert_eq!(
            baseline,
            scan_source_digest(&source_root, &|| false).unwrap()
        );
    }

    #[test]
    fn corpus_digest_does_not_follow_a_symlinked_directory() {
        let workspace = tempdir().unwrap();
        let source_root = workspace.path().join("src");
        let outside = workspace.path().join("outside");
        fs::create_dir_all(&source_root).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("External.bsl"), "Процедура A()\n").unwrap();
        let baseline = scan_source_digest(&source_root, &|| false).unwrap();
        let Some(link) = create_dir_symlink_for_test(&outside, source_root.join("vendor")) else {
            return;
        };
        link.unwrap();

        assert_eq!(
            baseline,
            scan_source_digest(&source_root, &|| false).unwrap()
        );
    }

    #[test]
    fn corpus_digest_rejects_an_indexed_symlink_with_its_relative_path() {
        let workspace = tempdir().unwrap();
        let source_root = workspace.path().join("src");
        fs::create_dir_all(&source_root).unwrap();
        let outside = workspace.path().join("Module.bsl");
        fs::write(&outside, "Процедура A()\n").unwrap();
        let Some(link) = create_file_symlink_for_test(&outside, source_root.join("Linked.bsl"))
        else {
            return;
        };
        link.unwrap();

        let error = scan_source_digest(&source_root, &|| false)
            .expect_err("an RLM-indexed symlink cannot produce a trusted revision");

        assert!(error.contains("Linked.bsl"), "{error}");
    }

    #[test]
    fn corpus_digest_tracks_edt_metadata() {
        let source_root = tempdir().unwrap();
        let metadata = source_root.path().join("Catalog.mdo");
        fs::write(&metadata, "<mdclass:Catalog uuid=\"first\"/>").unwrap();
        let baseline = scan_source_digest(source_root.path(), &|| false).unwrap();

        fs::write(&metadata, "<mdclass:Catalog uuid=\"second\"/>").unwrap();

        assert_ne!(
            baseline,
            scan_source_digest(source_root.path(), &|| false).unwrap()
        );
    }

    #[test]
    fn retained_snapshot_rejects_a_capability_from_another_source_identity() {
        let workspace = tempdir().unwrap();
        let source_a = workspace.path().join("source-a");
        let source_b = workspace.path().join("source-b");
        fs::create_dir_all(&source_a).unwrap();
        fs::create_dir_all(&source_b).unwrap();
        fs::write(
            source_a.join("Configuration.xml"),
            "<Configuration>A</Configuration>",
        )
        .unwrap();
        fs::write(
            source_b.join("Configuration.xml"),
            "<Configuration>B</Configuration>",
        )
        .unwrap();
        let context = WorkspaceContext {
            cwd: workspace.path().to_path_buf(),
            workspace_root: workspace.path().to_path_buf(),
            cache_root: workspace.path().join("cache"),
            workspace_epoch: 0,
        };
        let service = SourceRevisionService::new_reconciling_for_test(&context, &source_a)
            .expect("revision service");
        let retained_b = RetainedDirectoryCapability::open(&fs::canonicalize(&source_b).unwrap())
            .expect("retain source B");

        let error = service
            .snapshot_retained(
                &retained_b,
                ProviderDeadline::from_budget(std::time::Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .expect_err("an actor revision service cannot accept another retained source root");

        assert!(error.contains("identity"), "{error}");
    }

    #[test]
    fn retained_snapshot_reuses_a_clean_fence_and_reconciles_once_after_change() {
        let workspace = tempdir().unwrap();
        let source = workspace.path().join("source");
        fs::create_dir_all(&source).unwrap();
        let descriptor = source.join("Configuration.xml");
        fs::write(&descriptor, "<Configuration>A</Configuration>").unwrap();
        let context = WorkspaceContext {
            cwd: workspace.path().to_path_buf(),
            workspace_root: workspace.path().to_path_buf(),
            cache_root: workspace.path().join("cache"),
            workspace_epoch: 0,
        };
        let service = SourceRevisionService::new_with_fence_for_test(
            &context,
            &source,
            WorkspaceStateScope::LegacyPhysical,
            Arc::new(ScriptedFence {
                outcomes: Mutex::new(VecDeque::from([
                    FenceOutcome::Proven {
                        changed_paths: Vec::new(),
                    },
                    FenceOutcome::Proven {
                        changed_paths: Vec::new(),
                    },
                    FenceOutcome::Proven {
                        changed_paths: Vec::new(),
                    },
                    FenceOutcome::Proven {
                        changed_paths: vec![PathBuf::from("Configuration.xml")],
                    },
                    FenceOutcome::Proven {
                        changed_paths: Vec::new(),
                    },
                ])),
            }),
        )
        .unwrap();
        let retained = RetainedDirectoryCapability::open(&fs::canonicalize(&source).unwrap())
            .expect("retain source");
        let first = service
            .snapshot_retained(
                &retained,
                ProviderDeadline::from_budget(std::time::Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        assert_eq!(service.retained_scan_count(), 1, "one cold retained scan");
        fs::write(&descriptor, "<Configuration>B</Configuration>").unwrap();

        let cached = service
            .snapshot_retained(
                &retained,
                ProviderDeadline::from_budget(std::time::Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        assert_eq!(
            service.retained_scan_count(),
            1,
            "a proven-clean fence must not repeat the retained scan"
        );
        let changed = service
            .snapshot_retained(
                &retained,
                ProviderDeadline::from_budget(std::time::Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        assert_eq!(
            service.retained_scan_count(),
            2,
            "one changed fence must trigger exactly one retained reconcile"
        );

        assert_eq!(
            cached, first,
            "a proven-clean fence must reuse the manifest"
        );
        assert!(changed.generation > first.generation);
        assert_ne!(changed.digest, first.digest);
    }

    #[test]
    fn retained_manifest_uses_the_existing_source_digest_algorithm() {
        let workspace = tempdir().unwrap();
        let source = workspace.path().join("source");
        fs::create_dir_all(source.join("Catalogs/Orders/Ext")).unwrap();
        fs::write(source.join("Configuration.xml"), "<Configuration/>").unwrap();
        fs::write(
            source.join("Catalogs/Orders/Ext/Module.bsl"),
            "Процедура A()\nКонецПроцедуры",
        )
        .unwrap();
        let context = WorkspaceContext {
            cwd: workspace.path().to_path_buf(),
            workspace_root: workspace.path().to_path_buf(),
            cache_root: workspace.path().join("cache"),
            workspace_epoch: 0,
        };
        let service = SourceRevisionService::new_reconciling_for_test(&context, &source).unwrap();
        let retained = RetainedDirectoryCapability::open(&fs::canonicalize(&source).unwrap())
            .expect("retain source");

        let retained_revision = service
            .snapshot_retained(
                &retained,
                ProviderDeadline::from_budget(std::time::Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();

        assert_eq!(
            retained_revision.digest,
            scan_source_digest(&source, &|| false).unwrap()
        );
    }

    #[test]
    fn ambient_manifest_cannot_satisfy_a_retained_fast_path() {
        if !supports_retained_root_replacement_test() {
            return;
        }
        let workspace = tempdir().unwrap();
        let source = workspace.path().join("source");
        let replacement = workspace.path().join("replacement");
        let saved = workspace.path().join("saved");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&replacement).unwrap();
        fs::write(
            source.join("Configuration.xml"),
            "<Configuration>A</Configuration>",
        )
        .unwrap();
        fs::write(
            replacement.join("Configuration.xml"),
            "<Configuration>B</Configuration>",
        )
        .unwrap();
        let context = WorkspaceContext {
            cwd: workspace.path().to_path_buf(),
            workspace_root: workspace.path().to_path_buf(),
            cache_root: workspace.path().join("cache"),
            workspace_epoch: 0,
        };
        let service = SourceRevisionService::new_with_fence_for_test(
            &context,
            &source,
            WorkspaceStateScope::LegacyPhysical,
            Arc::new(ScriptedFence {
                outcomes: Mutex::new(VecDeque::from([
                    FenceOutcome::TrustLost(SourceRevisionTrustLoss::WatcherGap),
                    FenceOutcome::Proven {
                        changed_paths: Vec::new(),
                    },
                    FenceOutcome::Proven {
                        changed_paths: Vec::new(),
                    },
                    FenceOutcome::Proven {
                        changed_paths: Vec::new(),
                    },
                ])),
            }),
        )
        .unwrap();
        let retained = RetainedDirectoryCapability::open(&fs::canonicalize(&source).unwrap())
            .expect("retain source A");
        fs::rename(&source, &saved).unwrap();
        fs::rename(&replacement, &source).unwrap();
        let ambient_b = service
            .snapshot(
                ProviderDeadline::from_budget(std::time::Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        fs::rename(&source, &replacement).unwrap();
        fs::rename(&saved, &source).unwrap();

        let retained_a = service
            .snapshot_retained(
                &retained,
                ProviderDeadline::from_budget(std::time::Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();

        assert_ne!(retained_a.digest, ambient_b.digest);
        assert_eq!(
            retained_a.digest,
            scan_source_digest(&source, &|| false).unwrap()
        );
        assert_eq!(service.retained_scan_count(), 1);
    }

    #[test]
    fn retained_snapshot_never_mixes_a_replaced_root_name_with_the_open_tree() {
        if !supports_retained_root_replacement_test() {
            return;
        }
        let workspace = tempdir().unwrap();
        let source = workspace.path().join("source");
        let replacement = workspace.path().join("replacement");
        let saved = workspace.path().join("saved");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&replacement).unwrap();
        fs::write(
            source.join("Configuration.xml"),
            "<Configuration>A</Configuration>",
        )
        .unwrap();
        fs::write(
            replacement.join("Configuration.xml"),
            "<Configuration>B</Configuration>",
        )
        .unwrap();
        let context = WorkspaceContext {
            cwd: workspace.path().to_path_buf(),
            workspace_root: workspace.path().to_path_buf(),
            cache_root: workspace.path().join("cache"),
            workspace_epoch: 0,
        };
        let service = SourceRevisionService::new_reconciling_for_test(&context, &source)
            .expect("revision service");
        let retained = RetainedDirectoryCapability::open(&fs::canonicalize(&source).unwrap())
            .expect("retain source A");
        let baseline = service
            .snapshot_retained(
                &retained,
                ProviderDeadline::from_budget(std::time::Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .expect("baseline retained snapshot");

        fs::rename(&source, &saved).unwrap();
        fs::rename(&replacement, &source).unwrap();
        let after_replacement = service.snapshot_retained(
            &retained,
            ProviderDeadline::from_budget(std::time::Duration::from_secs(5)),
            &CancellationToken::new(),
        );

        match after_replacement {
            Ok(revision) => assert_eq!(revision, baseline),
            Err(error) => assert!(
                error.contains("identity") || error.contains("changed"),
                "typed invalidation expected, got: {error}"
            ),
        }
    }

    #[test]
    fn review_retained_manifest_cannot_satisfy_an_ambient_fast_path_after_root_swap() {
        if !supports_retained_root_replacement_test() {
            return;
        }
        let workspace = tempdir().unwrap();
        let source = workspace.path().join("source");
        let replacement = workspace.path().join("replacement");
        let saved = workspace.path().join("saved");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&replacement).unwrap();
        fs::write(
            source.join("Configuration.xml"),
            "<Configuration>A</Configuration>",
        )
        .unwrap();
        fs::write(
            replacement.join("Configuration.xml"),
            "<Configuration>B</Configuration>",
        )
        .unwrap();
        let context = WorkspaceContext {
            cwd: workspace.path().to_path_buf(),
            workspace_root: workspace.path().to_path_buf(),
            cache_root: workspace.path().join("cache"),
            workspace_epoch: 0,
        };
        let service = SourceRevisionService::new_with_fence_for_test(
            &context,
            &source,
            WorkspaceStateScope::LegacyPhysical,
            Arc::new(ScriptedFence {
                outcomes: Mutex::new(VecDeque::from([
                    FenceOutcome::Proven {
                        changed_paths: Vec::new(),
                    },
                    FenceOutcome::Proven {
                        changed_paths: Vec::new(),
                    },
                    FenceOutcome::Proven {
                        changed_paths: Vec::new(),
                    },
                    FenceOutcome::Proven {
                        changed_paths: Vec::new(),
                    },
                ])),
            }),
        )
        .unwrap();
        let retained = RetainedDirectoryCapability::open(&fs::canonicalize(&source).unwrap())
            .expect("retain source A");
        let retained_a = service
            .snapshot_retained(
                &retained,
                ProviderDeadline::from_budget(std::time::Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();

        fs::rename(&source, &saved).unwrap();
        fs::rename(&replacement, &source).unwrap();
        let ambient_after_swap = service
            .snapshot(
                ProviderDeadline::from_budget(std::time::Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();

        assert_ne!(
            retained_a.digest,
            scan_source_digest(&source, &|| false).unwrap()
        );
        assert_eq!(
            ambient_after_swap.digest,
            scan_source_digest(&source, &|| false).unwrap(),
            "ambient snapshot must describe lexical source B, not retained source A"
        );
    }

    #[test]
    fn unsupported_fence_stable_operation_lease_scans_at_admission_and_confirmation() {
        let workspace = tempdir().unwrap();
        let source = workspace.path().join("source");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("Configuration.xml"), "<Configuration/>").unwrap();
        let context = WorkspaceContext {
            cwd: workspace.path().to_path_buf(),
            workspace_root: workspace.path().to_path_buf(),
            cache_root: workspace.path().join("cache"),
            workspace_epoch: 0,
        };
        let service = SourceRevisionService::new_with_fence_for_test(
            &context,
            &source,
            WorkspaceStateScope::LegacyPhysical,
            Arc::new(UnsupportedFence),
        )
        .unwrap();
        let retained = RetainedDirectoryCapability::open(&fs::canonicalize(&source).unwrap())
            .expect("retain source");
        let deadline = ProviderDeadline::from_budget(std::time::Duration::from_secs(5));
        let cancellation = CancellationToken::new();

        let lease = service
            .begin_retained_operation(&retained, deadline, &cancellation)
            .unwrap();
        assert_eq!(service.retained_scan_count(), 2);
        for _ in 0..100 {
            assert_eq!(lease.revision_identity(), lease.revision_identity());
        }
        assert_eq!(
            service.retained_scan_count(),
            2,
            "logical node reads must not rescan the retained corpus"
        );

        service
            .confirm_retained_operation(&retained, &lease, deadline, &cancellation)
            .unwrap();
        assert_eq!(
            service.retained_scan_count(),
            4,
            "unsupported fence performs two stabilized admission passes and two final passes"
        );
    }

    #[test]
    fn unsupported_fence_reconcile_is_bounded_to_six_passes_when_corpus_never_stabilizes() {
        let fixture = RetainedConfirmationFixture::new(|source| {
            fs::write(source.join("Configuration.xml"), "A").unwrap();
        });
        let before = fixture.service.retained_scan_count();
        let descriptor = fixture.source.join("Configuration.xml");
        let write_a = std::cell::Cell::new(false);
        let _mutation = set_repeating_retained_scan_test_mutation(
            RetainedScanTestMutationPoint::ScanStart,
            move || {
                let contents = if write_a.replace(!write_a.get()) {
                    "A"
                } else {
                    "B"
                };
                fs::write(&descriptor, contents).unwrap();
            },
        );

        assert!(fixture.confirm().is_err());
        assert_eq!(
            fixture.service.retained_scan_count() - before,
            6,
            "three bounded stabilization attempts must perform exactly two passes each"
        );
    }

    struct RetainedConfirmationFixture {
        workspace: tempfile::TempDir,
        source: PathBuf,
        service: SourceRevisionService,
        retained: RetainedDirectoryCapability,
        lease: RetainedRevisionLease,
        deadline: ProviderDeadline,
        cancellation: CancellationToken,
    }

    impl RetainedConfirmationFixture {
        fn new(populate: impl FnOnce(&Path)) -> Self {
            let workspace = tempdir().unwrap();
            let source = workspace.path().join("source");
            fs::create_dir_all(&source).unwrap();
            populate(&source);
            let context = WorkspaceContext {
                cwd: workspace.path().to_path_buf(),
                workspace_root: workspace.path().to_path_buf(),
                cache_root: workspace.path().join("cache"),
                workspace_epoch: 0,
            };
            let service = SourceRevisionService::new_with_fence_for_test(
                &context,
                &source,
                WorkspaceStateScope::LegacyPhysical,
                Arc::new(UnsupportedFence),
            )
            .unwrap();
            let retained =
                RetainedDirectoryCapability::open(&fs::canonicalize(&source).unwrap()).unwrap();
            let deadline = ProviderDeadline::from_budget(std::time::Duration::from_secs(5));
            let cancellation = CancellationToken::new();
            let lease = service
                .begin_retained_operation(&retained, deadline, &cancellation)
                .unwrap();
            Self {
                workspace,
                source,
                service,
                retained,
                lease,
                deadline,
                cancellation,
            }
        }

        fn confirm(&self) -> Result<(), String> {
            self.service.confirm_retained_operation(
                &self.retained,
                &self.lease,
                self.deadline,
                &self.cancellation,
            )
        }
    }

    #[test]
    fn review_final_confirmation_rejects_root_replacement_during_retained_scan() {
        if !supports_retained_root_replacement_test() {
            return;
        }
        let fixture = RetainedConfirmationFixture::new(|source| {
            fs::write(
                source.join("Configuration.xml"),
                "<Configuration>A</Configuration>",
            )
            .unwrap();
        });
        let replacement = fixture.workspace.path().join("replacement");
        let saved = fixture.workspace.path().join("saved");
        fs::create_dir_all(&replacement).unwrap();
        fs::write(
            replacement.join("Configuration.xml"),
            "<Configuration>B</Configuration>",
        )
        .unwrap();
        let source = fixture.source.clone();
        let _mutation =
            set_retained_scan_test_mutation(RetainedScanTestMutationPoint::ScanStart, move || {
                fs::rename(&source, &saved).unwrap();
                fs::rename(&replacement, &source).unwrap();
            });

        assert!(
            fixture.confirm().is_err(),
            "root replacement during final retained scan must invalidate the operation"
        );
    }

    #[test]
    fn review_final_confirmation_rejects_nested_directory_replacement_after_retention() {
        if !supports_retained_root_replacement_test() {
            return;
        }
        let fixture = RetainedConfirmationFixture::new(|source| {
            fs::create_dir_all(source.join("Catalogs")).unwrap();
            fs::write(source.join("Configuration.xml"), "<Configuration/>").unwrap();
            fs::write(source.join("Catalogs/Items.xml"), "<Catalog>A</Catalog>").unwrap();
        });
        let nested = fixture.source.join("Catalogs");
        let replacement = fixture.workspace.path().join("replacement");
        let saved = fixture.workspace.path().join("saved");
        fs::create_dir_all(&replacement).unwrap();
        fs::write(replacement.join("Items.xml"), "<Catalog>B</Catalog>").unwrap();
        let _mutation = set_retained_scan_test_mutation(
            RetainedScanTestMutationPoint::BeforeDirectoryRecursion,
            move || {
                fs::rename(&nested, &saved).unwrap();
                fs::rename(&replacement, &nested).unwrap();
            },
        );

        assert!(
            fixture.confirm().is_err(),
            "nested directory replacement during final scan must invalidate"
        );
    }

    #[test]
    fn review_final_confirmation_rejects_file_replacement_after_retention() {
        if !supports_retained_root_replacement_test() {
            return;
        }
        let fixture = RetainedConfirmationFixture::new(|source| {
            fs::write(
                source.join("Configuration.xml"),
                "<Configuration>A</Configuration>",
            )
            .unwrap();
        });
        let descriptor = fixture.source.join("Configuration.xml");
        let replacement = fixture.workspace.path().join("replacement.xml");
        let saved = fixture.workspace.path().join("saved.xml");
        fs::write(&replacement, "<Configuration>B</Configuration>").unwrap();
        let _mutation = set_retained_scan_test_mutation(
            RetainedScanTestMutationPoint::BeforeFileHash,
            move || {
                fs::rename(&descriptor, &saved).unwrap();
                fs::rename(&replacement, &descriptor).unwrap();
            },
        );

        assert!(
            fixture.confirm().is_err(),
            "file replacement during final retained scan must invalidate"
        );
    }

    #[test]
    fn review_final_confirmation_rejects_membership_added_after_enumeration() {
        let fixture = RetainedConfirmationFixture::new(|source| {
            fs::write(
                source.join("Configuration.xml"),
                "<Configuration>A</Configuration>",
            )
            .unwrap();
        });
        let added = fixture.source.join("Added.xml");
        let _mutation = set_retained_scan_test_mutation(
            RetainedScanTestMutationPoint::AfterDirectoryEnumeration,
            move || fs::write(&added, "<Added/>").unwrap(),
        );

        assert!(
            fixture.confirm().is_err(),
            "membership added after final enumeration must invalidate"
        );

        let fixture = RetainedConfirmationFixture::new(|source| {
            fs::write(source.join("Configuration.xml"), "<Configuration/>").unwrap();
            fs::write(source.join("Removed.xml"), "<Removed/>").unwrap();
        });
        let removed = fixture.source.join("Removed.xml");
        let _mutation = set_retained_scan_test_mutation(
            RetainedScanTestMutationPoint::AfterDirectoryEnumeration,
            move || fs::remove_file(&removed).unwrap(),
        );
        assert!(
            fixture.confirm().is_err(),
            "membership removed after final enumeration must invalidate"
        );
    }

    #[test]
    fn review_final_confirmation_rejects_in_place_change_after_hash() {
        let fixture = RetainedConfirmationFixture::new(|source| {
            fs::write(
                source.join("Configuration.xml"),
                "<Configuration>A</Configuration>",
            )
            .unwrap();
        });
        let descriptor = fixture.source.join("Configuration.xml");
        let _mutation = set_retained_scan_test_mutation(
            RetainedScanTestMutationPoint::AfterFileHash,
            move || fs::write(&descriptor, "<Configuration>B</Configuration>").unwrap(),
        );

        assert!(
            fixture.confirm().is_err(),
            "in-place change after final hash must invalidate"
        );
    }

    #[test]
    fn retained_scan_limits_entries_files_and_aggregate_bytes() {
        let workspace = tempdir().unwrap();
        let source = workspace.path().join("source");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("one.xml"), b"1234").unwrap();
        fs::write(source.join("two.xml"), b"5678").unwrap();
        let retained = RetainedDirectoryCapability::open(&fs::canonicalize(&source).unwrap())
            .expect("retain source");
        let deadline = ProviderDeadline::from_budget(std::time::Duration::from_secs(5));
        let cancellation = CancellationToken::new();

        let entry_error = scan_retained_source_manifest_with_limits(
            &retained,
            deadline,
            &cancellation,
            RetainedScanLimits::new(1, 16, 32),
        )
        .err()
        .expect("entry limit must fail");
        assert!(entry_error.contains("entry limit"), "{entry_error}");

        let file_error = scan_retained_source_manifest_with_limits(
            &retained,
            deadline,
            &cancellation,
            RetainedScanLimits::new(4, 3, 32),
        )
        .err()
        .expect("file limit must fail");
        assert!(file_error.contains("file byte limit"), "{file_error}");

        let aggregate_error = scan_retained_source_manifest_with_limits(
            &retained,
            deadline,
            &cancellation,
            RetainedScanLimits::new(4, 8, 7),
        )
        .err()
        .expect("aggregate limit must fail");
        assert!(
            aggregate_error.contains("aggregate byte limit"),
            "{aggregate_error}"
        );
    }

    #[test]
    fn retained_scan_resolves_child_name_policy_once_per_directory() {
        let workspace = tempdir().unwrap();
        let source = workspace.path().join("source");
        let nested = source.join("Nested");
        fs::create_dir_all(&nested).unwrap();
        for index in 0..8 {
            fs::write(source.join(format!("Root{index}.xml")), b"root").unwrap();
        }
        for index in 0..6 {
            fs::write(nested.join(format!("Nested{index}.xml")), b"nested").unwrap();
        }
        let generated_alias = nested.join(".BUILD/unica");
        fs::create_dir_all(&generated_alias).unwrap();
        fs::write(generated_alias.join("Forged.bsl"), b"generated").unwrap();
        let retained = RetainedDirectoryCapability::open(&fs::canonicalize(&source).unwrap())
            .expect("retain source");
        let retained_nested = retained
            .retain_directory_child(OsStr::new("Nested"))
            .expect("retain nested source directory");
        let root_identity = retained.identity();
        let nested_identity = retained_nested.identity();
        let _policy = set_retained_directory_case_policy_for_test(move |identity| {
            if identity == root_identity {
                Some(true)
            } else if identity == nested_identity {
                Some(false)
            } else {
                Some(true)
            }
        });

        let manifest = scan_retained_source_manifest_with_limits(
            &retained,
            ProviderDeadline::from_budget(std::time::Duration::from_secs(5)),
            &CancellationToken::new(),
            RetainedScanLimits::new(32, 32, 1024),
        )
        .expect("fixture scan");

        assert!(
            manifest
                .keys()
                .all(|path| !path.starts_with("Nested/.BUILD")),
            "the nested parent policy did not prune its generated-name identity"
        );
        assert_eq!(
            retained_directory_case_policy_queries_for_test(),
            2,
            "one native child-name policy lookup is allowed per scanned retained directory"
        );
    }

    #[test]
    fn retained_file_hashing_checks_cancellation_between_bounded_chunks() {
        let workspace = tempdir().unwrap();
        let source = workspace.path().join("source");
        fs::create_dir_all(&source).unwrap();
        fs::write(
            source.join("Module.bsl"),
            vec![b'x'; RETAINED_HASH_CHUNK_BYTES * 3],
        )
        .unwrap();
        let retained = RetainedDirectoryCapability::open(&fs::canonicalize(&source).unwrap())
            .expect("retain source");
        let RetainedChildCapability::RegularFile(file) = retained
            .retain_immediate_child_nofollow(OsStr::new("Module.bsl"))
            .unwrap()
        else {
            panic!("fixture file must be retained")
        };
        let mut state = RetainedScanState::default();
        let mut checkpoints = 0;
        let error = hash_retained_source_file_with_checkpoint(
            &file,
            Path::new("Module.bsl"),
            RetainedScanLimits::new(4, u64::MAX, u64::MAX),
            &mut state,
            &mut || {
                checkpoints += 1;
                if checkpoints == 3 {
                    Err(RetainedRevisionError::new(
                        RetainedRevisionErrorKind::Cancelled,
                        "cancelled between chunks",
                    ))
                } else {
                    Ok(())
                }
            },
        )
        .unwrap_err();
        assert_eq!(error.kind(), RetainedRevisionErrorKind::Cancelled);
        assert_eq!(error.to_string(), "cancelled between chunks");
        assert_eq!(checkpoints, 3);
        assert_eq!(state.total_bytes, (RETAINED_HASH_CHUNK_BYTES * 2) as u64);
    }

    #[test]
    pub(crate) fn retained_revision_authority_contract_is_complete() {
        retained_snapshot_rejects_a_capability_from_another_source_identity();
        retained_snapshot_reuses_a_clean_fence_and_reconciles_once_after_change();
        retained_manifest_uses_the_existing_source_digest_algorithm();
        review_retained_manifest_cannot_satisfy_an_ambient_fast_path_after_root_swap();
        unsupported_fence_stable_operation_lease_scans_at_admission_and_confirmation();
        unsupported_fence_reconcile_is_bounded_to_six_passes_when_corpus_never_stabilizes();
        retained_final_confirmation_stabilization_contract_is_complete();
        retained_scan_limits_entries_files_and_aggregate_bytes();
        retained_file_hashing_checks_cancellation_between_bounded_chunks();
        if supports_retained_root_replacement_test() {
            ambient_manifest_cannot_satisfy_a_retained_fast_path();
            retained_snapshot_never_mixes_a_replaced_root_name_with_the_open_tree();
        }
    }

    #[test]
    pub(crate) fn retained_final_confirmation_stabilization_contract_is_complete() {
        review_final_confirmation_rejects_root_replacement_during_retained_scan();
        review_final_confirmation_rejects_nested_directory_replacement_after_retention();
        review_final_confirmation_rejects_file_replacement_after_retention();
        review_final_confirmation_rejects_membership_added_after_enumeration();
        review_final_confirmation_rejects_in_place_change_after_hash();
        unsupported_fence_stable_operation_lease_scans_at_admission_and_confirmation();
        unsupported_fence_reconcile_is_bounded_to_six_passes_when_corpus_never_stabilizes();
    }
}
