use crate::domain::cancellation::CancellationToken;
use crate::domain::code_intelligence::ProviderDeadline;
use crate::domain::source_revision::SourceRevision;
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::platform::filesystem::{
    path_starts_with_host_root, stable_path_identity_bytes, RetainedDirectoryCapability,
};
use crate::infrastructure::source_revision::{SourceRevisionService, WorkspaceStateScope};
use crate::infrastructure::source_roots::normalize_path_identity;
use crate::infrastructure::workspace_index::{IndexRunner, WorkspaceIndexService};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

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

    fn state_scope_digest(&self) -> Result<String, String> {
        let mut digest = Sha256::new();
        digest.update(b"unica-workspace-actor-state-v1\0");
        update_digest_path(&mut digest, &self.workspace_root)?;
        digest.update((self.source_sets.len() as u64).to_le_bytes());
        for source_set in &self.source_sets {
            update_digest_text(&mut digest, &source_set.name);
            update_digest_path(&mut digest, &source_set.root)?;
        }
        update_digest_text(&mut digest, &self.provider_profile);
        Ok(format!("{:x}", digest.finalize()))
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
    pub(crate) fn source_root(&self) -> &Path {
        self.source_root.path()
    }

    pub(crate) fn retained_root(&self) -> Arc<RetainedDirectoryCapability> {
        Arc::clone(&self.source_root)
    }
}

#[derive(Clone)]
pub(crate) struct WorkspaceRevisionFence {
    actor_identity: WorkspaceIdentity,
    actor_instance: ActorInstanceId,
    source_set: WorkspaceSourceSetIdentity,
    revision: SourceRevision,
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
    mutation_lane: Mutex<()>,
    source_revisions: Mutex<HashMap<WorkspaceSourceSetIdentity, Arc<SourceRevisionService>>>,
    runtime: R,
}

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
            mutation_lane: Mutex::new(()),
            source_revisions: Mutex::new(HashMap::new()),
            runtime,
        })
    }

    pub(crate) fn identity(&self) -> &WorkspaceIdentity {
        &self.identity
    }

    pub(crate) fn workspace_identity(&self) -> &WorkspaceIdentity {
        &self.identity
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

    pub(crate) fn begin_publication(
        &self,
        fence: &WorkspaceRevisionFence,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> Result<WorkspacePublicationLease<'_, R>, String> {
        let publication = self
            .mutation_lane
            .lock()
            .map_err(|_| "workspace actor mutation lane is poisoned".to_string())?;
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

fn validate_physical_root(root: &RetainedDirectoryCapability) -> Result<(), String> {
    root.validate_named_identity().map_err(|error| {
        format!(
            "workspace actor source-set physical identity changed after admission: {}: {error}",
            root.path().display()
        )
    })
}

#[derive(Debug, Default)]
pub(crate) struct WorkspaceActorRegistry {
    actors: Mutex<HashMap<WorkspaceIdentity, Arc<WorkspaceActor>>>,
}

impl WorkspaceActorRegistry {
    pub(crate) fn get_or_create<I, N, P>(
        &self,
        context: &WorkspaceContext,
        source_sets: I,
        provider_profile: &str,
    ) -> Result<Arc<WorkspaceActor>, String>
    where
        I: IntoIterator<Item = (N, P)>,
        N: AsRef<str>,
        P: AsRef<Path>,
    {
        let identity = WorkspaceIdentity::new(context, source_sets, provider_profile)?;
        let mut actors = self
            .actors
            .lock()
            .map_err(|_| "workspace actor registry is poisoned".to_string())?;
        if let Some(actor) = actors.get(&identity) {
            return Ok(Arc::clone(actor));
        }
        let actor = Arc::new(WorkspaceActor::new(identity.clone(), context.clone())?);
        actors.insert(identity, Arc::clone(&actor));
        Ok(actor)
    }

    #[cfg(test)]
    fn len(&self) -> Result<usize, String> {
        self.actors
            .lock()
            .map(|actors| actors.len())
            .map_err(|_| "workspace actor registry is poisoned".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::{WorkspaceActorRegistry, WorkspaceIdentity};
    use crate::domain::cancellation::CancellationToken;
    use crate::domain::code_intelligence::ProviderDeadline;
    use crate::domain::workspace::WorkspaceContext;
    use std::path::{Path, PathBuf};
    use std::sync::mpsc;
    use std::sync::Arc;
    use std::thread;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    macro_rules! assert_not_impl {
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

    assert_not_impl!(super::WorkspaceActor<()>: std::ops::Deref);
    assert_not_impl!(super::WorkspaceActor<()>: std::ops::DerefMut);

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
    fn workspace_actor_capabilities_reject_cross_instance_and_physical_rebinding() {
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
