use crate::domain::cancellation::CancellationToken;
use crate::domain::code_intelligence::ProviderDeadline;
use crate::domain::source_revision::SourceRevision;
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::platform::filesystem::path_starts_with_host_root;
use crate::infrastructure::source_revision::SourceRevisionService;
use crate::infrastructure::source_roots::normalize_path_identity;
use crate::infrastructure::workspace_index::{IndexRunner, WorkspaceIndexService};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

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
}

#[derive(Debug, Clone)]
pub(crate) struct ProviderRootBinding {
    actor_identity: WorkspaceIdentity,
    source_set: WorkspaceSourceSetIdentity,
    source_root: PathBuf,
}

impl ProviderRootBinding {
    pub(crate) fn source_root(&self) -> &Path {
        &self.source_root
    }
}

#[derive(Debug, Clone)]
pub(crate) struct WorkspaceRevisionFence {
    actor_identity: WorkspaceIdentity,
    source_set: WorkspaceSourceSetIdentity,
    revision: SourceRevision,
}

/// One daemon-owned coordination boundary for a canonical worktree.
///
/// Reads do not take the mutation lane. Publication is exclusive and checks
/// the actor-owned source revision immediately before the caller is allowed to
/// commit staged bytes.
pub(crate) struct WorkspaceActor<R = ()> {
    identity: WorkspaceIdentity,
    context: WorkspaceContext,
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
        mut context: WorkspaceContext,
        runtime: R,
    ) -> Result<Self, String> {
        let context_root = normalize_path_identity(&context.workspace_root)?;
        if context_root != identity.workspace_root {
            return Err(
                "workspace actor context does not match its canonical identity".to_string(),
            );
        }
        context.workspace_root = context_root.clone();
        context.cwd = context_root;
        Ok(Self {
            identity,
            context,
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

    pub(crate) fn runtime(&self) -> &R {
        &self.runtime
    }

    #[cfg(test)]
    pub(crate) fn runtime_mut(&mut self) -> &mut R {
        &mut self.runtime
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
        Ok(ProviderRootBinding {
            actor_identity: self.identity.clone(),
            source_set,
            source_root: requested_root,
        })
    }

    pub(crate) fn read<T>(
        &self,
        binding: &ProviderRootBinding,
        read: impl FnOnce(&Path) -> Result<T, String>,
    ) -> Result<T, String> {
        self.validate_binding(binding)?;
        read(&binding.source_root)
    }

    pub(crate) fn capture_revision(
        &self,
        binding: &ProviderRootBinding,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> Result<WorkspaceRevisionFence, String> {
        self.validate_binding(binding)?;
        let revision = self
            .source_revision_service(binding)?
            .snapshot(deadline, cancellation)?;
        Ok(WorkspaceRevisionFence {
            actor_identity: self.identity.clone(),
            source_set: binding.source_set.clone(),
            revision,
        })
    }

    pub(crate) fn publish_mutation<T>(
        &self,
        fence: &WorkspaceRevisionFence,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
        publish: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        let _publication = self
            .mutation_lane
            .lock()
            .map_err(|_| "workspace actor mutation lane is poisoned".to_string())?;
        if fence.actor_identity != self.identity
            || !self.identity.source_sets.contains(&fence.source_set)
        {
            return Err("source revision fence belongs to another workspace actor".to_string());
        }
        let binding = ProviderRootBinding {
            actor_identity: fence.actor_identity.clone(),
            source_root: fence.source_set.root.clone(),
            source_set: fence.source_set.clone(),
        };
        let current = self
            .source_revision_service(&binding)?
            .snapshot(deadline, cancellation)?;
        if current != fence.revision {
            return Err("source revision changed before publication".to_string());
        }
        publish()
    }

    pub(crate) fn index_service<'a>(
        &self,
        binding: &ProviderRootBinding,
        runner: &'a dyn IndexRunner,
    ) -> Result<WorkspaceIndexService<'a>, String> {
        self.validate_binding(binding)?;
        Ok(WorkspaceIndexService::with_runner(runner)
            .with_source_revision_service(self.source_revision_service(binding)?)
            .with_bound_source_root(binding.source_root.clone()))
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
        let service = Arc::new(SourceRevisionService::new(
            &self.context,
            &binding.source_root,
        )?);
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

    fn validate_binding(&self, binding: &ProviderRootBinding) -> Result<(), String> {
        if binding.actor_identity != self.identity
            || !self.identity.source_sets.contains(&binding.source_set)
            || binding.source_set.root != binding.source_root
        {
            return Err("provider root binding belongs to another workspace actor".to_string());
        }
        Ok(())
    }
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
            first_actor.publish_mutation(
                &first_fence,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
                || {
                    first_entered.send("first").unwrap();
                    first_release_rx.recv().unwrap();
                    Ok(())
                },
            )
        });
        assert_eq!(
            entered_rx.recv_timeout(Duration::from_secs(2)).unwrap(),
            "first"
        );

        let second_actor = Arc::clone(&fixture.actor);
        let second = thread::spawn(move || {
            attempted_tx.send(()).unwrap();
            second_actor.publish_mutation(
                &fence,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
                || {
                    entered_tx.send("second").unwrap();
                    second_release_rx.recv().unwrap();
                    Ok(())
                },
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
        let published = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let publication = Arc::clone(&published);

        let error = fixture
            .actor
            .publish_mutation(
                &fence,
                ProviderDeadline::from_budget(Duration::from_secs(5)),
                &CancellationToken::new(),
                || {
                    publication.store(true, std::sync::atomic::Ordering::Release);
                    Ok(())
                },
            )
            .unwrap_err();

        assert!(
            error.contains("source revision changed before publication"),
            "{error}"
        );
        assert!(!published.load(std::sync::atomic::Ordering::Acquire));
        fixture.cleanup();
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
            .get_or_create(&second_context, [("main", &source)], "legacy-bsl-rlm")
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

        let from_a = fixture
            .actor
            .read(&binding_a, |root| {
                std::fs::read_to_string(root.join(relative)).map_err(|error| error.to_string())
            })
            .unwrap();
        let from_b = fixture
            .actor
            .read(&binding_b, |root| {
                std::fs::read_to_string(root.join(relative)).map_err(|error| error.to_string())
            })
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
