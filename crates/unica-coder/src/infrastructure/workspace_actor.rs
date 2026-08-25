use crate::application::shared_work::{
    LongWorkFailure, SharedWork, SharedWorkKey, SharedWorkLease, SharedWorkLifetime,
    SharedWorkProducer,
};
use crate::domain::cancellation::CancellationToken;
use crate::domain::code_intelligence::ProviderDeadline;
use crate::domain::invocation::SafeIdentityHash;
use crate::domain::source_revision::SourceRevision;
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::deadline_lock::{DeadlineLock, FailClosed};
use crate::infrastructure::native_operations::apply::ApplyStagedState;
use crate::infrastructure::native_operations::compile_transaction::CompileTransaction;
use crate::infrastructure::platform::filesystem::{
    path_starts_with_host_root, stable_path_identity_bytes, RetainedDirectoryCapability,
};
use crate::infrastructure::source_revision::{
    RetainedRevisionLease, SourceRevisionService, WorkspaceStateScope,
};
use crate::infrastructure::source_roots::normalize_path_identity;
use crate::infrastructure::workspace_index::{IndexRunner, WorkspaceIndexService};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::Duration;

#[cfg(test)]
thread_local! {
    static LOGICAL_PUBLICATION_AFTER_CONFIRMATION_HOOK: std::cell::RefCell<
        Option<Box<dyn FnOnce()>>,
    > = std::cell::RefCell::new(None);
    static APPLY_DRY_RUN_AFTER_CONFIRMATION_HOOK: std::cell::RefCell<
        Option<Box<dyn FnOnce()>>,
    > = std::cell::RefCell::new(None);
}

#[cfg(test)]
pub(crate) fn set_logical_publication_after_confirmation_hook(hook: impl FnOnce() + 'static) {
    LOGICAL_PUBLICATION_AFTER_CONFIRMATION_HOOK.with(|slot| {
        *slot.borrow_mut() = Some(Box::new(hook));
    });
}

#[cfg(test)]
fn run_logical_publication_after_confirmation_hook() {
    LOGICAL_PUBLICATION_AFTER_CONFIRMATION_HOOK.with(|slot| {
        if let Some(hook) = slot.borrow_mut().take() {
            hook();
        }
    });
}

#[cfg(test)]
fn set_apply_dry_run_after_confirmation_hook(hook: impl FnOnce() + 'static) {
    APPLY_DRY_RUN_AFTER_CONFIRMATION_HOOK.with(|slot| {
        *slot.borrow_mut() = Some(Box::new(hook));
    });
}

#[cfg(test)]
fn run_apply_dry_run_after_confirmation_hook() {
    APPLY_DRY_RUN_AFTER_CONFIRMATION_HOOK.with(|slot| {
        if let Some(hook) = slot.borrow_mut().take() {
            hook();
        }
    });
}

pub(crate) const MAX_ACTIVE_WORKSPACE_ACTORS: usize = 64;
const INDEX_FENCE_BUDGET: Duration = Duration::from_secs(7);
static APPLY_WRITER_AUTHORITY_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct IndexWorkIdentity([u8; 32]);

macro_rules! assert_not_impl_production {
    ($type:ty: $trait:path) => {
        const _: fn() = || {
            trait AmbiguousIfImpl<Marker> {
                fn check() {}
            }
            struct ImplementsTrait;
            impl<T: ?Sized> AmbiguousIfImpl<()> for T {}
            impl<T: ?Sized + $trait> AmbiguousIfImpl<ImplementsTrait> for T {}
            let _ = <$type as AmbiguousIfImpl<_>>::check;
        };
    };
}

/// Exact daemon-local ownership key for mutable workspace state.
///
/// A Git repository is intentionally absent. Linked worktrees have separate
/// canonical roots, revisions and publication lanes even when `.git` points at
/// the same repository metadata.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct WorkspaceIdentity {
    workspace_root: PathBuf,
    source_sets: Vec<WorkspaceSourceSetIdentity>,
    provider_profile: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct WorkspaceSourceSetIdentity {
    name: String,
    root: PathBuf,
}

impl WorkspaceIdentity {
    pub(crate) fn new<I, N, P>(
        context: &WorkspaceContext,
        source_sets: I,
        provider_profile: &str,
    ) -> Result<Self, String>
    where
        I: IntoIterator<Item = (N, P)>,
        N: AsRef<str>,
        P: AsRef<Path>,
    {
        if provider_profile.trim().is_empty() || provider_profile.chars().any(char::is_control) {
            return Err("workspace provider profile must be non-empty text".to_string());
        }
        let workspace_root = normalize_path_identity(&context.workspace_root)?;
        let mut source_sets = source_sets
            .into_iter()
            .map(|(name, root)| {
                let name = name.as_ref();
                if name.trim().is_empty() || name.chars().any(char::is_control) {
                    return Err("workspace source-set name must be non-empty text".to_string());
                }
                Ok(WorkspaceSourceSetIdentity {
                    name: name.to_string(),
                    root: normalize_path_identity(root.as_ref())?,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        if source_sets.is_empty() {
            return Err("workspace actor requires at least one source-set root".to_string());
        }
        if let Some(outside) = source_sets
            .iter()
            .find(|source_set| !path_starts_with_host_root(&source_set.root, &workspace_root))
        {
            return Err(format!(
                "workspace actor source-set root is outside its canonical workspace root: {}",
                outside.root.display()
            ));
        }
        source_sets.sort();
        if source_sets
            .windows(2)
            .any(|pair| pair[0].name == pair[1].name)
        {
            return Err("workspace actor source-set names must be unique".to_string());
        }
        let mut physical_roots = HashSet::new();
        if source_sets
            .iter()
            .any(|source_set| !physical_roots.insert(source_set.root.clone()))
        {
            return Err(
                "workspace actor source-set roots must be physically unambiguous".to_string(),
            );
        }
        Ok(Self {
            workspace_root,
            source_sets,
            provider_profile: provider_profile.to_string(),
        })
    }

    pub(crate) fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }

    pub(crate) fn source_sets(&self) -> &[WorkspaceSourceSetIdentity] {
        &self.source_sets
    }

    pub(crate) fn provider_profile(&self) -> &str {
        &self.provider_profile
    }

    fn state_scope_hash(&self) -> Result<[u8; 32], String> {
        let mut digest = Sha256::new();
        digest.update(b"unica-workspace-actor-state-v1\0");
        update_digest_path(&mut digest, &self.workspace_root)?;
        digest.update((self.source_sets.len() as u64).to_le_bytes());
        for source_set in &self.source_sets {
            update_digest_text(&mut digest, &source_set.name);
            update_digest_path(&mut digest, &source_set.root)?;
        }
        update_digest_text(&mut digest, &self.provider_profile);
        Ok(digest.finalize().into())
    }

    fn state_scope_digest(&self) -> Result<String, String> {
        use std::fmt::Write as _;

        let mut encoded = String::with_capacity(64);
        for byte in self.state_scope_hash()? {
            write!(&mut encoded, "{byte:02x}").expect("writing to String cannot fail");
        }
        Ok(encoded)
    }
}

fn update_digest_text(digest: &mut Sha256, value: &str) {
    digest.update((value.len() as u64).to_le_bytes());
    digest.update(value.as_bytes());
}

fn update_digest_path(digest: &mut Sha256, value: &Path) -> Result<(), String> {
    let bytes = stable_path_identity_bytes(value)?;
    digest.update((bytes.len() as u64).to_le_bytes());
    digest.update(bytes);
    Ok(())
}

#[derive(Clone, PartialEq, Eq)]
struct ActorInstanceId(uuid::Uuid);

impl ActorInstanceId {
    fn new() -> Self {
        Self(uuid::Uuid::new_v4())
    }
}

#[derive(Clone)]
pub(crate) struct ProviderRootBinding {
    actor_identity: WorkspaceIdentity,
    actor_instance: ActorInstanceId,
    source_set: WorkspaceSourceSetIdentity,
    source_root: Arc<RetainedDirectoryCapability>,
}

impl std::fmt::Debug for ProviderRootBinding {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProviderRootBinding")
            .field("actor_identity", &self.actor_identity)
            .field("source_set", &self.source_set)
            .field("source_root", &self.source_root.path())
            .finish_non_exhaustive()
    }
}

impl ProviderRootBinding {
    pub(super) fn source_root(&self) -> &Path {
        self.source_root.path()
    }

    pub(super) fn retained_root(&self) -> Arc<RetainedDirectoryCapability> {
        Arc::clone(&self.source_root)
    }
}

/// Unforgeable admission token for the retained apply publisher. Low-level
/// transaction methods require the same token that created the staged state;
/// infrastructure callers can name the type but cannot construct a value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::infrastructure) struct ApplyWriterAuthority(u64);

impl ApplyWriterAuthority {
    fn issue() -> Self {
        Self(APPLY_WRITER_AUTHORITY_SEQUENCE.fetch_add(1, Ordering::Relaxed))
    }
}

#[cfg(test)]
pub(in crate::infrastructure) fn apply_writer_authority_for_test() -> ApplyWriterAuthority {
    ApplyWriterAuthority::issue()
}

#[derive(Clone)]
pub(crate) struct WorkspaceRevisionFence {
    actor_identity: WorkspaceIdentity,
    actor_instance: ActorInstanceId,
    source_set: WorkspaceSourceSetIdentity,
    revision: SourceRevision,
}

/// Actor-issued retained revision authority for one V13 logical source set.
/// The caller can copy its revision identity into typed readers, but cannot
/// substitute a root, revision service or actor identity at publication.
#[derive(Clone)]
pub(crate) struct WorkspaceLogicalReadFence {
    actor_identity: WorkspaceIdentity,
    actor_instance: ActorInstanceId,
    source_set: WorkspaceSourceSetIdentity,
    revision: RetainedRevisionLease,
}

impl WorkspaceLogicalReadFence {
    pub(crate) fn revision(&self) -> RetainedRevisionLease {
        self.revision.clone()
    }
}

/// Actor-issued authority for planning exactly one hidden-v0.13 apply batch.
/// All identity, root, revision, deadline and cancellation fields are closed;
/// callers can stage bytes but cannot substitute publication authority.
pub(crate) struct ApplyAdmission {
    actor_identity: WorkspaceIdentity,
    actor_instance: ActorInstanceId,
    source_set: WorkspaceSourceSetIdentity,
    source_root: Arc<RetainedDirectoryCapability>,
    revision_service: Arc<SourceRevisionService>,
    revision: RetainedRevisionLease,
    dry_run: bool,
    deadline: ProviderDeadline,
    cancellation: CancellationToken,
    writer_authority: ApplyWriterAuthority,
}

impl std::fmt::Debug for ApplyAdmission {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ApplyAdmission")
            .field("source_set", &self.source_set)
            .field("dry_run", &self.dry_run)
            .finish_non_exhaustive()
    }
}

impl ApplyAdmission {
    pub(crate) fn revision_identity(&self) -> String {
        self.revision.revision_identity()
    }

    pub(crate) fn staged_state(&self) -> Result<ApplyStagedState, String> {
        apply_checkpoint(self.deadline, &self.cancellation, "apply planning")?;
        Ok(ApplyStagedState::from_retained_root(
            Arc::clone(&self.source_root),
            self.deadline,
            self.cancellation.clone(),
            self.writer_authority.clone(),
        ))
    }

    pub(crate) fn prepare(self, state: ApplyStagedState) -> Result<PreparedApplyBatch, String> {
        apply_checkpoint(self.deadline, &self.cancellation, "apply preparation")?;
        if state.retained_root_identity() != self.source_root.identity() {
            return Err("apply staged state belongs to another retained source root".to_string());
        }
        if !state.has_writer_authority(&self.writer_authority) {
            return Err("apply staged state belongs to another actor-issued authority".to_string());
        }
        let transaction = state.finalize()?;
        Ok(PreparedApplyBatch {
            actor_identity: self.actor_identity,
            actor_instance: self.actor_instance,
            source_set: self.source_set,
            source_root: self.source_root,
            revision_service: self.revision_service,
            revision: self.revision,
            dry_run: self.dry_run,
            deadline: self.deadline,
            cancellation: self.cancellation,
            transaction,
            writer_authority: self.writer_authority,
        })
    }
}

pub(crate) struct PreparedApplyBatch {
    actor_identity: WorkspaceIdentity,
    actor_instance: ActorInstanceId,
    source_set: WorkspaceSourceSetIdentity,
    source_root: Arc<RetainedDirectoryCapability>,
    revision_service: Arc<SourceRevisionService>,
    revision: RetainedRevisionLease,
    dry_run: bool,
    deadline: ProviderDeadline,
    cancellation: CancellationToken,
    transaction: CompileTransaction,
    writer_authority: ApplyWriterAuthority,
}

impl std::fmt::Debug for PreparedApplyBatch {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PreparedApplyBatch")
            .field("source_set", &self.source_set)
            .field("dry_run", &self.dry_run)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ApplyCleanupDiagnosticKind {
    RetainedRecoveryCleanupIncomplete,
}

/// Bounded actor-facing cleanup context. The target stays relative to the
/// selected source set and the artifact is only the fixed-format internal leaf;
/// neither field exposes the retained provider root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ApplyCleanupDiagnostic {
    kind: ApplyCleanupDiagnosticKind,
    logical_target: PathBuf,
    last_known_artifact_name: OsString,
}

impl ApplyCleanupDiagnostic {
    pub(crate) const fn kind(&self) -> ApplyCleanupDiagnosticKind {
        self.kind
    }

    pub(crate) fn logical_target(&self) -> &Path {
        &self.logical_target
    }

    /// The transaction-generated recovery leaf before cleanup began. A
    /// concurrent namespace mutation may mean it no longer names the retained
    /// artifact, so callers must not treat this diagnostic as delete authority.
    pub(crate) fn last_known_artifact_name(&self) -> &OsStr {
        &self.last_known_artifact_name
    }
}

#[derive(Debug)]
pub(crate) struct ApplyPublicationResult {
    rev: String,
    commit_count: usize,
    cleanup_diagnostics: Vec<ApplyCleanupDiagnostic>,
}

impl ApplyPublicationResult {
    pub(crate) fn rev(&self) -> &str {
        &self.rev
    }

    pub(crate) fn cleanup_diagnostics(&self) -> &[ApplyCleanupDiagnostic] {
        &self.cleanup_diagnostics
    }

    #[cfg(test)]
    pub(crate) const fn commit_count_for_test(&self) -> usize {
        self.commit_count
    }
}

/// Exclusive, revision-fenced permission to make one staged mutation result
/// observable.  It intentionally exposes neither an ambient source-root path
/// nor a generic publication callback.  Task 15 can add descriptor-relative
/// writer operations to this capability when writers are routed through the
/// actor; until then there is no production validate-then-unchecked escape.
pub(crate) struct WorkspacePublicationLease<'actor, R> {
    actor: &'actor WorkspaceActor<R>,
    binding: ProviderRootBinding,
    issued_revision: SourceRevision,
    revision_service: Arc<SourceRevisionService>,
    _lane: MutexGuard<'actor, ()>,
}

/// Actor-owned runtime payloads expose only a payload-defined projection.
///
/// This prevents the generic actor from returning `&R` in production while
/// still allowing a compatibility adapter to define a module-private view of
/// the state it must bridge during migration.
pub(super) trait WorkspaceActorRuntimeProjection {
    type Projection<'a>
    where
        Self: 'a;

    fn project_for_actor(&self) -> Self::Projection<'_>;
}

#[cfg(test)]
pub(super) trait WorkspaceActorRuntimeTestProjection {
    type ProjectionMut<'a>
    where
        Self: 'a;

    fn project_mut_for_actor_test(&mut self) -> Self::ProjectionMut<'_>;
}

impl<R> std::fmt::Debug for WorkspacePublicationLease<'_, R> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("WorkspacePublicationLease")
            .field("binding", &self.binding)
            .finish_non_exhaustive()
    }
}

impl<R> WorkspacePublicationLease<'_, R> {
    pub(crate) fn publish<T>(
        self,
        staged_result: T,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> Result<T, String> {
        self.actor.validate_binding(&self.binding)?;
        let current = self.revision_service.snapshot(deadline, cancellation);
        self.actor.validate_binding(&self.binding)?;
        let current = current?;
        if current != self.issued_revision {
            return Err("source revision changed before staged result publication".to_string());
        }
        Ok(staged_result)
    }
}

/// One daemon-owned coordination boundary for a canonical worktree.
///
/// Reads do not take the mutation lane. Read-only staged-result confirmation
/// is exclusive and rechecks the actor-owned source revision immediately
/// before that result is returned. Task 15 adds the writer boundary.
pub(crate) struct WorkspaceActor<R = ()> {
    identity: WorkspaceIdentity,
    instance_id: ActorInstanceId,
    context: WorkspaceContext,
    source_roots: HashMap<WorkspaceSourceSetIdentity, Arc<RetainedDirectoryCapability>>,
    state_scope: WorkspaceStateScope,
    mutation_lane: DeadlineLock<FailClosed>,
    source_revisions: Mutex<HashMap<WorkspaceSourceSetIdentity, Arc<SourceRevisionService>>>,
    index_work: SharedWork<(), LongWorkFailure>,
    runtime: R,
}

assert_not_impl_production!(WorkspaceActor<()>: std::ops::Deref);
assert_not_impl_production!(WorkspaceActor<()>: std::ops::DerefMut);

impl<R> std::fmt::Debug for WorkspaceActor<R> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("WorkspaceActor")
            .field("identity", &self.identity)
            .finish_non_exhaustive()
    }
}

impl WorkspaceActor<()> {
    pub(crate) fn new(
        identity: WorkspaceIdentity,
        context: WorkspaceContext,
    ) -> Result<Self, String> {
        Self::with_runtime(identity, context, ())
    }
}

impl<R> WorkspaceActor<R> {
    pub(crate) fn with_runtime(
        identity: WorkspaceIdentity,
        context: WorkspaceContext,
        runtime: R,
    ) -> Result<Self, String> {
        let state_scope = WorkspaceStateScope::scoped_sha256(identity.state_scope_digest()?)?;
        Self::with_runtime_scope(identity, context, runtime, state_scope)
    }

    pub(crate) fn with_legacy_runtime(
        identity: WorkspaceIdentity,
        context: WorkspaceContext,
        runtime: R,
    ) -> Result<Self, String> {
        Self::with_runtime_scope(
            identity,
            context,
            runtime,
            WorkspaceStateScope::LegacyPhysical,
        )
    }

    fn with_runtime_scope(
        identity: WorkspaceIdentity,
        mut context: WorkspaceContext,
        runtime: R,
        state_scope: WorkspaceStateScope,
    ) -> Result<Self, String> {
        let context_root = normalize_path_identity(&context.workspace_root)?;
        if context_root != identity.workspace_root {
            return Err(
                "workspace actor context does not match its canonical identity".to_string(),
            );
        }
        let mut source_roots = HashMap::new();
        let mut physical_identities = HashSet::new();
        for source_set in &identity.source_sets {
            let capability = RetainedDirectoryCapability::open(&source_set.root).map_err(|error| {
                format!(
                    "workspace actor source-set root cannot be retained without following links: {}: {error}",
                    source_set.root.display()
                )
            })?;
            if !physical_identities.insert(capability.identity()) {
                return Err(
                    "workspace actor source-set roots resolve to one ambiguous physical directory"
                        .to_string(),
                );
            }
            source_roots.insert(source_set.clone(), Arc::new(capability));
        }
        context.workspace_root = context_root.clone();
        context.cwd = context_root;
        Ok(Self {
            identity,
            instance_id: ActorInstanceId::new(),
            context,
            source_roots,
            state_scope,
            mutation_lane: DeadlineLock::fail_closed("workspace actor mutation lane is poisoned"),
            source_revisions: Mutex::new(HashMap::new()),
            index_work: SharedWork::new(SharedWorkLifetime::ProducerBound),
            runtime,
        })
    }

    pub(crate) fn identity(&self) -> &WorkspaceIdentity {
        &self.identity
    }

    pub(crate) fn workspace_identity(&self) -> &WorkspaceIdentity {
        &self.identity
    }

    /// Closed daemon-safe identity for task persistence. It is derived from
    /// the actor's complete structural identity; no caller-provided path or
    /// repository label can forge it.
    pub(crate) fn safe_identity_hash(&self) -> Result<SafeIdentityHash, String> {
        self.identity
            .state_scope_hash()
            .map(SafeIdentityHash::from_sha256)
    }

    pub(super) fn runtime_projection(&self) -> R::Projection<'_>
    where
        R: WorkspaceActorRuntimeProjection,
    {
        self.runtime.project_for_actor()
    }

    #[cfg(test)]
    pub(super) fn runtime_projection_mut_for_test(&mut self) -> R::ProjectionMut<'_>
    where
        R: WorkspaceActorRuntimeTestProjection,
    {
        self.runtime.project_mut_for_actor_test()
    }

    pub(crate) fn context(&self) -> &WorkspaceContext {
        &self.context
    }

    pub(crate) fn bind_provider_root(
        &self,
        source_set_name: &str,
        requested_root: &Path,
    ) -> Result<ProviderRootBinding, String> {
        let requested_root = normalize_path_identity(requested_root)?;
        let source_set = self
            .identity
            .source_sets
            .iter()
            .find(|source_set| {
                source_set.name == source_set_name && source_set.root == requested_root
            })
            .cloned();
        let Some(source_set) = source_set else {
            return Err(format!(
                "provider root is not bound to source set `{source_set_name}` in this workspace actor: {}",
                requested_root.display()
            ));
        };
        let source_root = self
            .source_roots
            .get(&source_set)
            .cloned()
            .ok_or_else(|| "workspace actor retained root is unavailable".to_string())?;
        validate_physical_root(&source_root)?;
        Ok(ProviderRootBinding {
            actor_identity: self.identity.clone(),
            actor_instance: self.instance_id.clone(),
            source_set,
            source_root,
        })
    }

    pub(crate) fn read<T>(
        &self,
        binding: &ProviderRootBinding,
        read: impl FnOnce(&Path) -> Result<T, String>,
    ) -> Result<T, String> {
        self.validate_binding(binding)?;
        let result = read(binding.source_root.path());
        self.validate_binding(binding)?;
        result
    }

    pub(crate) fn read_relative_file(
        &self,
        binding: &ProviderRootBinding,
        relative: &Path,
        max_bytes: usize,
    ) -> Result<Vec<u8>, String> {
        self.validate_binding(binding)?;
        let result = binding
            .source_root
            .read_relative_regular_bounded(relative, max_bytes)
            .map_err(|error| format!("actor-bound relative read failed: {error}"));
        self.validate_binding(binding)?;
        result
    }

    pub(crate) fn capture_revision(
        &self,
        binding: &ProviderRootBinding,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> Result<WorkspaceRevisionFence, String> {
        self.validate_binding(binding)?;
        let revision = self
            .source_revision_service(binding)
            .and_then(|service| service.snapshot(deadline, cancellation));
        self.validate_binding(binding)?;
        let revision = revision?;
        Ok(WorkspaceRevisionFence {
            actor_identity: self.identity.clone(),
            actor_instance: self.instance_id.clone(),
            source_set: binding.source_set.clone(),
            revision,
        })
    }

    pub(crate) fn capture_logical_read_revision(
        &self,
        binding: &ProviderRootBinding,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> Result<WorkspaceLogicalReadFence, String> {
        self.validate_binding(binding)?;
        let revision = self
            .source_revision_service(binding)?
            .begin_retained_operation(&binding.source_root, deadline, cancellation);
        self.validate_binding(binding)?;
        Ok(WorkspaceLogicalReadFence {
            actor_identity: self.identity.clone(),
            actor_instance: self.instance_id.clone(),
            source_set: binding.source_set.clone(),
            revision: revision?,
        })
    }

    pub(crate) fn admit_apply(
        &self,
        binding: &ProviderRootBinding,
        if_rev: Option<&str>,
        dry_run: bool,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> Result<ApplyAdmission, String> {
        apply_checkpoint(deadline, cancellation, "apply admission")?;
        self.validate_binding(binding)?;
        let revision_service = self.source_revision_service(binding)?;
        let revision = revision_service.begin_retained_operation(
            &binding.source_root,
            deadline,
            cancellation,
        )?;
        self.validate_binding(binding)?;
        let revision_identity = revision.revision_identity();
        if if_rev.is_some_and(|expected| expected != revision_identity) {
            return Err(format!(
                "apply ifRev is stale: expected {}, admitted {revision_identity}",
                if_rev.unwrap_or_default()
            ));
        }
        Ok(ApplyAdmission {
            actor_identity: self.identity.clone(),
            actor_instance: self.instance_id.clone(),
            source_set: binding.source_set.clone(),
            source_root: Arc::clone(&binding.source_root),
            revision_service,
            revision,
            dry_run,
            deadline,
            cancellation: cancellation.clone(),
            writer_authority: ApplyWriterAuthority::issue(),
        })
    }

    pub(crate) fn publish_prepared_apply(
        &self,
        prepared: PreparedApplyBatch,
    ) -> Result<ApplyPublicationResult, String> {
        if prepared.actor_identity != self.identity
            || prepared.actor_instance != self.instance_id
            || !self.identity.source_sets.contains(&prepared.source_set)
        {
            return Err("prepared apply batch belongs to another workspace actor".to_string());
        }
        apply_checkpoint(
            prepared.deadline,
            &prepared.cancellation,
            "prepared apply publication",
        )?;
        let _lane = self.mutation_lane.acquire_before(
            prepared.deadline,
            &prepared.cancellation,
            "workspace actor prepared apply wait",
        )?;
        let binding = ProviderRootBinding {
            actor_identity: prepared.actor_identity.clone(),
            actor_instance: prepared.actor_instance.clone(),
            source_set: prepared.source_set.clone(),
            source_root: Arc::clone(&prepared.source_root),
        };
        self.validate_binding(&binding)?;
        prepared.revision_service.confirm_retained_operation(
            &prepared.source_root,
            &prepared.revision,
            prepared.deadline,
            &prepared.cancellation,
        )?;
        if prepared.dry_run {
            prepared.transaction.validate_retained_for_apply()?;
            prepared.revision_service.confirm_retained_operation(
                &prepared.source_root,
                &prepared.revision,
                prepared.deadline,
                &prepared.cancellation,
            )?;
            #[cfg(test)]
            run_apply_dry_run_after_confirmation_hook();
            self.validate_binding(&binding)?;
            apply_checkpoint(
                prepared.deadline,
                &prepared.cancellation,
                "prepared apply result",
            )?;
            return Ok(ApplyPublicationResult {
                rev: prepared.revision.revision_identity(),
                commit_count: 0,
                cleanup_diagnostics: Vec::new(),
            });
        }

        let deadline = prepared.deadline;
        let cancellation = prepared.cancellation.clone();
        let root = Arc::clone(&prepared.source_root);
        let revisions = Arc::clone(&prepared.revision_service);
        let actor = self;
        let (report, revision) = prepared.transaction.commit_retained_apply_with(
            prepared.writer_authority,
            || apply_checkpoint(deadline, &cancellation, "prepared apply commit"),
            || {
                apply_checkpoint(deadline, &cancellation, "apply revision reconciliation")?;
                revisions.mark_dirty();
                let revision = revisions.snapshot_retained(&root, deadline, &cancellation)?;
                actor.validate_binding(&binding)?;
                apply_checkpoint(deadline, &cancellation, "prepared apply result")?;
                Ok(revision)
            },
        )?;
        debug_assert_eq!(
            report.cleanup_warnings.len(),
            report.retained_apply_cleanup_diagnostics.len(),
            "every retained apply cleanup warning must have structured actor context"
        );
        let cleanup_diagnostics = report
            .retained_apply_cleanup_diagnostics
            .into_iter()
            .map(|diagnostic| {
                let (logical_target, last_known_artifact_name) = diagnostic.into_parts();
                ApplyCleanupDiagnostic {
                    kind: ApplyCleanupDiagnosticKind::RetainedRecoveryCleanupIncomplete,
                    logical_target,
                    last_known_artifact_name,
                }
            })
            .collect();
        Ok(ApplyPublicationResult {
            rev: format!(
                "{}:{}:{}",
                revision.algorithm, revision.generation, revision.digest
            ),
            commit_count: 1,
            cleanup_diagnostics,
        })
    }

    /// Makes one staged logical-read result observable while holding the
    /// actor's single mutation lane across validation of every selected source
    /// and every retained final confirmation.
    pub(crate) fn publish_logical_read<T>(
        &self,
        fences: &[WorkspaceLogicalReadFence],
        staged_result: T,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> Result<T, String> {
        if fences.is_empty() {
            return Err("logical read publication requires an admitted source fence".to_string());
        }
        let _publication = self.mutation_lane.acquire_before(
            deadline,
            cancellation,
            "workspace actor logical read publication wait",
        )?;
        let mut confirmations = Vec::with_capacity(fences.len());
        let mut source_sets = HashSet::with_capacity(fences.len());
        for fence in fences {
            if fence.actor_identity != self.identity
                || fence.actor_instance != self.instance_id
                || !source_sets.insert(fence.source_set.clone())
            {
                return Err(
                    "logical read revision fence belongs to another or duplicate workspace source"
                        .to_string(),
                );
            }
            let source_root = self
                .source_roots
                .get(&fence.source_set)
                .cloned()
                .ok_or_else(|| "workspace actor retained root is unavailable".to_string())?;
            let binding = ProviderRootBinding {
                actor_identity: fence.actor_identity.clone(),
                actor_instance: fence.actor_instance.clone(),
                source_set: fence.source_set.clone(),
                source_root,
            };
            self.validate_binding(&binding)?;
            confirmations.push((
                self.source_revision_service(&binding)?,
                binding,
                &fence.revision,
            ));
        }
        for (revisions, binding, revision) in confirmations {
            revisions.confirm_retained_operation(
                &binding.source_root,
                revision,
                deadline,
                cancellation,
            )?;
            #[cfg(test)]
            run_logical_publication_after_confirmation_hook();
        }
        Ok(staged_result)
    }

    /// Exact actor-owned index readiness identity. The workspace component is
    /// derived from this actor's complete canonical identity; the source-set
    /// and revision come from capabilities issued by this same actor instance.
    pub(crate) fn join_index_work<W>(
        &self,
        binding: &ProviderRootBinding,
        fence: &WorkspaceRevisionFence,
        provider: &str,
        profile: &str,
        work: W,
    ) -> Result<(IndexWorkIdentity, SharedWorkLease<(), LongWorkFailure>), String>
    where
        W: FnOnce(SharedWorkProducer) -> Result<(), LongWorkFailure> + Send + 'static,
    {
        self.validate_binding(binding)?;
        if fence.actor_identity != self.identity
            || fence.actor_instance != self.instance_id
            || fence.source_set != binding.source_set
        {
            return Err("index revision fence belongs to another workspace actor".to_string());
        }
        if !closed_index_component(provider)
            || !closed_index_component(profile)
            || fence.revision.algorithm != crate::domain::source_revision::SOURCE_REVISION_ALGORITHM
            || fence.revision.generation == 0
            || !is_lowercase_sha256(&fence.revision.digest)
        {
            return Err("actor-owned index work identity is invalid".to_string());
        }
        let revision_service = self.source_revision_service(binding)?;
        let current = revision_service.snapshot(
            ProviderDeadline::from_budget(INDEX_FENCE_BUDGET),
            &CancellationToken::new(),
        );
        self.validate_binding(binding)?;
        if current? != fence.revision {
            return Err("index revision fence changed before shared-work admission".to_string());
        }

        let mut digest = Sha256::new();
        digest.update(b"unica-index-work-v2\0");
        digest.update(self.identity.state_scope_digest()?);
        digest_index_component(&mut digest, binding.source_set.name.as_bytes())?;
        digest_index_component(&mut digest, fence.revision.algorithm.as_bytes())?;
        digest.update(fence.revision.generation.to_le_bytes());
        digest_index_component(&mut digest, fence.revision.digest.as_bytes())?;
        digest_index_component(&mut digest, provider.as_bytes())?;
        digest_index_component(&mut digest, profile.as_bytes())?;
        let identity = IndexWorkIdentity(digest.finalize().into());

        let retained_root = Arc::clone(&binding.source_root);
        let expected_revision = fence.revision.clone();
        let producer_revision_service = Arc::clone(&revision_service);
        let lease = self.index_work.join_or_start(
            SharedWorkKey::Index {
                identity: identity.0,
            },
            move |producer| {
                retained_root
                    .validate_named_identity()
                    .map_err(|_| LongWorkFailure::Invalidated)?;
                let current = producer_revision_service
                    .snapshot(
                        ProviderDeadline::from_budget(INDEX_FENCE_BUDGET),
                        &CancellationToken::new(),
                    )
                    .map_err(|_| LongWorkFailure::Invalidated)?;
                retained_root
                    .validate_named_identity()
                    .map_err(|_| LongWorkFailure::Invalidated)?;
                if current != expected_revision {
                    return Err(LongWorkFailure::Invalidated);
                }
                work(producer)
            },
        );
        Ok((identity, lease))
    }

    pub(crate) fn begin_publication(
        &self,
        fence: &WorkspaceRevisionFence,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> Result<WorkspacePublicationLease<'_, R>, String> {
        let publication = self.mutation_lane.acquire_before(
            deadline,
            cancellation,
            "workspace actor mutation lane wait",
        )?;
        if fence.actor_identity != self.identity
            || fence.actor_instance != self.instance_id
            || !self.identity.source_sets.contains(&fence.source_set)
        {
            return Err("source revision fence belongs to another workspace actor".to_string());
        }
        let binding = ProviderRootBinding {
            actor_identity: fence.actor_identity.clone(),
            actor_instance: fence.actor_instance.clone(),
            source_root: self
                .source_roots
                .get(&fence.source_set)
                .cloned()
                .ok_or_else(|| "workspace actor retained root is unavailable".to_string())?,
            source_set: fence.source_set.clone(),
        };
        self.validate_binding(&binding)?;
        let revision_service = self.source_revision_service(&binding)?;
        let current = revision_service.snapshot(deadline, cancellation);
        self.validate_binding(&binding)?;
        let current = current?;
        if current != fence.revision {
            return Err("source revision changed before publication".to_string());
        }
        Ok(WorkspacePublicationLease {
            actor: self,
            binding,
            issued_revision: fence.revision.clone(),
            revision_service,
            _lane: publication,
        })
    }

    pub(crate) fn index_service<'a>(
        &self,
        binding: &ProviderRootBinding,
        runner: &'a dyn IndexRunner,
    ) -> Result<WorkspaceIndexService<'a>, String> {
        self.validate_binding(binding)?;
        Ok(WorkspaceIndexService::with_runner(runner)
            .with_source_revision_service(self.source_revision_service(binding)?)
            .with_bound_source_root(Arc::clone(&binding.source_root))
            .with_state_scope(self.state_scope.clone()))
    }

    pub(crate) fn source_revision_service(
        &self,
        binding: &ProviderRootBinding,
    ) -> Result<Arc<SourceRevisionService>, String> {
        self.validate_binding(binding)?;
        let mut revisions = self
            .source_revisions
            .lock()
            .map_err(|_| "workspace actor revision registry is poisoned".to_string())?;
        if let Some(service) = revisions.get(&binding.source_set) {
            return Ok(Arc::clone(service));
        }
        let service = Arc::new(match self.state_scope.scoped_digest() {
            None => SourceRevisionService::new(&self.context, binding.source_root.path())?,
            Some(_) => SourceRevisionService::new_scoped(
                &self.context,
                binding.source_root.path(),
                self.state_scope.clone(),
            )?,
        });
        revisions.insert(binding.source_set.clone(), Arc::clone(&service));
        Ok(service)
    }

    pub(crate) fn mark_source_revisions_dirty(&self) {
        if let Ok(revisions) = self.source_revisions.lock() {
            for revision in revisions.values() {
                revision.mark_dirty();
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn install_source_revision_service_for_test(
        &self,
        binding: &ProviderRootBinding,
        service: Arc<SourceRevisionService>,
    ) -> Result<(), String> {
        self.validate_binding(binding)?;
        self.source_revisions
            .lock()
            .map_err(|_| "workspace actor revision registry is poisoned".to_string())?
            .insert(binding.source_set.clone(), service);
        Ok(())
    }

    pub(crate) fn validate_binding(&self, binding: &ProviderRootBinding) -> Result<(), String> {
        if binding.actor_identity != self.identity
            || binding.actor_instance != self.instance_id
            || !self.identity.source_sets.contains(&binding.source_set)
            || binding.source_set.root != binding.source_root.path()
        {
            return Err("provider root binding belongs to another workspace actor".to_string());
        }
        validate_physical_root(&binding.source_root)
    }
}

fn apply_checkpoint(
    deadline: ProviderDeadline,
    cancellation: &CancellationToken,
    phase: &str,
) -> Result<(), String> {
    if cancellation.is_cancelled() {
        Err(format!("{phase} cancelled"))
    } else if deadline.remaining().is_zero() {
        Err(format!("{phase} deadline exceeded"))
    } else {
        Ok(())
    }
}

fn validate_physical_root(root: &RetainedDirectoryCapability) -> Result<(), String> {
    root.validate_named_identity().map_err(|error| {
        format!(
            "workspace actor source-set physical identity changed after admission: {}: {error}",
            root.path().display()
        )
    })
}

fn closed_index_component(value: &str) -> bool {
    !value.is_empty() && value.trim() == value && !value.bytes().any(|byte| byte == 0)
}

fn is_lowercase_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn digest_index_component(digest: &mut Sha256, bytes: &[u8]) -> Result<(), String> {
    let length = u64::try_from(bytes.len())
        .map_err(|_| "actor-owned index work component is too large".to_string())?;
    digest.update(length.to_le_bytes());
    digest.update(bytes);
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WorkspaceActorRegistryError {
    Capacity { limit: usize },
    InvalidIdentity(String),
    Poisoned,
}

impl std::fmt::Display for WorkspaceActorRegistryError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Capacity { limit } => {
                write!(
                    formatter,
                    "workspace actor capacity {limit} is fully leased"
                )
            }
            Self::InvalidIdentity(message) => formatter.write_str(message),
            Self::Poisoned => formatter.write_str("workspace actor registry is poisoned"),
        }
    }
}

impl std::error::Error for WorkspaceActorRegistryError {}

#[derive(Debug)]
pub(crate) struct WorkspaceActorRegistry {
    actors: Mutex<HashMap<WorkspaceIdentity, Weak<WorkspaceActor>>>,
    #[cfg(test)]
    max_active_override: Option<usize>,
}

impl Default for WorkspaceActorRegistry {
    fn default() -> Self {
        Self {
            actors: Mutex::new(HashMap::new()),
            #[cfg(test)]
            max_active_override: None,
        }
    }
}

impl WorkspaceActorRegistry {
    pub(crate) fn get_or_create<I, N, P>(
        &self,
        context: &WorkspaceContext,
        source_sets: I,
        provider_profile: &str,
    ) -> Result<Arc<WorkspaceActor>, WorkspaceActorRegistryError>
    where
        I: IntoIterator<Item = (N, P)>,
        N: AsRef<str>,
        P: AsRef<Path>,
    {
        let identity = WorkspaceIdentity::new(context, source_sets, provider_profile)
            .map_err(WorkspaceActorRegistryError::InvalidIdentity)?;
        let mut actors = self
            .actors
            .lock()
            .map_err(|_| WorkspaceActorRegistryError::Poisoned)?;
        actors.retain(|_, actor| actor.strong_count() > 0);
        if let Some(actor) = actors.get(&identity).and_then(Weak::upgrade) {
            return Ok(actor);
        }
        let max_active = self.max_active();
        if actors.len() >= max_active {
            return Err(WorkspaceActorRegistryError::Capacity { limit: max_active });
        }
        let actor = Arc::new(
            WorkspaceActor::new(identity.clone(), context.clone())
                .map_err(WorkspaceActorRegistryError::InvalidIdentity)?,
        );
        actors.insert(identity, Arc::downgrade(&actor));
        Ok(actor)
    }

    #[cfg(test)]
    fn len(&self) -> Result<usize, String> {
        self.actors
            .lock()
            .map(|actors| {
                actors
                    .values()
                    .filter(|actor| actor.strong_count() > 0)
                    .count()
            })
            .map_err(|_| "workspace actor registry is poisoned".to_string())
    }

    #[cfg(test)]
    pub(crate) fn with_capacity_for_test(max_active: usize) -> Self {
        assert!(max_active > 0);
        Self {
            actors: Mutex::new(HashMap::new()),
            max_active_override: Some(max_active),
        }
    }

    fn max_active(&self) -> usize {
        #[cfg(test)]
        if let Some(max_active) = self.max_active_override {
            return max_active;
        }
        MAX_ACTIVE_WORKSPACE_ACTORS
    }

    #[cfg(test)]
    pub(crate) fn live_len_for_test(&self) -> Result<usize, String> {
        self.len()
    }

    #[cfg(test)]
    pub(crate) fn entry_len_for_test(&self) -> Result<usize, String> {
        self.actors
            .lock()
            .map(|actors| actors.len())
            .map_err(|_| "workspace actor registry is poisoned".to_string())
    }

    #[cfg(test)]
    pub(crate) fn poison_for_test(&self) {
        let poisoned = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = self.actors.lock().unwrap();
            panic!("poison workspace actor registry for deterministic admission test");
        }));
        assert!(poisoned.is_err());
    }

    #[cfg(test)]
    fn prune_dead_for_test(&self) -> Result<(), String> {
        self.actors
            .lock()
            .map(|mut actors| actors.retain(|_, actor| actor.strong_count() > 0))
            .map_err(|_| "workspace actor registry is poisoned".to_string())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::{
        set_apply_dry_run_after_confirmation_hook, WorkspaceActorRegistry,
        WorkspaceActorRegistryError, WorkspaceIdentity,
    };
    use crate::domain::cancellation::CancellationToken;
    use crate::domain::code_intelligence::ProviderDeadline;
    use crate::domain::workspace::WorkspaceContext;
    use crate::infrastructure::platform::source_revision_fence::{
        FenceCapability, FenceOutcome, SourceRevisionFence,
    };
    use crate::infrastructure::source_revision::SourceRevisionService;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::mpsc;
    use std::sync::Arc;
    use std::thread;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    #[test]
    fn index_coordination_has_no_raw_crate_level_constructor_or_owner_escape() {
        let shared_work = include_str!("../application/shared_work.rs");
        let actor = include_str!("workspace_actor.rs");
        assert!(
            !shared_work.contains("pub(crate) struct IndexWorkKey"),
            "raw IndexWorkKey remains constructible across the crate"
        );
        assert!(
            !shared_work.contains("IndexWorkOwner"),
            "raw IndexWorkOwner remains joinable outside the actor"
        );
        let raw_key_accessor = ["pub(crate) fn index_", "work_key"].concat();
        let raw_owner_accessor = ["pub(crate) fn index_", "work(&self)"].concat();
        assert!(
            !actor.contains(&raw_key_accessor) && !actor.contains(&raw_owner_accessor),
            "WorkspaceActor still exposes a movable key/owner pair"
        );
        assert!(
            actor.contains("pub(crate) fn join_index_work"),
            "the actor-owned exact revision join seam is absent"
        );
    }

    #[test]
    fn workspace_actor_serializes_mutation_publication() {
        let fixture = actor_fixture("serialized-mutations", &["src"]);
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        let fence = fixture
            .actor
            .capture_revision(
                &binding,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        let (entered_tx, entered_rx) = mpsc::channel();
        let (attempted_tx, attempted_rx) = mpsc::channel();
        let (first_release_tx, first_release_rx) = mpsc::channel();
        let (second_release_tx, second_release_rx) = mpsc::channel();

        let first_actor = Arc::clone(&fixture.actor);
        let first_fence = fence.clone();
        let first_entered = entered_tx.clone();
        let first = thread::spawn(move || {
            let lease = first_actor.begin_publication(
                &first_fence,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )?;
            first_entered.send("first").unwrap();
            first_release_rx.recv().unwrap();
            lease.publish(
                (),
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
        });
        assert_eq!(
            entered_rx.recv_timeout(Duration::from_secs(2)).unwrap(),
            "first"
        );

        let second_actor = Arc::clone(&fixture.actor);
        let second = thread::spawn(move || {
            attempted_tx.send(()).unwrap();
            let lease = second_actor.begin_publication(
                &fence,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )?;
            entered_tx.send("second").unwrap();
            second_release_rx.recv().unwrap();
            lease.publish(
                (),
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
        });
        attempted_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(
            entered_rx.recv_timeout(Duration::from_millis(100)).is_err(),
            "a second mutation crossed the first actor publication lease"
        );
        first_release_tx.send(()).unwrap();
        assert_eq!(
            entered_rx.recv_timeout(Duration::from_secs(2)).unwrap(),
            "second"
        );
        second_release_tx.send(()).unwrap();
        first.join().unwrap().unwrap();
        second.join().unwrap().unwrap();
        fixture.cleanup();
    }

    #[test]
    fn workspace_actor_pre_cancelled_publication_does_not_wait_for_mutation_lane() {
        let fixture = actor_fixture("pre-cancelled-mutation-lane", &["src"]);
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        let cancellation = CancellationToken::new();
        let fence = fixture
            .actor
            .capture_revision(
                &binding,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &cancellation,
            )
            .unwrap();
        let owner = fixture
            .actor
            .begin_publication(
                &fence,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &cancellation,
            )
            .unwrap();
        let waiter_cancellation = CancellationToken::new();
        waiter_cancellation.cancel();
        let waiter_actor = Arc::clone(&fixture.actor);
        let (result_tx, result_rx) = mpsc::channel();
        let waiter = thread::spawn(move || {
            let result = waiter_actor
                .begin_publication(
                    &fence,
                    ProviderDeadline::from_budget(Duration::from_secs(5)),
                    &waiter_cancellation,
                )
                .map(drop);
            result_tx.send(result).unwrap();
        });

        let result_before_release = result_rx.recv_timeout(Duration::from_millis(250));
        let returned_before_release = result_before_release.is_ok();
        drop(owner);
        let result = result_before_release.unwrap_or_else(|_| {
            result_rx
                .recv_timeout(Duration::from_secs(2))
                .expect("waiter must finish after the owner releases the lane")
        });
        waiter.join().unwrap();

        assert!(
            returned_before_release,
            "pre-cancelled publication waited for the held mutation lane"
        );
        assert!(result.unwrap_err().starts_with("cancelled:"));
        fixture.cleanup();
    }

    #[test]
    pub(crate) fn logical_read_publication_lane_wait_honors_existing_cancellation_and_deadline() {
        let fixture = actor_fixture("logical-read-lane-bounds", &["src"]);
        std::fs::write(
            fixture.roots[0].join("Configuration.xml"),
            "<Configuration/>",
        )
        .unwrap();
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        let logical_fence = fixture
            .actor
            .capture_logical_read_revision(
                &binding,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        let legacy_fence = fixture
            .actor
            .capture_revision(
                &binding,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        let owner = fixture
            .actor
            .begin_publication(
                &legacy_fence,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();

        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let cancelled = fixture.actor.publish_logical_read(
            std::slice::from_ref(&logical_fence),
            (),
            ProviderDeadline::from_budget(Duration::from_secs(5)),
            &cancellation,
        );
        assert!(cancelled.unwrap_err().starts_with("cancelled:"));

        let deadline = fixture.actor.publish_logical_read(
            &[logical_fence],
            (),
            ProviderDeadline::from_budget(Duration::ZERO),
            &CancellationToken::new(),
        );
        assert!(deadline.unwrap_err().ends_with("deadline exceeded"));
        drop(owner);
        fixture.cleanup();
    }

    #[test]
    fn workspace_actor_late_cancellation_stops_mutation_lane_wait() {
        let fixture = actor_fixture("late-cancelled-mutation-lane", &["src"]);
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        let cancellation = CancellationToken::new();
        let fence = fixture
            .actor
            .capture_revision(
                &binding,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &cancellation,
            )
            .unwrap();
        let owner = fixture
            .actor
            .begin_publication(
                &fence,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &cancellation,
            )
            .unwrap();
        let waiter_cancellation = CancellationToken::new();
        let waiter_signal = waiter_cancellation.clone();
        let waiter_actor = Arc::clone(&fixture.actor);
        let (started_tx, started_rx) = mpsc::channel();
        let (result_tx, result_rx) = mpsc::channel();
        let waiter = thread::spawn(move || {
            started_tx.send(()).unwrap();
            let result = waiter_actor
                .begin_publication(
                    &fence,
                    ProviderDeadline::from_budget(Duration::from_secs(5)),
                    &waiter_cancellation,
                )
                .map(drop);
            result_tx.send(result).unwrap();
        });
        started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(
            result_rx.recv_timeout(Duration::from_millis(50)).is_err(),
            "waiter unexpectedly crossed the held mutation lane"
        );
        waiter_signal.cancel();

        let result_before_release = result_rx.recv_timeout(Duration::from_millis(250));
        let returned_before_release = result_before_release.is_ok();
        drop(owner);
        let result = result_before_release.unwrap_or_else(|_| {
            result_rx
                .recv_timeout(Duration::from_secs(2))
                .expect("waiter must finish after the owner releases the lane")
        });
        waiter.join().unwrap();

        assert!(
            returned_before_release,
            "late cancellation did not stop the held mutation-lane wait"
        );
        assert!(result.unwrap_err().starts_with("cancelled:"));
        fixture.cleanup();
    }

    #[test]
    fn workspace_actor_deadline_bounds_mutation_lane_wait() {
        for (label, budget) in [
            ("expired", Duration::ZERO),
            ("elapsed", Duration::from_millis(40)),
        ] {
            let fixture = actor_fixture(&format!("{label}-mutation-lane"), &["src"]);
            let binding = fixture
                .actor
                .bind_provider_root("src", &fixture.roots[0])
                .unwrap();
            let cancellation = CancellationToken::new();
            let fence = fixture
                .actor
                .capture_revision(
                    &binding,
                    ProviderDeadline::from_budget(Duration::from_secs(5)),
                    &cancellation,
                )
                .unwrap();
            let owner = fixture
                .actor
                .begin_publication(
                    &fence,
                    ProviderDeadline::from_budget(Duration::from_secs(5)),
                    &cancellation,
                )
                .unwrap();
            let waiter_actor = Arc::clone(&fixture.actor);
            let (result_tx, result_rx) = mpsc::channel();
            let waiter = thread::spawn(move || {
                let result = waiter_actor
                    .begin_publication(
                        &fence,
                        ProviderDeadline::from_budget(budget),
                        &CancellationToken::new(),
                    )
                    .map(drop);
                result_tx.send(result).unwrap();
            });

            let result_before_release = result_rx.recv_timeout(Duration::from_millis(250));
            let returned_before_release = result_before_release.is_ok();
            drop(owner);
            let result = result_before_release.unwrap_or_else(|_| {
                result_rx
                    .recv_timeout(Duration::from_secs(2))
                    .expect("waiter must finish after the owner releases the lane")
            });
            waiter.join().unwrap();

            assert!(
                returned_before_release,
                "{label} deadline did not bound the held mutation-lane wait"
            );
            assert!(result.unwrap_err().contains("deadline exceeded"));
            fixture.cleanup();
        }
    }

    #[test]
    fn workspace_actor_poisoned_mutation_lane_fails_closed_before_publication() {
        let fixture = actor_fixture("poisoned-mutation-lane", &["src"]);
        std::fs::write(fixture.roots[0].join("Module.bsl"), "test").unwrap();
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        let fence_calls = Arc::new(AtomicUsize::new(0));
        let revision_service = Arc::new(
            SourceRevisionService::new_with_fence_for_test(
                fixture.actor.context(),
                &fixture.roots[0],
                fixture.actor.state_scope.clone(),
                Arc::new(CountingActorFence {
                    calls: Arc::clone(&fence_calls),
                }),
            )
            .unwrap(),
        );
        fixture
            .actor
            .install_source_revision_service_for_test(&binding, revision_service)
            .unwrap();
        let fence = fixture
            .actor
            .capture_revision(
                &binding,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        let poison_actor = Arc::clone(&fixture.actor);
        let poison_fence = fence.clone();
        let poison = thread::spawn(move || {
            let _lease = poison_actor
                .begin_publication(
                    &poison_fence,
                    ProviderDeadline::from_budget(Duration::from_secs(5)),
                    &CancellationToken::new(),
                )
                .unwrap();
            panic!("poison workspace actor mutation lane");
        });
        assert!(poison.join().is_err());
        let calls_after_poison = fence_calls.load(Ordering::Acquire);

        let result = fixture
            .actor
            .begin_publication(
                &fence,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .and_then(|lease| {
                lease.publish(
                    "POISONED-STAGED-TEXT",
                    ProviderDeadline::from_budget(Duration::from_secs(5)),
                    &CancellationToken::new(),
                )
            });
        let error = result.unwrap_err();

        assert_eq!(error, "workspace actor mutation lane is poisoned");
        assert!(!error.contains("POISONED-STAGED-TEXT"), "{error}");
        assert_eq!(
            fence_calls.load(Ordering::Acquire),
            calls_after_poison,
            "poisoned mutation lane executed a source revision fence"
        );
        fixture.cleanup();
    }

    struct CountingActorFence {
        calls: Arc<AtomicUsize>,
    }

    impl SourceRevisionFence for CountingActorFence {
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

    #[test]
    fn workspace_actor_allows_reads_to_overlap() {
        let fixture = actor_fixture("concurrent-reads", &["src"]);
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        let (entered_tx, entered_rx) = mpsc::channel();
        let (first_release_tx, first_release_rx) = mpsc::channel();
        let (second_release_tx, second_release_rx) = mpsc::channel();

        let first_actor = Arc::clone(&fixture.actor);
        let first_binding = binding.clone();
        let first_entered = entered_tx.clone();
        let first = thread::spawn(move || {
            first_actor.read(&first_binding, |_| {
                first_entered.send("first").unwrap();
                first_release_rx.recv().unwrap();
                Ok(())
            })
        });
        assert_eq!(
            entered_rx.recv_timeout(Duration::from_secs(2)).unwrap(),
            "first"
        );

        let second_actor = Arc::clone(&fixture.actor);
        let second = thread::spawn(move || {
            second_actor.read(&binding, |_| {
                entered_tx.send("second").unwrap();
                second_release_rx.recv().unwrap();
                Ok(())
            })
        });
        assert_eq!(
            entered_rx.recv_timeout(Duration::from_secs(2)).unwrap(),
            "second",
            "reads through one actor must not share the exclusive mutation lane"
        );
        first_release_tx.send(()).unwrap();
        second_release_tx.send(()).unwrap();
        first.join().unwrap().unwrap();
        second.join().unwrap().unwrap();
        fixture.cleanup();
    }

    #[test]
    fn workspace_actor_rejects_a_stale_revision_before_publication() {
        let fixture = actor_fixture("revision-fence", &["src"]);
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        let module = fixture.roots[0].join("Module.bsl");
        std::fs::write(&module, "Процедура До()\nКонецПроцедуры\n").unwrap();
        let fence = fixture
            .actor
            .capture_revision(
                &binding,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        std::fs::write(&module, "Процедура После()\nКонецПроцедуры\n").unwrap();
        let error = fixture
            .actor
            .begin_publication(
                &fence,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap_err();

        assert!(
            error.contains("source revision changed before publication"),
            "{error}"
        );
        fixture.cleanup();
    }

    #[test]
    fn workspace_actor_rejects_a_revision_changed_after_lease_before_publish() {
        let fixture = actor_fixture("revision-after-lease", &["src"]);
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        let module = fixture.roots[0].join("Module.bsl");
        std::fs::write(&module, "Процедура До()\nКонецПроцедуры\n").unwrap();
        let cancellation = CancellationToken::new();
        let fence = fixture
            .actor
            .capture_revision(
                &binding,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &cancellation,
            )
            .unwrap();
        let lease = fixture
            .actor
            .begin_publication(
                &fence,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &cancellation,
            )
            .unwrap();
        std::fs::write(&module, "Процедура После()\nКонецПроцедуры\n").unwrap();

        let error = lease
            .publish(
                "FOREIGN-STAGED-PROVIDER-TEXT",
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &cancellation,
            )
            .unwrap_err();

        assert!(error.contains("source revision changed"), "{error}");
        assert!(!error.contains("FOREIGN-STAGED-PROVIDER-TEXT"), "{error}");
        fixture.cleanup();
    }

    #[test]
    fn workspace_actor_rejects_root_replacement_after_lease_before_publish() {
        let fixture = actor_fixture("root-after-lease", &["src"]);
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        std::fs::write(fixture.roots[0].join("Module.bsl"), "test").unwrap();
        let cancellation = CancellationToken::new();
        let fence = fixture
            .actor
            .capture_revision(
                &binding,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &cancellation,
            )
            .unwrap();
        let lease = fixture
            .actor
            .begin_publication(
                &fence,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &cancellation,
            )
            .unwrap();
        let displaced = fixture.root.join("src-displaced-after-lease");
        std::fs::rename(&fixture.roots[0], &displaced).unwrap();
        std::fs::create_dir_all(&fixture.roots[0]).unwrap();

        let error = lease
            .publish(
                "FOREIGN-STAGED-PROVIDER-TEXT",
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &cancellation,
            )
            .unwrap_err();

        assert!(error.contains("physical identity changed"), "{error}");
        assert!(!error.contains("FOREIGN-STAGED-PROVIDER-TEXT"), "{error}");
        fixture.cleanup();
    }

    #[test]
    fn workspace_actor_publish_honors_cancellation_and_masks_staged_text() {
        let fixture = actor_fixture("cancelled-publish", &["src"]);
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        std::fs::write(fixture.roots[0].join("Module.bsl"), "test").unwrap();
        let cancellation = CancellationToken::new();
        let fence = fixture
            .actor
            .capture_revision(
                &binding,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &cancellation,
            )
            .unwrap();
        let lease = fixture
            .actor
            .begin_publication(
                &fence,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &cancellation,
            )
            .unwrap();
        cancellation.cancel();

        let error = lease
            .publish(
                "FOREIGN-STAGED-PROVIDER-TEXT",
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &cancellation,
            )
            .unwrap_err();

        assert!(error.starts_with("cancelled:"), "{error}");
        assert!(!error.contains("FOREIGN-STAGED-PROVIDER-TEXT"), "{error}");
        fixture.cleanup();
    }

    #[test]
    fn workspace_actor_publish_cancellation_bounds_revision_lane_contention() {
        let (error, returned_before_owner_release) =
            publish_while_revision_operation_is_held(Duration::from_secs(5), true);

        assert!(returned_before_owner_release);
        assert!(error.starts_with("cancelled:"), "{error}");
        assert!(!error.contains("FOREIGN-STAGED-PROVIDER-TEXT"), "{error}");
    }

    #[test]
    fn workspace_actor_publish_deadline_bounds_revision_lane_contention() {
        let (error, returned_before_owner_release) =
            publish_while_revision_operation_is_held(Duration::from_millis(40), false);

        assert!(returned_before_owner_release);
        assert!(error.contains("deadline exceeded"), "{error}");
        assert!(!error.contains("FOREIGN-STAGED-PROVIDER-TEXT"), "{error}");
    }

    fn publish_while_revision_operation_is_held(
        budget: Duration,
        cancel_before_publish: bool,
    ) -> (String, bool) {
        let fixture = actor_fixture("contended-publish-revision", &["src"]);
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        std::fs::write(fixture.roots[0].join("Module.bsl"), "test").unwrap();
        let cancellation = CancellationToken::new();
        let fence = fixture
            .actor
            .capture_revision(
                &binding,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &cancellation,
            )
            .unwrap();
        let lease = fixture
            .actor
            .begin_publication(
                &fence,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &cancellation,
            )
            .unwrap();
        let revision_service = fixture.actor.source_revision_service(&binding).unwrap();
        let owner_released = Arc::new(AtomicBool::new(false));
        let owner_released_signal = Arc::clone(&owner_released);
        let (owner_held_tx, owner_held_rx) = mpsc::channel();
        let (owner_release_tx, owner_release_rx) = mpsc::channel();
        let owner = thread::spawn(move || {
            let guard = revision_service.hold_operation_for_test();
            owner_held_tx.send(()).unwrap();
            owner_release_rx.recv().unwrap();
            owner_released_signal.store(true, Ordering::Release);
            drop(guard);
        });
        owner_held_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let (watchdog_done_tx, watchdog_done_rx) = mpsc::channel();
        let emergency_release = owner_release_tx.clone();
        let watchdog = thread::spawn(move || {
            if watchdog_done_rx
                .recv_timeout(Duration::from_millis(500))
                .is_err()
            {
                let _ = emergency_release.send(());
            }
        });
        if cancel_before_publish {
            cancellation.cancel();
        }

        let error = lease
            .publish(
                "FOREIGN-STAGED-PROVIDER-TEXT",
                ProviderDeadline::from_budget(budget),
                &cancellation,
            )
            .unwrap_err();
        let returned_before_owner_release = !owner_released.load(Ordering::Acquire);
        let _ = watchdog_done_tx.send(());
        let _ = owner_release_tx.send(());
        watchdog.join().unwrap();
        owner.join().unwrap();
        fixture.cleanup();
        (error, returned_before_owner_release)
    }

    #[test]
    fn retained_binding_rejects_a_root_replaced_by_another_source_set_link() {
        use crate::infrastructure::platform::testing::{
            create_directory_link_fixture_for_test, FileLinkFixtureOutcome,
        };

        let fixture = actor_fixture("root-link-replacement", &["A", "B"]);
        let relative = Path::new("CommonModules/Same/Ext/Module.bsl");
        for (root, contents) in fixture.roots.iter().zip(["root A", "root B"]) {
            std::fs::create_dir_all(root.join(relative).parent().unwrap()).unwrap();
            std::fs::write(root.join(relative), contents).unwrap();
        }
        let binding = fixture
            .actor
            .bind_provider_root("A", &fixture.roots[0])
            .unwrap();
        let displaced = fixture.root.join("A-displaced");
        std::fs::rename(&fixture.roots[0], &displaced).unwrap();
        match create_directory_link_fixture_for_test(&fixture.roots[1], &fixture.roots[0]).unwrap()
        {
            FileLinkFixtureOutcome::Created => {}
            FileLinkFixtureOutcome::Unsupported
            | FileLinkFixtureOutcome::WindowsPrivilegeUnavailable => {
                let _ = std::fs::rename(&displaced, &fixture.roots[0]);
                fixture.cleanup();
                return;
            }
        }

        let result = fixture.actor.read_relative_file(&binding, relative, 1024);

        assert!(
            result.is_err(),
            "retained binding followed replacement: {result:?}"
        );
        fixture.cleanup();
    }

    #[test]
    fn retained_binding_rejects_a_same_path_directory_replacement() {
        let fixture = actor_fixture("root-directory-replacement", &["A"]);
        let relative = Path::new("Module.bsl");
        std::fs::write(fixture.roots[0].join(relative), "original").unwrap();
        let binding = fixture
            .actor
            .bind_provider_root("A", &fixture.roots[0])
            .unwrap();
        std::fs::rename(&fixture.roots[0], fixture.root.join("A-displaced")).unwrap();
        std::fs::create_dir_all(&fixture.roots[0]).unwrap();
        std::fs::write(fixture.roots[0].join(relative), "replacement").unwrap();

        let result = fixture.actor.read_relative_file(&binding, relative, 1024);

        assert!(
            result.is_err(),
            "retained binding accepted replacement: {result:?}"
        );
        fixture.cleanup();
    }

    #[test]
    fn descriptor_relative_read_never_follows_a_nested_directory_link() {
        use crate::infrastructure::platform::testing::{
            create_directory_link_fixture_for_test, FileLinkFixtureOutcome,
        };

        let fixture = actor_fixture("nested-read-link", &["A", "B"]);
        std::fs::write(fixture.roots[1].join("Secret.bsl"), "root B").unwrap();
        let nested = fixture.roots[0].join("Nested");
        match create_directory_link_fixture_for_test(&fixture.roots[1], &nested).unwrap() {
            FileLinkFixtureOutcome::Created => {}
            FileLinkFixtureOutcome::Unsupported
            | FileLinkFixtureOutcome::WindowsPrivilegeUnavailable => {
                fixture.cleanup();
                return;
            }
        }
        let binding = fixture
            .actor
            .bind_provider_root("A", &fixture.roots[0])
            .unwrap();

        let result =
            fixture
                .actor
                .read_relative_file(&binding, Path::new("Nested/Secret.bsl"), 1024);

        assert!(result.is_err(), "nested link was followed: {result:?}");
        fixture.cleanup();
    }

    #[test]
    fn path_provider_discards_output_when_root_changes_mid_operation() {
        let fixture = actor_fixture("root-mid-operation-swap", &["A"]);
        let relative = Path::new("Module.bsl");
        std::fs::write(fixture.roots[0].join(relative), "before swap").unwrap();
        let binding = fixture
            .actor
            .bind_provider_root("A", &fixture.roots[0])
            .unwrap();
        let displaced = fixture.root.join("A-displaced");
        let replacement = fixture.roots[0].clone();

        let result = fixture.actor.read(&binding, |root| {
            let output =
                std::fs::read_to_string(root.join(relative)).map_err(|error| error.to_string())?;
            std::fs::rename(root, &displaced).map_err(|error| error.to_string())?;
            std::fs::create_dir_all(&replacement).map_err(|error| error.to_string())?;
            Ok(output)
        });

        assert!(
            result.is_err(),
            "provider published output after swap: {result:?}"
        );
        fixture.cleanup();
    }

    #[test]
    fn capabilities_do_not_cross_distinct_actor_instances_with_equal_identity() {
        let root = temp_root("actor-instance-capability");
        let source = root.join("src");
        std::fs::create_dir_all(&source).unwrap();
        let context = context(&root);
        let identity = WorkspaceIdentity::new(&context, [("main", &source)], "profile").unwrap();
        let first = super::WorkspaceActor::new(identity.clone(), context.clone()).unwrap();
        let second = super::WorkspaceActor::new(identity, context).unwrap();
        let binding = first.bind_provider_root("main", &source).unwrap();
        let fence = first
            .capture_revision(
                &binding,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();

        assert!(second.read(&binding, |_| Ok(())).is_err());
        assert!(second
            .begin_publication(
                &fence,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .is_err());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn workspace_actor_capabilities_enforce_identity_physical_and_bounded_publication() {
        use crate::infrastructure::platform::testing::{
            create_directory_link_fixture_for_test, FileLinkFixtureOutcome,
        };

        let root = temp_root("active-capability-contract");
        let source = root.join("src");
        let outside = root.join("outside");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(source.join("Module.bsl"), "test").unwrap();
        std::fs::write(outside.join("Secret.bsl"), "outside").unwrap();
        let context = context(&root);
        assert!(WorkspaceIdentity::new(
            &context,
            [("main", &source), ("alias", &source)],
            "profile",
        )
        .is_err());
        let identity = WorkspaceIdentity::new(&context, [("main", &source)], "profile").unwrap();
        let first = super::WorkspaceActor::new(identity.clone(), context.clone()).unwrap();
        let second = super::WorkspaceActor::new(identity, context).unwrap();
        let binding = first.bind_provider_root("main", &source).unwrap();
        let fence = first
            .capture_revision(
                &binding,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();

        assert!(second.read(&binding, |_| Ok(())).is_err());
        assert!(second
            .begin_publication(
                &fence,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .is_err());

        let publication = first
            .begin_publication(
                &fence,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        std::fs::write(source.join("Module.bsl"), "changed after lease").unwrap();
        assert!(publication
            .publish(
                "must not escape",
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .is_err());

        let nested = source.join("Nested");
        if matches!(
            create_directory_link_fixture_for_test(&outside, &nested).unwrap(),
            FileLinkFixtureOutcome::Created
        ) {
            assert!(first
                .read_relative_file(&binding, Path::new("Nested/Secret.bsl"), 1024)
                .is_err());
        }

        let displaced = root.join("src-displaced");
        let replacement = source.clone();
        let external = first.read(&binding, |root| {
            let output =
                std::fs::read(root.join("Module.bsl")).map_err(|error| error.to_string())?;
            std::fs::rename(root, &displaced).map_err(|error| error.to_string())?;
            std::fs::create_dir_all(&replacement).map_err(|error| error.to_string())?;
            Ok(output)
        });
        assert!(external.is_err());
        assert!(first
            .begin_publication(
                &fence,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .is_err());
        let _ = std::fs::remove_dir_all(root);

        workspace_actor_pre_cancelled_publication_does_not_wait_for_mutation_lane();
        workspace_actor_late_cancellation_stops_mutation_lane_wait();
        workspace_actor_deadline_bounds_mutation_lane_wait();

        for (budget, cancel_before_publish) in [
            (Duration::from_secs(5), true),
            (Duration::from_millis(40), false),
        ] {
            let (error, returned_before_owner_release) =
                publish_while_revision_operation_is_held(budget, cancel_before_publish);
            assert!(returned_before_owner_release, "{error}");
            assert!(!error.contains("FOREIGN-STAGED-PROVIDER-TEXT"), "{error}");
        }
    }

    #[test]
    fn duplicate_physical_root_names_are_rejected_as_ambiguous() {
        let root = temp_root("duplicate-physical-root");
        let source = root.join("src");
        std::fs::create_dir_all(&source).unwrap();
        let context = context(&root);

        let result =
            WorkspaceIdentity::new(&context, [("main", &source), ("alias", &source)], "profile");

        assert!(result.is_err(), "duplicate physical root was accepted");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn remapped_names_and_profiles_do_not_share_revision_index_or_coordination_state() {
        let root = temp_root("state-scope-separation");
        let source = root.join("src");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(source.join("Module.bsl"), "test").unwrap();
        let context = context(&root);
        let deadline = || ProviderDeadline::from_budget(Duration::from_secs(5));
        let cancellation = CancellationToken::new();
        let mut state_roots = Vec::new();

        for (name, profile) in [
            ("main", "program"),
            ("renamed", "program"),
            ("main", "program-and-service"),
        ] {
            let identity = WorkspaceIdentity::new(&context, [(name, &source)], profile).unwrap();
            let actor = super::WorkspaceActor::new(identity, context.clone()).unwrap();
            let binding = actor.bind_provider_root(name, &source).unwrap();
            actor
                .capture_revision(&binding, deadline(), &cancellation)
                .unwrap();
            let service = actor
                .index_service(
                    &binding,
                    &crate::infrastructure::workspace_index::SYSTEM_INDEX_RUNNER,
                )
                .unwrap();
            state_roots.push(
                service
                    .provider_state_root_for_test(&context, &source)
                    .unwrap(),
            );
        }
        let records = std::fs::read_dir(context.cache_root.join("source-revisions"))
            .unwrap()
            .count();

        assert_eq!(
            records, 3,
            "logical actor scopes collapsed persisted revisions"
        );
        assert_ne!(state_roots[0], state_roots[1]);
        assert_ne!(state_roots[0], state_roots[2]);
        assert_ne!(state_roots[1], state_roots[2]);

        let legacy_direct =
            crate::infrastructure::source_revision::SourceRevisionService::new(&context, &source)
                .unwrap();
        legacy_direct.snapshot(deadline(), &cancellation).unwrap();
        let legacy_identity =
            WorkspaceIdentity::new(&context, [("main", &source)], "legacy-profile").unwrap();
        let legacy_actor =
            super::WorkspaceActor::with_legacy_runtime(legacy_identity, context.clone(), ())
                .unwrap();
        let legacy_binding = legacy_actor.bind_provider_root("main", &source).unwrap();
        legacy_actor
            .capture_revision(&legacy_binding, deadline(), &cancellation)
            .unwrap();
        assert_eq!(
            std::fs::read_dir(context.cache_root.join("source-revisions"))
                .unwrap()
                .count(),
            4,
            "legacy adapter moved the v0.12 revision namespace"
        );
        let direct_index = crate::infrastructure::workspace_index::WorkspaceIndexService::new()
            .provider_state_root_for_test(&context, &source)
            .unwrap();
        let actor_index = legacy_actor
            .index_service(
                &legacy_binding,
                &crate::infrastructure::workspace_index::SYSTEM_INDEX_RUNNER,
            )
            .unwrap()
            .provider_state_root_for_test(&context, &source)
            .unwrap();
        assert_eq!(
            direct_index, actor_index,
            "legacy adapter moved the v0.12 index/provider namespace"
        );
        if let Some((first, second)) =
            crate::infrastructure::platform::filesystem::distinct_non_unicode_paths_for_test()
        {
            let identity = |source_root| WorkspaceIdentity {
                workspace_root: PathBuf::from("/workspace"),
                source_sets: vec![super::WorkspaceSourceSetIdentity {
                    name: "main".to_string(),
                    root: source_root,
                }],
                provider_profile: "profile".to_string(),
            };
            assert_ne!(
                identity(first).state_scope_digest().unwrap(),
                identity(second).state_scope_digest().unwrap(),
                "distinct native path identities collapsed actor state"
            );
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn workspace_actor_exposes_no_raw_generic_runtime_payload() {
        let immutable_return = ["-> ", "&R"].concat();
        let mutable_return = ["-> ", "&mut R"].concat();
        let source = include_str!("workspace_actor.rs");

        assert!(
            !source.lines().any(|line| line.contains(&immutable_return)),
            "workspace actor exposes a raw immutable runtime payload"
        );
        assert!(
            !source.lines().any(|line| line.contains(&mutable_return)),
            "workspace actor exposes a raw mutable runtime payload"
        );
    }

    #[test]
    fn actor_state_scope_digest_is_fallible_and_bounded() {
        fn digest(identity: &WorkspaceIdentity) -> Result<String, String> {
            identity.state_scope_digest()
        }

        let identity = WorkspaceIdentity {
            workspace_root: PathBuf::from("/workspace"),
            source_sets: vec![super::WorkspaceSourceSetIdentity {
                name: "main".to_string(),
                root: PathBuf::from("/workspace/source"),
            }],
            provider_profile: "profile".to_string(),
        };
        let digest = digest(&identity).unwrap();

        assert_eq!(digest.len(), 64);
        assert!(digest.bytes().all(|byte| byte.is_ascii_hexdigit()));
    }

    #[test]
    fn actor_state_scope_distinguishes_native_non_unicode_paths() {
        let Some((first, second)) =
            crate::infrastructure::platform::filesystem::distinct_non_unicode_paths_for_test()
        else {
            return;
        };
        let identity = |root| WorkspaceIdentity {
            workspace_root: PathBuf::from("/workspace"),
            source_sets: vec![super::WorkspaceSourceSetIdentity {
                name: "main".to_string(),
                root,
            }],
            provider_profile: "profile".to_string(),
        };

        assert_ne!(
            identity(first).state_scope_digest().unwrap(),
            identity(second).state_scope_digest().unwrap(),
            "distinct native non-Unicode roots collapsed to one actor scope"
        );
    }

    #[test]
    fn workspace_actor_registry_keys_exact_identity_and_separates_worktrees_and_source_roots() {
        let root = temp_root("identity");
        let shared_git = root.join("repository/.git");
        let worktree_a = root.join("worktrees/a");
        let worktree_b = root.join("worktrees/b");
        std::fs::create_dir_all(&shared_git).unwrap();
        for worktree in [&worktree_a, &worktree_b] {
            std::fs::create_dir_all(worktree.join("src-a")).unwrap();
            std::fs::create_dir_all(worktree.join("src-b")).unwrap();
            std::fs::write(worktree.join("v8project.yaml"), "format: DESIGNER\n").unwrap();
            std::fs::write(
                worktree.join(".git"),
                format!("gitdir: {}\n", shared_git.display()),
            )
            .unwrap();
        }
        let registry = WorkspaceActorRegistry::default();
        let context_a = context(&worktree_a);
        let context_b = context(&worktree_b);
        let a = registry
            .get_or_create(
                &context_a,
                [
                    ("main", worktree_a.join("src-a")),
                    ("extension", worktree_a.join("src-b")),
                ],
                "bsl-ls:program",
            )
            .unwrap();
        let same_a = registry
            .get_or_create(
                &context_a,
                [
                    ("extension", worktree_a.join("src-b")),
                    ("main", worktree_a.join("src-a")),
                ],
                "bsl-ls:program",
            )
            .unwrap();
        let other_worktree = registry
            .get_or_create(
                &context_b,
                [
                    ("main", worktree_b.join("src-a")),
                    ("extension", worktree_b.join("src-b")),
                ],
                "bsl-ls:program",
            )
            .unwrap();
        let other_roots = registry
            .get_or_create(
                &context_a,
                [("main", worktree_a.join("src-a"))],
                "bsl-ls:program",
            )
            .unwrap();
        let other_profile = registry
            .get_or_create(
                &context_a,
                [
                    ("main", worktree_a.join("src-a")),
                    ("extension", worktree_a.join("src-b")),
                ],
                "bsl-ls:program-and-service",
            )
            .unwrap();
        let remapped_names = registry
            .get_or_create(
                &context_a,
                [
                    ("main", worktree_a.join("src-b")),
                    ("extension", worktree_a.join("src-a")),
                ],
                "bsl-ls:program",
            )
            .unwrap();

        assert!(Arc::ptr_eq(&a, &same_a));
        assert!(!Arc::ptr_eq(&a, &other_worktree));
        assert!(!Arc::ptr_eq(&a, &other_roots));
        assert!(!Arc::ptr_eq(&a, &other_profile));
        assert!(!Arc::ptr_eq(&a, &remapped_names));
        assert_eq!(registry.len().unwrap(), 5);
        assert_ne!(
            a.identity().workspace_root(),
            other_worktree.identity().workspace_root()
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn two_frontends_reuse_the_actor_for_one_canonical_worktree() {
        let root = temp_root("frontend-reuse");
        let source = root.join("src");
        let nested = root.join("frontend/cwd");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::create_dir_all(&nested).unwrap();
        let mut first_context = context(&root);
        first_context.cwd = root.clone();
        let mut second_context = context(&root.join("frontend/.."));
        second_context.cwd = nested;
        let registry = WorkspaceActorRegistry::default();

        let first = registry
            .get_or_create(&first_context, [("main", &source)], "legacy-bsl-rlm")
            .unwrap();
        let second = registry
            .get_or_create(
                &second_context,
                [("main", root.join("frontend/../src"))],
                "legacy-bsl-rlm",
            )
            .unwrap();

        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(registry.len().unwrap(), 1);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn daemon_actor_registry_prunes_dead_entries_and_bounds_sequential_roots() {
        let parent = temp_root("bounded-sequential");
        let registry = WorkspaceActorRegistry::with_capacity_for_test(2);

        for index in 0..12 {
            let root = parent.join(format!("workspace-{index}"));
            let source = root.join("src");
            std::fs::create_dir_all(&source).unwrap();
            let actor = registry
                .get_or_create(&context(&root), [("main", &source)], "canonical-v0.13")
                .unwrap();
            assert_eq!(registry.live_len_for_test().unwrap(), 1);
            drop(actor);
        }

        registry.prune_dead_for_test().unwrap();
        assert_eq!(registry.entry_len_for_test().unwrap(), 0);
        let _ = std::fs::remove_dir_all(parent);
    }

    #[test]
    fn daemon_actor_registry_rejects_only_when_all_capacity_entries_are_live() {
        let parent = temp_root("bounded-live");
        let registry = WorkspaceActorRegistry::with_capacity_for_test(2);
        let mut actors = Vec::new();
        for index in 0..2 {
            let root = parent.join(format!("workspace-{index}"));
            let source = root.join("src");
            std::fs::create_dir_all(&source).unwrap();
            actors.push(
                registry
                    .get_or_create(&context(&root), [("main", &source)], "canonical-v0.13")
                    .unwrap(),
            );
        }

        let rejected_root = parent.join("workspace-rejected");
        let rejected_source = rejected_root.join("src");
        std::fs::create_dir_all(&rejected_source).unwrap();
        assert_eq!(
            registry
                .get_or_create(
                    &context(&rejected_root),
                    [("main", &rejected_source)],
                    "canonical-v0.13",
                )
                .unwrap_err(),
            WorkspaceActorRegistryError::Capacity { limit: 2 }
        );
        assert_eq!(registry.live_len_for_test().unwrap(), 2);

        drop(actors.pop());
        let admitted = registry
            .get_or_create(
                &context(&rejected_root),
                [("main", &rejected_source)],
                "canonical-v0.13",
            )
            .unwrap();
        assert_eq!(registry.live_len_for_test().unwrap(), 2);
        drop(admitted);
        drop(actors);
        let _ = std::fs::remove_dir_all(parent);
    }

    #[test]
    fn active_alias_reuses_actor_and_dropped_actor_recreates_a_new_instance() {
        let root = temp_root("weak-alias-recreate");
        let source = root.join("src");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::create_dir_all(root.join("nested")).unwrap();
        let registry = WorkspaceActorRegistry::with_capacity_for_test(1);
        let first = registry
            .get_or_create(&context(&root), [("main", &source)], "canonical-v0.13")
            .unwrap();
        let stale_binding = first.bind_provider_root("main", &source).unwrap();
        let alias = registry
            .get_or_create(
                &context(&root.join("nested/..")),
                [("main", root.join("nested/../src"))],
                "canonical-v0.13",
            )
            .unwrap();
        assert!(Arc::ptr_eq(&first, &alias));

        drop(alias);
        drop(first);
        let replacement = registry
            .get_or_create(&context(&root), [("main", &source)], "canonical-v0.13")
            .unwrap();
        assert!(replacement.validate_binding(&stale_binding).is_err());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn daemon_actor_registry_is_bounded_weak_and_alias_safe() {
        daemon_actor_registry_prunes_dead_entries_and_bounds_sequential_roots();
        daemon_actor_registry_rejects_only_when_all_capacity_entries_are_live();
        active_alias_reuses_actor_and_dropped_actor_recreates_a_new_instance();
    }

    #[test]
    fn multiroot_provider_keeps_identical_relative_paths_bound_to_the_requesting_root() {
        let fixture = actor_fixture("multiroot", &["A", "B"]);
        let relative = Path::new("CommonModules/Same/Ext/Module.bsl");
        for (root, contents) in fixture.roots.iter().zip(["root A", "root B"]) {
            std::fs::create_dir_all(root.join(relative).parent().unwrap()).unwrap();
            std::fs::write(root.join(relative), contents).unwrap();
        }
        let binding_a = fixture
            .actor
            .bind_provider_root("A", &fixture.roots[0])
            .unwrap();
        let binding_b = fixture
            .actor
            .bind_provider_root("B", &fixture.roots[1])
            .unwrap();
        assert!(fixture
            .actor
            .bind_provider_root("A", &fixture.roots[1])
            .is_err());

        let from_a = String::from_utf8(
            fixture
                .actor
                .read_relative_file(&binding_a, relative, 1024)
                .unwrap(),
        )
        .unwrap();
        let from_b = String::from_utf8(
            fixture
                .actor
                .read_relative_file(&binding_b, relative, 1024)
                .unwrap(),
        )
        .unwrap();

        assert_eq!(from_a, "root A");
        assert_eq!(from_b, "root B");
        fixture.cleanup();
    }

    #[test]
    fn actor_bound_index_session_rejects_another_source_set_root() {
        let fixture = actor_fixture("index-root-binding", &["A", "B"]);
        let binding = fixture
            .actor
            .bind_provider_root("A", &fixture.roots[0])
            .unwrap();
        let runner = &crate::infrastructure::workspace_index::SYSTEM_INDEX_RUNNER;
        let service = fixture.actor.index_service(&binding, runner).unwrap();
        let args = serde_json::json!({
            "sourceDir": fixture.roots[1].display().to_string()
        })
        .as_object()
        .unwrap()
        .clone();

        let readiness = service.ready_index(fixture.actor.context(), &args);

        assert!(
            matches!(
                readiness,
                crate::infrastructure::workspace_index::IndexReadiness::Unavailable(ref message)
                    if message.contains("escaped its actor-bound source root")
            ),
            "{readiness:?}"
        );
        fixture.cleanup();
    }

    #[test]
    fn prepared_apply_dry_run_is_byte_identical_and_real_apply_commits_once_with_new_revision() {
        let fixture = actor_fixture("prepared-apply", &["src"]);
        let target = fixture.roots[0].join("Module.bsl");
        std::fs::write(&target, b"original").unwrap();
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        let cancellation = CancellationToken::new();
        let admitted = fixture
            .actor
            .admit_apply(
                &binding,
                None,
                true,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &cancellation,
            )
            .unwrap();
        let admitted_rev = admitted.revision_identity().to_string();
        let mut state = admitted.staged_state().unwrap();
        state
            .replace("Module.bsl", b"original", b"dry-run".to_vec())
            .unwrap();
        let dry_result = fixture
            .actor
            .publish_prepared_apply(admitted.prepare(state).unwrap())
            .unwrap();
        assert_eq!(dry_result.rev(), admitted_rev);
        assert_eq!(std::fs::read(&target).unwrap(), b"original");
        assert_eq!(dry_result.commit_count_for_test(), 0);
        assert!(dry_result.cleanup_diagnostics().is_empty());

        let admitted = fixture
            .actor
            .admit_apply(
                &binding,
                Some(&admitted_rev),
                false,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &cancellation,
            )
            .unwrap();
        let mut state = admitted.staged_state().unwrap();
        state
            .replace("Module.bsl", b"original", b"published".to_vec())
            .unwrap();
        let result = fixture
            .actor
            .publish_prepared_apply(admitted.prepare(state).unwrap())
            .unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"published");
        assert_ne!(result.rev(), admitted_rev);
        assert_eq!(result.commit_count_for_test(), 1);
        assert!(result.cleanup_diagnostics().is_empty());
        fixture.cleanup();
    }

    #[test]
    fn prepared_apply_cleanup_race_surfaces_a_relative_actor_diagnostic() {
        if !crate::infrastructure::platform::testing::can_swap_named_child_behind_retained_handle_for_test() {
            return;
        }
        let fixture = actor_fixture("prepared-cleanup-diagnostic", &["src"]);
        let parent = fixture.roots[0].join("Nested");
        std::fs::create_dir_all(&parent).unwrap();
        let target = parent.join("Module.bsl");
        let owned_recovery = parent.join("owned-recovery.bsl");
        std::fs::write(&target, b"original").unwrap();
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        let admitted = fixture
            .actor
            .admit_apply(
                &binding,
                None,
                false,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        let mut state = admitted.staged_state().unwrap();
        state
            .replace("Nested/Module.bsl", b"original", b"published".to_vec())
            .unwrap();
        let prepared = admitted.prepare(state).unwrap();
        let hook_parent = parent.clone();
        let hook_owned = owned_recovery.clone();
        crate::infrastructure::platform::filesystem::set_before_identity_bound_cleanup_mutation_hook(
            move || {
                let recovery = std::fs::read_dir(&hook_parent)
                    .unwrap()
                    .map(|entry| entry.unwrap().path())
                    .find(|path| {
                        path.file_name()
                            .and_then(|name| name.to_str())
                            .is_some_and(|name| name.starts_with(".unica-apply-"))
                    })
                    .expect("retained apply recovery artifact");
                std::fs::rename(&recovery, &hook_owned).unwrap();
                std::fs::write(&recovery, b"concurrent-recovery").unwrap();
            },
        );

        let result = fixture.actor.publish_prepared_apply(prepared).unwrap();

        assert_eq!(std::fs::read(&target).unwrap(), b"published");
        assert_eq!(std::fs::read(&owned_recovery).unwrap(), b"original");
        assert!(std::fs::read_dir(&parent).unwrap().any(|entry| {
            let path = entry.unwrap().path();
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(".unica-apply-"))
                && std::fs::read(path).unwrap() == b"concurrent-recovery"
        }));
        let [diagnostic] = result.cleanup_diagnostics() else {
            panic!("successful actor result discarded the retained cleanup diagnostic: {result:?}");
        };
        assert_eq!(
            diagnostic.kind(),
            super::ApplyCleanupDiagnosticKind::RetainedRecoveryCleanupIncomplete
        );
        assert_eq!(diagnostic.logical_target(), Path::new("Nested/Module.bsl"));
        let artifact_name = diagnostic.last_known_artifact_name();
        assert!(artifact_name.to_string_lossy().starts_with(".unica-apply-"));
        assert_eq!(Path::new(artifact_name).components().count(), 1);
        assert!(!format!("{diagnostic:?}").contains(&fixture.root.display().to_string()));
        fixture.cleanup();
    }

    #[test]
    fn prepared_apply_rejects_stale_revision_source_change_cancellation_and_deadline_before_publication(
    ) {
        let fixture = actor_fixture("prepared-apply-fences", &["src"]);
        let target = fixture.roots[0].join("Module.bsl");
        std::fs::write(&target, b"original").unwrap();
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        let cancellation = CancellationToken::new();
        assert!(fixture
            .actor
            .admit_apply(
                &binding,
                Some("stale-revision"),
                false,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &cancellation,
            )
            .unwrap_err()
            .contains("ifRev"));

        let admitted = fixture
            .actor
            .admit_apply(
                &binding,
                None,
                false,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &cancellation,
            )
            .unwrap();
        let mut state = admitted.staged_state().unwrap();
        state
            .replace("Module.bsl", b"original", b"must-not-publish".to_vec())
            .unwrap();
        let prepared = admitted.prepare(state).unwrap();
        std::fs::write(&target, b"concurrent").unwrap();
        assert!(fixture
            .actor
            .publish_prepared_apply(prepared)
            .unwrap_err()
            .contains("revision"));
        assert_eq!(std::fs::read(&target).unwrap(), b"concurrent");

        let cancelled = CancellationToken::new();
        let admitted = fixture
            .actor
            .admit_apply(
                &binding,
                None,
                false,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &cancelled,
            )
            .unwrap();
        let mut state = admitted.staged_state().unwrap();
        state
            .replace("Module.bsl", b"concurrent", b"cancelled".to_vec())
            .unwrap();
        let prepared = admitted.prepare(state).unwrap();
        cancelled.cancel();
        assert!(fixture
            .actor
            .publish_prepared_apply(prepared)
            .unwrap_err()
            .contains("cancel"));
        assert_eq!(std::fs::read(&target).unwrap(), b"concurrent");

        assert!(fixture
            .actor
            .admit_apply(
                &binding,
                None,
                false,
                ProviderDeadline::from_budget(Duration::ZERO),
                &CancellationToken::new(),
            )
            .unwrap_err()
            .contains("deadline"));
        fixture.cleanup();
    }

    #[test]
    fn prepared_apply_root_and_actor_capabilities_cannot_be_redirected_or_replayed() {
        let first = actor_fixture("prepared-capability-a", &["src"]);
        let second = actor_fixture("prepared-capability-b", &["src"]);
        let target = first.roots[0].join("Module.bsl");
        std::fs::write(&target, b"original").unwrap();
        let binding = first
            .actor
            .bind_provider_root("src", &first.roots[0])
            .unwrap();
        let admitted = first
            .actor
            .admit_apply(
                &binding,
                None,
                false,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        let mut state = admitted.staged_state().unwrap();
        state
            .replace("Module.bsl", b"original", b"must-not-publish".to_vec())
            .unwrap();
        let prepared = admitted.prepare(state).unwrap();
        assert!(second
            .actor
            .publish_prepared_apply(prepared)
            .unwrap_err()
            .contains("another workspace actor"));
        assert_eq!(std::fs::read(&target).unwrap(), b"original");

        let admitted = first
            .actor
            .admit_apply(
                &binding,
                None,
                false,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        let mut state = admitted.staged_state().unwrap();
        state
            .replace("Module.bsl", b"original", b"redirected".to_vec())
            .unwrap();
        let prepared = admitted.prepare(state).unwrap();
        let displaced = first.root.join("src-displaced");
        std::fs::rename(&first.roots[0], &displaced).unwrap();
        std::fs::create_dir_all(&first.roots[0]).unwrap();
        std::fs::write(first.roots[0].join("Module.bsl"), b"replacement-tree").unwrap();
        assert!(first
            .actor
            .publish_prepared_apply(prepared)
            .unwrap_err()
            .contains("physical identity"));
        assert_eq!(
            std::fs::read(displaced.join("Module.bsl")).unwrap(),
            b"original"
        );
        assert_eq!(
            std::fs::read(first.roots[0].join("Module.bsl")).unwrap(),
            b"replacement-tree"
        );
        first.cleanup();
        second.cleanup();
    }

    #[test]
    fn prepared_apply_root_replacement_during_commit_rolls_back_before_result() {
        let fixture = actor_fixture("prepared-root-race", &["src"]);
        let target = fixture.roots[0].join("Module.bsl");
        std::fs::write(&target, b"original").unwrap();
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        let admitted = fixture
            .actor
            .admit_apply(
                &binding,
                None,
                false,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        let mut state = admitted.staged_state().unwrap();
        state
            .replace("Module.bsl", b"original", b"published-postimage".to_vec())
            .unwrap();
        let prepared = admitted.prepare(state).unwrap();
        let named_root = fixture.roots[0].clone();
        let displaced = fixture.root.join("src-race-displaced");
        let hook_displaced = displaced.clone();
        crate::infrastructure::native_operations::compile_transaction::set_retained_apply_before_post_validation_hook(
            move || {
                std::fs::rename(&named_root, &hook_displaced).unwrap();
                std::fs::create_dir_all(&named_root).unwrap();
                std::fs::write(named_root.join("Module.bsl"), b"replacement-tree").unwrap();
            },
        );

        let error = fixture.actor.publish_prepared_apply(prepared).unwrap_err();
        assert!(
            error.contains("physical identity")
                || error.contains("identity changed after admission"),
            "{error}"
        );
        assert_eq!(
            std::fs::read(displaced.join("Module.bsl")).unwrap(),
            b"original"
        );
        assert_eq!(
            std::fs::read(fixture.roots[0].join("Module.bsl")).unwrap(),
            b"replacement-tree"
        );
        fixture.cleanup();
    }

    #[test]
    fn prepared_apply_create_post_rename_failure_rolls_back_the_published_name() {
        let fixture = actor_fixture("prepared-create-post-rename-failure", &["src"]);
        let target = fixture.roots[0].join("created.txt");
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        let admitted = fixture
            .actor
            .admit_apply(
                &binding,
                None,
                false,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        let mut state = admitted.staged_state().unwrap();
        state.create("created.txt", b"published".to_vec()).unwrap();
        let prepared = admitted.prepare(state).unwrap();
        crate::infrastructure::platform::filesystem::inject_post_rename_sync_failure_for_test();

        let error = fixture.actor.publish_prepared_apply(prepared).unwrap_err();
        assert!(error.contains("post-rename"), "{error}");
        assert!(
            !target.exists(),
            "reported failure left created bytes published"
        );
        fixture.cleanup();
    }

    #[test]
    fn apply_admission_refuses_staged_state_from_another_actor_issued_authority() {
        let fixture = actor_fixture("prepared-foreign-writer-authority", &["src"]);
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        let cancellation = CancellationToken::new();
        let first = fixture
            .actor
            .admit_apply(
                &binding,
                None,
                true,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &cancellation,
            )
            .unwrap();
        let second = fixture
            .actor
            .admit_apply(
                &binding,
                None,
                true,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &cancellation,
            )
            .unwrap();
        let foreign_state = second.staged_state().unwrap();

        let error = first.prepare(foreign_state).unwrap_err();
        assert!(error.contains("authority"), "{error}");
        fixture.cleanup();
    }

    #[test]
    fn prepared_apply_replace_post_rename_failure_restores_the_exact_preimage() {
        let fixture = actor_fixture("prepared-replace-post-rename-failure", &["src"]);
        let target = fixture.roots[0].join("Module.bsl");
        std::fs::write(&target, b"original").unwrap();
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        let admitted = fixture
            .actor
            .admit_apply(
                &binding,
                None,
                false,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        let mut state = admitted.staged_state().unwrap();
        state
            .replace("Module.bsl", b"original", b"published".to_vec())
            .unwrap();
        let prepared = admitted.prepare(state).unwrap();
        crate::infrastructure::platform::filesystem::inject_post_rename_sync_failure_for_test();

        let error = fixture.actor.publish_prepared_apply(prepared).unwrap_err();
        assert!(error.contains("post-rename"), "{error}");
        assert_eq!(std::fs::read(&target).unwrap(), b"original");
        fixture.cleanup();
    }

    #[test]
    fn prepared_apply_replace_preserves_a_destination_replaced_at_the_mutation_boundary() {
        let fixture = actor_fixture("prepared-replace-destination-race", &["src"]);
        let target = fixture.roots[0].join("Module.bsl");
        let displaced = fixture.roots[0].join("concurrent-old.bsl");
        std::fs::write(&target, b"original").unwrap();
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        let admitted = fixture
            .actor
            .admit_apply(
                &binding,
                None,
                false,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        let mut state = admitted.staged_state().unwrap();
        state
            .replace("Module.bsl", b"original", b"apply-bytes".to_vec())
            .unwrap();
        let prepared = admitted.prepare(state).unwrap();
        let raced_target = target.clone();
        let raced_displaced = displaced.clone();
        crate::infrastructure::platform::filesystem::set_before_identity_bound_no_replace_rename_hook(
            move || {
                std::fs::rename(&raced_target, &raced_displaced).unwrap();
                std::fs::write(&raced_target, b"concurrent").unwrap();
            },
        );

        let error = fixture.actor.publish_prepared_apply(prepared).unwrap_err();
        assert!(
            error.contains("preimage") || error.contains("identity"),
            "{error}"
        );
        assert_eq!(std::fs::read(&target).unwrap(), b"concurrent");
        fixture.cleanup();
    }

    #[test]
    fn prepared_apply_remove_preserves_a_destination_replaced_at_the_mutation_boundary() {
        let fixture = actor_fixture("prepared-remove-destination-race", &["src"]);
        let target = fixture.roots[0].join("Module.bsl");
        let displaced = fixture.roots[0].join("concurrent-old.bsl");
        std::fs::write(&target, b"original").unwrap();
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        let admitted = fixture
            .actor
            .admit_apply(
                &binding,
                None,
                false,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        let mut state = admitted.staged_state().unwrap();
        state.remove("Module.bsl", b"original").unwrap();
        let prepared = admitted.prepare(state).unwrap();
        let raced_target = target.clone();
        let raced_displaced = displaced.clone();
        crate::infrastructure::platform::filesystem::set_before_identity_bound_no_replace_rename_hook(
            move || {
                std::fs::rename(&raced_target, &raced_displaced).unwrap();
                std::fs::write(&raced_target, b"concurrent").unwrap();
            },
        );

        let error = fixture.actor.publish_prepared_apply(prepared).unwrap_err();
        assert!(
            error.contains("preimage") || error.contains("identity"),
            "{error}"
        );
        assert_eq!(std::fs::read(&target).unwrap(), b"concurrent");
        fixture.cleanup();
    }

    #[test]
    fn prepared_apply_dry_run_rejects_root_replacement_before_result() {
        let fixture = actor_fixture("prepared-dry-root-race", &["src"]);
        let target = fixture.roots[0].join("Module.bsl");
        std::fs::write(&target, b"original").unwrap();
        let binding = fixture
            .actor
            .bind_provider_root("src", &fixture.roots[0])
            .unwrap();
        let admitted = fixture
            .actor
            .admit_apply(
                &binding,
                None,
                true,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
            )
            .unwrap();
        let mut state = admitted.staged_state().unwrap();
        state
            .replace("Module.bsl", b"original", b"dry-postimage".to_vec())
            .unwrap();
        let prepared = admitted.prepare(state).unwrap();
        let named_root = fixture.roots[0].clone();
        let displaced = fixture.root.join("src-dry-race-displaced");
        let hook_displaced = displaced.clone();
        set_apply_dry_run_after_confirmation_hook(move || {
            std::fs::rename(&named_root, &hook_displaced).unwrap();
            std::fs::create_dir_all(&named_root).unwrap();
            std::fs::write(named_root.join("Module.bsl"), b"replacement-tree").unwrap();
        });

        let error = fixture.actor.publish_prepared_apply(prepared).unwrap_err();
        assert!(error.contains("physical identity"), "{error}");
        assert_eq!(
            std::fs::read(displaced.join("Module.bsl")).unwrap(),
            b"original"
        );
        assert_eq!(
            std::fs::read(fixture.roots[0].join("Module.bsl")).unwrap(),
            b"replacement-tree"
        );
        fixture.cleanup();
    }

    struct ActorFixture {
        root: PathBuf,
        roots: Vec<PathBuf>,
        actor: Arc<super::WorkspaceActor>,
    }

    impl ActorFixture {
        fn cleanup(self) {
            let _ = std::fs::remove_dir_all(self.root);
        }
    }

    fn actor_fixture(name: &str, relative_roots: &[&str]) -> ActorFixture {
        let root = temp_root(name);
        let roots = relative_roots
            .iter()
            .map(|relative| root.join(relative))
            .collect::<Vec<_>>();
        for source_root in &roots {
            std::fs::create_dir_all(source_root).unwrap();
        }
        let context = context(&root);
        let source_sets = relative_roots.iter().zip(roots.iter());
        let identity = WorkspaceIdentity::new(&context, source_sets, "test-provider").unwrap();
        let actor = Arc::new(super::WorkspaceActor::new(identity, context).unwrap());
        ActorFixture { root, roots, actor }
    }

    fn context(root: &Path) -> WorkspaceContext {
        WorkspaceContext {
            cwd: root.to_path_buf(),
            workspace_root: root.to_path_buf(),
            cache_root: root.join(".build/unica"),
            workspace_epoch: 1,
        }
    }

    fn temp_root(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "unica-workspace-actor-{name}-{}-{nonce}",
            std::process::id()
        ))
    }
}
