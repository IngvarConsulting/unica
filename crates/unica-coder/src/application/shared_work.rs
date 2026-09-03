use std::collections::{BTreeSet, HashMap};
use std::io;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, Weak};
use std::time::{Duration, Instant};

#[allow(dead_code)] // V13 constructors are exercised by injected services until Task 22 cutover.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SharedWorkIdentityError {
    Provider,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct ProviderHostKey {
    engine: String,
    target: String,
    capabilities: BTreeSet<String>,
}

impl ProviderHostKey {
    #[allow(dead_code)] // Canonical provider handler is installed only at Task 22 cutover.
    pub(crate) fn new(
        engine: impl Into<String>,
        target: impl Into<String>,
        capabilities: BTreeSet<String>,
    ) -> Result<Self, SharedWorkIdentityError> {
        let engine = engine.into();
        let target = target.into();
        if !closed_nonempty(&engine)
            || !closed_nonempty(&target)
            || capabilities.is_empty()
            || capabilities.iter().any(|value| !closed_nonempty(value))
        {
            return Err(SharedWorkIdentityError::Provider);
        }
        Ok(Self {
            engine,
            target,
            capabilities,
        })
    }
}

fn closed_nonempty(value: &str) -> bool {
    !value.is_empty() && value.trim() == value && !value.bytes().any(|byte| byte == 0)
}

/// Stable byte form used in an exact delivery identity.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum DeliveryFormIdentity {
    Archive,
    File,
}

/// Identity of work which may be joined.
///
/// The non-delivery variants deliberately have no routes in Task 10. Naming
/// the complete vocabulary here prevents later adapters from inventing weaker
/// keys while Task 11 connects those producers.
#[allow(dead_code)] // Task 10 routes Delivery; Task 11 owns the reserved exact-key routes.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) enum SharedWorkKey {
    Delivery {
        artifact: String,
        version: String,
        target: String,
        sha256: String,
        form: DeliveryFormIdentity,
    },
    Index {
        identity: [u8; 32],
    },
    Provider(ProviderHostKey),
    Runtime {
        resource_identity: [u8; 32],
        lease_identity: uuid::Uuid,
    },
}

impl From<&ProviderHostKey> for SharedWorkKey {
    fn from(key: &ProviderHostKey) -> Self {
        Self::Provider(key.clone())
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct DeliveryWorkKey {
    artifact: String,
    version: String,
    target: String,
    sha256: String,
    form: DeliveryFormIdentity,
}

impl DeliveryWorkKey {
    pub(crate) fn new(
        artifact: impl Into<String>,
        version: impl Into<String>,
        target: impl Into<String>,
        sha256: impl Into<String>,
        form: DeliveryFormIdentity,
    ) -> Result<Self, DeliveryFailure> {
        let key = Self {
            artifact: artifact.into(),
            version: version.into(),
            target: target.into(),
            sha256: sha256.into(),
            form,
        };
        if key.artifact.is_empty()
            || key.version.is_empty()
            || key.target.is_empty()
            || key.sha256.len() != 64
            || !key
                .sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(DeliveryFailure::new(
                DeliveryFailureClass::Configuration,
                "invalid exact delivery identity",
            ));
        }
        Ok(key)
    }

    pub(crate) fn artifact(&self) -> &str {
        &self.artifact
    }
}

impl From<&DeliveryWorkKey> for SharedWorkKey {
    fn from(key: &DeliveryWorkKey) -> Self {
        Self::Delivery {
            artifact: key.artifact.clone(),
            version: key.version.clone(),
            target: key.target.clone(),
            sha256: key.sha256.clone(),
            form: key.form,
        }
    }
}

impl TryFrom<SharedWorkKey> for DeliveryWorkKey {
    type Error = DeliveryFailure;

    fn try_from(key: SharedWorkKey) -> Result<Self, Self::Error> {
        match key {
            SharedWorkKey::Delivery {
                artifact,
                version,
                target,
                sha256,
                form,
            } => Self::new(artifact, version, target, sha256, form),
            _ => Err(DeliveryFailure::new(
                DeliveryFailureClass::Internal,
                "non-delivery shared-work key reached delivery boundary",
            )),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ArtifactReady {
    identity: DeliveryWorkKey,
    install_root: PathBuf,
}

impl ArtifactReady {
    pub(crate) fn new(
        identity: DeliveryWorkKey,
        install_root: PathBuf,
    ) -> Result<Self, DeliveryFailure> {
        if !install_root.is_absolute() {
            return Err(DeliveryFailure::new(
                DeliveryFailureClass::Internal,
                "artifact-ready installation root is not absolute",
            ));
        }
        Ok(Self {
            identity,
            install_root,
        })
    }

    pub(crate) fn identity(&self) -> &DeliveryWorkKey {
        &self.identity
    }

    pub(crate) fn install_root(&self) -> &std::path::Path {
        &self.install_root
    }
}

#[allow(dead_code)] // Closed classes reserved for real Task22 producer adapters.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DeliveryFailureClass {
    Network,
    Timeout,
    Disk,
    Checksum,
    Configuration,
    Internal,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DeliveryFailure {
    class: DeliveryFailureClass,
    /// Retained only for the byte-compatible V12 projection. Canonical V13
    /// callers project the closed class and never runtime prose.
    legacy_diagnostic: String,
}

#[derive(Clone, Debug, Default)]
pub(crate) enum EngineDeliveryState {
    #[default]
    NotRequired,
    Ready(Arc<ArtifactReady>),
    Working {
        artifact: String,
        received: u64,
        total: Option<u64>,
        poll_interval_ms: Option<u64>,
    },
    Failed {
        artifact: String,
        failure: Arc<DeliveryFailure>,
    },
}

impl DeliveryFailure {
    pub(crate) fn new(class: DeliveryFailureClass, legacy_diagnostic: impl Into<String>) -> Self {
        Self {
            class,
            legacy_diagnostic: legacy_diagnostic.into(),
        }
    }

    #[allow(dead_code)] // Canonical V13 delivery projection consumes the closed class at cutover.
    pub(crate) fn class(&self) -> DeliveryFailureClass {
        self.class
    }

    pub(crate) fn legacy_diagnostic(&self) -> &str {
        &self.legacy_diagnostic
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SharedWorkLifetime {
    /// The last observer owns cancellation of the producer.
    OwnerBound,
    /// The process owns the producer; observers may leave without stopping it.
    ProducerBound,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct SharedWorkProgress {
    pub(crate) completed: u64,
    pub(crate) total: Option<u64>,
}

#[derive(Debug)]
pub(crate) enum SharedWorkError<E> {
    Producer(Arc<E>),
    ProducerPanicked,
    ProducerSpawnFailed,
}

impl<E> SharedWorkError<E> {
    #[cfg(test)]
    pub(crate) fn producer(&self) -> Option<&E> {
        match self {
            Self::Producer(error) => Some(error),
            Self::ProducerPanicked | Self::ProducerSpawnFailed => None,
        }
    }
}

#[derive(Debug)]
pub(crate) enum SharedWorkSnapshot<R, E> {
    Running {
        progress: SharedWorkProgress,
        elapsed: Duration,
    },
    Ready(Arc<R>),
    Failed(Arc<SharedWorkError<E>>),
}

enum SharedWorkState<R, E> {
    Running,
    Finished(Result<Arc<R>, Arc<SharedWorkError<E>>>),
}

struct ProducerSignal {
    cancelled: AtomicBool,
    changed: Mutex<()>,
    wake: Condvar,
    #[cfg(test)]
    before_cancel_wait: Mutex<Option<Box<dyn FnOnce() + Send + 'static>>>,
}

impl ProducerSignal {
    fn new() -> Self {
        Self {
            cancelled: AtomicBool::new(false),
            changed: Mutex::new(()),
            wake: Condvar::new(),
            #[cfg(test)]
            before_cancel_wait: Mutex::new(None),
        }
    }

    fn cancel(&self) {
        // The predicate write and notification must share the waiter's mutex;
        // otherwise cancellation can land after its check but before it sleeps.
        let changed = self
            .changed
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.cancelled.store(true, Ordering::Release);
        self.wake.notify_all();
        drop(changed);
    }
}

pub(crate) struct SharedWorkProducer {
    signal: Arc<ProducerSignal>,
    progress: Arc<Mutex<SharedWorkProgress>>,
}

impl SharedWorkProducer {
    #[allow(dead_code)] // Owner-bound Task 11 producers need a prompt cancellation wait seam.
    pub(crate) fn is_cancelled(&self) -> bool {
        self.signal.cancelled.load(Ordering::Acquire)
    }

    #[allow(dead_code)] // Owner-bound Task 11 producers need a prompt cancellation wait seam.
    pub(crate) fn wait_cancelled(&self) {
        let mut changed = self
            .signal
            .changed
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while !self.is_cancelled() {
            #[cfg(test)]
            if let Some(hook) = self
                .signal
                .before_cancel_wait
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take()
            {
                hook();
            }
            changed = self
                .signal
                .wake
                .wait(changed)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
    }

    pub(crate) fn report(&self, completed: u64, total: Option<u64>) {
        *self
            .progress
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            SharedWorkProgress { completed, total };
    }
}

struct SharedWorkEntry<R, E> {
    key: SharedWorkKey,
    registry: Weak<SharedWorkRegistry<R, E>>,
    ownership: Mutex<SharedWorkOwnership>,
    lifetime: SharedWorkLifetime,
    signal: Arc<ProducerSignal>,
    progress: Arc<Mutex<SharedWorkProgress>>,
    state: Mutex<SharedWorkState<R, E>>,
    settled: Condvar,
    started: Instant,
}

struct SharedWorkOwnership {
    owners: usize,
    cancellation_requested: bool,
}

impl<R, E> SharedWorkEntry<R, E> {
    fn new(
        key: SharedWorkKey,
        registry: Weak<SharedWorkRegistry<R, E>>,
        lifetime: SharedWorkLifetime,
    ) -> Self {
        Self {
            key,
            registry,
            ownership: Mutex::new(SharedWorkOwnership {
                owners: 1,
                cancellation_requested: false,
            }),
            lifetime,
            signal: Arc::new(ProducerSignal::new()),
            progress: Arc::new(Mutex::new(SharedWorkProgress::default())),
            state: Mutex::new(SharedWorkState::Running),
            settled: Condvar::new(),
            started: Instant::now(),
        }
    }

    fn producer(&self) -> SharedWorkProducer {
        SharedWorkProducer {
            signal: Arc::clone(&self.signal),
            progress: Arc::clone(&self.progress),
        }
    }

    fn finish(this: &Arc<Self>, outcome: Result<Arc<R>, Arc<SharedWorkError<E>>>) {
        let mut state = this
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *state = SharedWorkState::Finished(outcome);
        drop(state);
        this.settled.notify_all();
        Self::retire_if_terminal_and_unowned(this);
    }

    fn retire_if_terminal_and_unowned(this: &Arc<Self>) {
        let terminal = matches!(
            *this
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            SharedWorkState::Finished(_)
        );
        let unowned = this
            .ownership
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .owners
            == 0;
        if !terminal || !unowned {
            return;
        }
        let Some(registry) = this.registry.upgrade() else {
            return;
        };
        #[cfg(test)]
        if let Some(hook) = registry
            .before_terminal_retire_registry
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
        {
            hook();
        }
        let mut entries = registry
            .entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let still_current = entries
            .get(&this.key)
            .and_then(Weak::upgrade)
            .is_some_and(|current| Arc::ptr_eq(&current, this));
        if !still_current {
            return;
        }
        // Registry -> ownership is the same lock order as join. The first
        // unowned observation is only a fast path; this recheck is the
        // linearization point for terminal retirement.
        let still_unowned = this
            .ownership
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .owners
            == 0;
        if still_unowned {
            entries.remove(&this.key);
        }
    }
}

struct SharedWorkRegistry<R, E> {
    entries: Mutex<HashMap<SharedWorkKey, Weak<SharedWorkEntry<R, E>>>>,
    #[cfg(test)]
    producer_spawner: Mutex<Option<Box<ProducerSpawner>>>,
    #[cfg(test)]
    before_existing_owner_attach: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
    #[cfg(test)]
    before_terminal_retire_registry: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
}

impl<R, E> Default for SharedWorkRegistry<R, E> {
    fn default() -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
            #[cfg(test)]
            producer_spawner: Mutex::new(None),
            #[cfg(test)]
            before_existing_owner_attach: Mutex::new(None),
            #[cfg(test)]
            before_terminal_retire_registry: Mutex::new(None),
        }
    }
}

type ProducerTask = Box<dyn FnOnce() + Send + 'static>;

#[cfg(test)]
type ProducerSpawner = dyn FnOnce(ProducerTask) -> io::Result<()> + Send + 'static;

fn spawn_producer(task: ProducerTask) -> io::Result<()> {
    std::thread::Builder::new()
        .name("unica-shared-work".to_string())
        .spawn(task)
        .map(drop)
}

impl<R, E> SharedWorkRegistry<R, E> {
    fn spawn_producer(&self, task: ProducerTask) -> io::Result<()> {
        #[cfg(test)]
        if let Some(spawn) = self
            .producer_spawner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
        {
            return spawn(task);
        }
        spawn_producer(task)
    }
}

/// A typed exact-key single-flight coordinator.
///
/// Admission never waits: the producer always runs outside the registry lock
/// and every caller receives a lease immediately. Waiting is an explicit later
/// action, after request admission can be released or projected to a Task.
pub(crate) struct SharedWork<R, E> {
    lifetime: SharedWorkLifetime,
    registry: Arc<SharedWorkRegistry<R, E>>,
}

/// Closed failure classes for the daemon-owned readiness coordinators. The
/// producer's raw provider/index/runtime text remains behind its owning
/// adapter and is never fanned out as coordinator state.
#[allow(dead_code)] // Closed classes reserved for real Task22 producer adapters.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LongWorkFailure {
    Invalidated,
    Unavailable,
}

macro_rules! exact_owner {
    ($name:ident, $key:ty, $lifetime:expr) => {
        pub(crate) struct $name {
            shared: SharedWork<(), LongWorkFailure>,
        }

        impl Default for $name {
            fn default() -> Self {
                Self {
                    shared: SharedWork::new($lifetime),
                }
            }
        }

        impl $name {
            pub(crate) fn join_or_start<W>(
                &self,
                key: $key,
                work: W,
            ) -> SharedWorkLease<(), LongWorkFailure>
            where
                W: FnOnce(SharedWorkProducer) -> Result<(), LongWorkFailure> + Send + 'static,
            {
                self.shared.join_or_start(SharedWorkKey::from(&key), work)
            }
        }
    };
}

// Index and provider readiness may finish after an individual follower drops;
// runtime execution remains lease-owned and requests cancellation when its
// final exact owner disappears. Cross-process authorities remain outside these
// in-process coordinators.
exact_owner!(
    ProviderHostOwner,
    ProviderHostKey,
    SharedWorkLifetime::ProducerBound
);

impl<R, E> SharedWork<R, E>
where
    R: Send + Sync + 'static,
    E: Send + Sync + 'static,
{
    pub(crate) fn new(lifetime: SharedWorkLifetime) -> Self {
        Self {
            lifetime,
            registry: Arc::new(SharedWorkRegistry::default()),
        }
    }

    pub(crate) fn join_or_start<W>(&self, key: SharedWorkKey, work: W) -> SharedWorkLease<R, E>
    where
        W: FnOnce(SharedWorkProducer) -> Result<R, E> + Send + 'static,
    {
        let mut entries = self
            .registry
            .entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        entries.retain(|_, entry| entry.strong_count() > 0);
        if let Some(entry) = entries.get(&key).and_then(Weak::upgrade) {
            #[cfg(test)]
            if let Some(hook) = self
                .registry
                .before_existing_owner_attach
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone()
            {
                hook();
            }
            entry
                .ownership
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .owners += 1;
            return SharedWorkLease {
                entry,
                started_here: false,
            };
        }

        let entry = Arc::new(SharedWorkEntry::new(
            key.clone(),
            Arc::downgrade(&self.registry),
            self.lifetime,
        ));
        entries.insert(key, Arc::downgrade(&entry));
        drop(entries);

        let running = Arc::clone(&entry);
        let task = Box::new(move || {
            let outcome = catch_unwind(AssertUnwindSafe(|| work(running.producer())))
                .map_err(|_| SharedWorkError::ProducerPanicked)
                .and_then(|outcome| {
                    outcome.map_err(|error| SharedWorkError::Producer(Arc::new(error)))
                })
                .map(Arc::new)
                .map_err(Arc::new);
            SharedWorkEntry::finish(&running, outcome);
        });
        if self.registry.spawn_producer(task).is_err() {
            SharedWorkEntry::finish(&entry, Err(Arc::new(SharedWorkError::ProducerSpawnFailed)));
        }

        SharedWorkLease {
            entry,
            started_here: true,
        }
    }

    #[cfg(test)]
    fn set_before_existing_owner_attach_for_test(&self, hook: impl Fn() + Send + Sync + 'static) {
        *self
            .registry
            .before_existing_owner_attach
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Arc::new(hook));
    }

    #[cfg(test)]
    fn set_before_terminal_retire_registry_for_test(
        &self,
        hook: impl Fn() + Send + Sync + 'static,
    ) {
        *self
            .registry
            .before_terminal_retire_registry
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Arc::new(hook));
    }

    #[cfg(test)]
    fn set_producer_spawner_for_test(
        &self,
        spawn: impl FnOnce(ProducerTask) -> io::Result<()> + Send + 'static,
    ) {
        *self
            .registry
            .producer_spawner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Box::new(spawn));
    }
}

pub(crate) struct SharedWorkLease<R, E> {
    entry: Arc<SharedWorkEntry<R, E>>,
    started_here: bool,
}

impl<R, E> SharedWorkLease<R, E> {
    #[cfg(test)]
    pub(crate) fn snapshot(&self) -> SharedWorkSnapshot<R, E> {
        let state = self
            .entry
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match &*state {
            SharedWorkState::Running => SharedWorkSnapshot::Running {
                progress: *self
                    .entry
                    .progress
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
                elapsed: self.entry.started.elapsed(),
            },
            SharedWorkState::Finished(Ok(result)) => SharedWorkSnapshot::Ready(Arc::clone(result)),
            SharedWorkState::Finished(Err(error)) => SharedWorkSnapshot::Failed(Arc::clone(error)),
        }
    }

    #[allow(dead_code)] // Canonical daemon handlers join through this bounded ownership seam.
    pub(crate) fn wait(self) -> Result<Arc<R>, Arc<SharedWorkError<E>>> {
        let mut state = self
            .entry
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        loop {
            match &*state {
                SharedWorkState::Finished(result) => return result.clone(),
                SharedWorkState::Running => {
                    state = self
                        .entry
                        .settled
                        .wait(state)
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                }
            }
        }
    }

    pub(crate) fn started_here(&self) -> bool {
        self.started_here
    }

    pub(crate) fn wait_timeout(&self, timeout: Duration) -> SharedWorkSnapshot<R, E> {
        let state = self
            .entry
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let state = if matches!(*state, SharedWorkState::Running) && !timeout.is_zero() {
            self.entry
                .settled
                .wait_timeout(state, timeout)
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .0
        } else {
            state
        };
        match &*state {
            SharedWorkState::Running => SharedWorkSnapshot::Running {
                progress: *self
                    .entry
                    .progress
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
                elapsed: self.entry.started.elapsed(),
            },
            SharedWorkState::Finished(Ok(result)) => SharedWorkSnapshot::Ready(Arc::clone(result)),
            SharedWorkState::Finished(Err(error)) => SharedWorkSnapshot::Failed(Arc::clone(error)),
        }
    }
}

impl<R, E> Drop for SharedWorkLease<R, E> {
    fn drop(&mut self) {
        let request_cancellation = {
            let mut ownership = self
                .entry
                .ownership
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            debug_assert!(ownership.owners > 0, "shared work lease count underflow");
            ownership.owners -= 1;
            if ownership.owners == 0
                && self.entry.lifetime == SharedWorkLifetime::OwnerBound
                && !ownership.cancellation_requested
            {
                ownership.cancellation_requested = true;
                true
            } else {
                false
            }
        };
        if request_cancellation {
            self.entry.signal.cancel();
        }
        SharedWorkEntry::retire_if_terminal_and_unowned(&self.entry);
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DeliveryFormIdentity, ProviderHostKey, SharedWork, SharedWorkKey, SharedWorkLifetime,
        SharedWorkSnapshot,
    };

    use std::collections::BTreeSet;
    use std::io;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{mpsc, Arc, Barrier};
    use std::time::Duration;

    fn delivery(sha256: &str) -> SharedWorkKey {
        SharedWorkKey::Delivery {
            artifact: "rlm-tools-bsl".to_string(),
            version: "1.33.0".to_string(),
            target: "aarch64-apple-darwin".to_string(),
            sha256: sha256.to_string(),
            form: DeliveryFormIdentity::Archive,
        }
    }

    #[test]
    fn exact_key_vocabulary_covers_delivery_index_provider_and_runtime() {
        let delivery = delivery(&"a".repeat(64));
        let index = SharedWorkKey::Index { identity: [2; 32] };
        let provider = SharedWorkKey::from(
            &ProviderHostKey::new(
                "bsl-analyzer",
                "aarch64-apple-darwin",
                BTreeSet::from(["diagnostics".to_string(), "search".to_string()]),
            )
            .unwrap(),
        );
        let runtime = SharedWorkKey::Runtime {
            resource_identity: [3; 32],
            lease_identity: uuid::Uuid::new_v4(),
        };

        assert_ne!(delivery, index);
        assert_ne!(index, provider);
        assert_ne!(provider, runtime);
    }

    #[test]
    fn typed_provider_and_runtime_keys_reject_weak_identity_and_remain_exact() {
        assert!(
            ProviderHostKey::new("bsl-analyzer", "aarch64-apple-darwin", BTreeSet::new(),).is_err()
        );
        assert_ne!(
            ProviderHostKey::new(
                "bsl-analyzer",
                "aarch64-apple-darwin",
                BTreeSet::from(["search".to_string()]),
            )
            .unwrap(),
            ProviderHostKey::new(
                "bsl-analyzer",
                "aarch64-apple-darwin",
                BTreeSet::from(["diagnostics".to_string()]),
            )
            .unwrap()
        );

        assert_ne!(
            SharedWorkKey::Runtime {
                resource_identity: [8; 32],
                lease_identity: uuid::Uuid::new_v4(),
            },
            SharedWorkKey::Runtime {
                resource_identity: [8; 32],
                lease_identity: uuid::Uuid::new_v4(),
            }
        );
    }

    #[test]
    fn one_producer_serves_many_exact_key_followers_and_fans_out_the_result() {
        let work = Arc::new(SharedWork::<usize, &'static str>::new(
            SharedWorkLifetime::OwnerBound,
        ));
        let producers = Arc::new(AtomicUsize::new(0));
        let release = Arc::new(Barrier::new(2));
        let key = delivery(&"b".repeat(64));

        let owner = work.join_or_start(key.clone(), {
            let producers = Arc::clone(&producers);
            let release = Arc::clone(&release);
            move |_| {
                producers.fetch_add(1, Ordering::SeqCst);
                release.wait();
                Ok(17)
            }
        });
        let followers = (0..8)
            .map(|_| {
                work.join_or_start(key.clone(), |_| {
                    panic!("an exact-key follower must not start a second producer")
                })
            })
            .collect::<Vec<_>>();

        release.wait();
        assert_eq!(*owner.wait().expect("owner result"), 17);
        for follower in followers {
            assert_eq!(*follower.wait().expect("follower result"), 17);
        }
        assert_eq!(producers.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn different_exact_keys_do_not_share_and_failure_is_fanned_out() {
        let work = SharedWork::<usize, &'static str>::new(SharedWorkLifetime::OwnerBound);
        let first = work.join_or_start(delivery(&"c".repeat(64)), |_| Err("download failed"));
        let same = work.join_or_start(delivery(&"c".repeat(64)), |_| {
            panic!("same failure must be observed, not restarted")
        });
        let different = work.join_or_start(delivery(&"d".repeat(64)), |_| Ok(29));

        assert_eq!(
            first.wait().unwrap_err().producer(),
            Some(&"download failed")
        );
        assert_eq!(
            same.wait().unwrap_err().producer(),
            Some(&"download failed")
        );
        assert_eq!(*different.wait().expect("different key result"), 29);
    }

    #[test]
    fn producer_spawn_failure_is_terminal_for_the_leader_and_attached_follower() {
        let work = Arc::new(SharedWork::<usize, &'static str>::new(
            SharedWorkLifetime::ProducerBound,
        ));
        let (spawn_entered_tx, spawn_entered_rx) = mpsc::channel();
        let (fail_spawn_tx, fail_spawn_rx) = mpsc::channel();
        work.set_producer_spawner_for_test({
            move |_| {
                spawn_entered_tx.send(()).unwrap();
                fail_spawn_rx
                    .recv_timeout(Duration::from_secs(2))
                    .expect("test releases the injected spawn failure");
                Err(io::Error::other("forced shared-work spawn failure"))
            }
        });

        let key = delivery(&"0".repeat(64));
        let (leader_tx, leader_rx) = mpsc::channel();
        let leader_thread = {
            let work = Arc::clone(&work);
            let key = key.clone();
            std::thread::spawn(move || {
                let leader =
                    work.join_or_start(key, |_| panic!("failed spawn must not run producer work"));
                leader_tx.send(leader).unwrap();
            })
        };
        spawn_entered_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("injected spawner is reached");
        let follower = work.join_or_start(key.clone(), |_| {
            panic!("a follower must attach before spawn failure is published")
        });
        assert!(!follower.started_here());
        fail_spawn_tx.send(()).unwrap();
        let leader = leader_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("spawn failure must return a leader lease without unwind");
        leader_thread
            .join()
            .expect("spawn failure must not unwind caller");

        let SharedWorkSnapshot::Failed(leader_error) = leader.wait_timeout(Duration::from_secs(2))
        else {
            panic!("leader must see terminal spawn failure");
        };
        let SharedWorkSnapshot::Failed(follower_error) =
            follower.wait_timeout(Duration::from_secs(2))
        else {
            panic!("attached follower must see terminal spawn failure");
        };
        assert!(matches!(
            &*leader_error,
            super::SharedWorkError::ProducerSpawnFailed
        ));
        assert!(matches!(
            &*follower_error,
            super::SharedWorkError::ProducerSpawnFailed
        ));
        drop(leader);
        drop(follower);

        let replacement = work.join_or_start(key, |_| Ok(61));
        assert!(replacement.started_here());
        assert!(matches!(
            replacement.wait_timeout(Duration::from_secs(2)),
            SharedWorkSnapshot::Ready(result) if *result == 61
        ));
    }

    #[test]
    fn follower_cancellation_does_not_cancel_a_live_owner() {
        let work = SharedWork::<usize, &'static str>::new(SharedWorkLifetime::OwnerBound);
        let entered = Arc::new(Barrier::new(2));
        let release = Arc::new(Barrier::new(2));
        let key = delivery(&"e".repeat(64));
        let owner = work.join_or_start(key.clone(), {
            let entered = Arc::clone(&entered);
            let release = Arc::clone(&release);
            move |producer| {
                entered.wait();
                release.wait();
                assert!(
                    !producer.is_cancelled(),
                    "follower drop cancelled the owner"
                );
                Ok(31)
            }
        });
        entered.wait();
        let follower = work.join_or_start(key, |_| unreachable!());

        drop(follower);
        release.wait();
        assert_eq!(*owner.wait().expect("owner survives"), 31);
    }

    #[test]
    fn owner_bound_work_is_cancelled_when_the_last_owner_leaves() {
        let work = SharedWork::<usize, &'static str>::new(SharedWorkLifetime::OwnerBound);
        let (entered_tx, entered_rx) = mpsc::channel();
        let (cancelled_tx, cancelled_rx) = mpsc::channel();
        let key = delivery(&"f".repeat(64));
        let owner = work.join_or_start(key.clone(), move |producer| {
            entered_tx.send(()).unwrap();
            producer.wait_cancelled();
            cancelled_tx.send(()).unwrap();
            Err("cancelled")
        });
        entered_rx.recv().unwrap();
        let follower = work.join_or_start(key, |_| unreachable!());

        drop(follower);
        assert!(cancelled_rx.try_recv().is_err());
        drop(owner);
        cancelled_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("last owner cancellation reaches producer");
    }

    #[test]
    fn owner_attach_racing_last_owner_drop_observes_one_retiring_producer() {
        let work = Arc::new(SharedWork::<usize, &'static str>::new(
            SharedWorkLifetime::OwnerBound,
        ));
        let attach_entered = Arc::new(Barrier::new(2));
        let resume_attach = Arc::new(Barrier::new(2));
        work.set_before_existing_owner_attach_for_test({
            let attach_entered = Arc::clone(&attach_entered);
            let resume_attach = Arc::clone(&resume_attach);
            move || {
                attach_entered.wait();
                resume_attach.wait();
            }
        });
        let producers = Arc::new(AtomicUsize::new(0));
        let (started_tx, started_rx) = mpsc::channel();
        let key = delivery(&"2".repeat(64));
        let owner = work.join_or_start(key.clone(), {
            let producers = Arc::clone(&producers);
            move |producer| {
                producers.fetch_add(1, Ordering::SeqCst);
                started_tx.send(()).unwrap();
                producer.wait_cancelled();
                Err("cancelled")
            }
        });
        started_rx.recv().unwrap();

        let follower = {
            let work = Arc::clone(&work);
            let producers = Arc::clone(&producers);
            let key = key.clone();
            std::thread::spawn(move || {
                work.join_or_start(key, move |_| {
                    producers.fetch_add(1, Ordering::SeqCst);
                    Ok(41)
                })
            })
        };
        attach_entered.wait();
        drop(owner);
        resume_attach.wait();

        let follower = follower.join().expect("racing follower");
        assert_eq!(
            follower.wait().unwrap_err().producer(),
            Some(&"cancelled"),
            "the racing owner joins the retiring attempt instead of overlapping it"
        );
        assert_eq!(producers.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn cancelled_attempt_retires_before_a_replacement_producer_starts() {
        let work = SharedWork::<usize, &'static str>::new(SharedWorkLifetime::OwnerBound);
        let (started_tx, started_rx) = mpsc::channel();
        let (cancelled_tx, cancelled_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let producers = Arc::new(AtomicUsize::new(0));
        let key = delivery(&"3".repeat(64));
        let owner = work.join_or_start(key.clone(), {
            let producers = Arc::clone(&producers);
            move |producer| {
                producers.fetch_add(1, Ordering::SeqCst);
                started_tx.send(()).unwrap();
                producer.wait_cancelled();
                cancelled_tx.send(()).unwrap();
                release_rx.recv().unwrap();
                Err("cancelled")
            }
        });
        started_rx.recv().unwrap();
        drop(owner);
        cancelled_rx.recv().unwrap();

        let retiring = work.join_or_start(key.clone(), {
            let producers = Arc::clone(&producers);
            move |_| {
                producers.fetch_add(1, Ordering::SeqCst);
                Ok(43)
            }
        });
        assert_eq!(
            producers.load(Ordering::SeqCst),
            1,
            "a cancelled-but-running exact key must not overlap a replacement"
        );
        release_tx.send(()).unwrap();
        assert_eq!(retiring.wait().unwrap_err().producer(), Some(&"cancelled"));

        let replacement = work.join_or_start(key, {
            let producers = Arc::clone(&producers);
            move |_| {
                producers.fetch_add(1, Ordering::SeqCst);
                Ok(47)
            }
        });
        assert_eq!(*replacement.wait().expect("replacement result"), 47);
        assert_eq!(producers.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn owner_cancellation_cannot_be_lost_between_predicate_check_and_wait() {
        let work = SharedWork::<usize, &'static str>::new(SharedWorkLifetime::OwnerBound);
        let (gap_entered_tx, gap_entered_rx) = mpsc::channel();
        let (release_gap_tx, release_gap_rx) = mpsc::channel();
        let (producer_done_tx, producer_done_rx) = mpsc::channel();
        let (producer_retired_tx, producer_retired_rx) = mpsc::channel();
        let (rescue_tx, rescue_rx) = mpsc::channel();
        let key = delivery(&"4".repeat(64));
        let owner = work.join_or_start(key, move |producer| {
            let rescue_signal = Arc::clone(&producer.signal);
            let rescue = std::thread::spawn(move || {
                rescue_rx.recv().unwrap();
                rescue_signal.wake.notify_all();
            });
            *producer
                .signal
                .before_cancel_wait
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Box::new(move || {
                gap_entered_tx.send(()).unwrap();
                release_gap_rx.recv().unwrap();
            }));
            producer.wait_cancelled();
            producer_done_tx.send(()).unwrap();
            rescue.join().unwrap();
            producer_retired_tx.send(()).unwrap();
            Err("cancelled")
        });
        gap_entered_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("producer reached the cancellation check/wait gap");

        let (drop_started_tx, drop_started_rx) = mpsc::channel();
        let (drop_done_tx, drop_done_rx) = mpsc::channel();
        let cancel = std::thread::spawn(move || {
            drop_started_tx.send(()).unwrap();
            drop(owner);
            drop_done_tx.send(()).unwrap();
        });
        drop_started_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("owner cancellation started");
        let cancellation_completed_in_gap =
            drop_done_rx.recv_timeout(Duration::from_secs(1)).is_ok();

        release_gap_tx.send(()).unwrap();
        let producer_finished_without_rescue = producer_done_rx
            .recv_timeout(Duration::from_secs(1))
            .is_ok();
        rescue_tx.send(()).unwrap();
        if !producer_finished_without_rescue {
            producer_done_rx
                .recv_timeout(Duration::from_secs(1))
                .expect("rescue notification releases the buggy waiter");
        }
        producer_retired_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("producer cleanup completed before the test returned");
        cancel.join().unwrap();

        assert!(
            !cancellation_completed_in_gap,
            "owner cancellation must serialize with the predicate-to-wait transition",
        );
        assert!(
            producer_finished_without_rescue,
            "owner cancellation notification was lost between the predicate check and wait",
        );
    }

    #[test]
    fn terminal_retirement_cannot_remove_an_entry_while_a_new_owner_attaches() {
        let work = Arc::new(SharedWork::<usize, &'static str>::new(
            SharedWorkLifetime::OwnerBound,
        ));
        let retire_entered = Arc::new(Barrier::new(2));
        let resume_retire = Arc::new(Barrier::new(2));
        work.set_before_terminal_retire_registry_for_test({
            let retire_entered = Arc::clone(&retire_entered);
            let resume_retire = Arc::clone(&resume_retire);
            move || {
                retire_entered.wait();
                resume_retire.wait();
            }
        });
        let producers = Arc::new(AtomicUsize::new(0));
        let key = delivery(&"4".repeat(64));
        let owner = work.join_or_start(key.clone(), {
            let producers = Arc::clone(&producers);
            move |_| {
                producers.fetch_add(1, Ordering::SeqCst);
                Ok(53)
            }
        });
        let retire = std::thread::spawn(move || owner.wait().expect("owner result"));
        retire_entered.wait();

        let attached = work.join_or_start(key.clone(), |_| {
            panic!("terminal attach must observe the existing exact result")
        });
        resume_retire.wait();
        assert_eq!(*retire.join().expect("retirement thread"), 53);

        let later = work.join_or_start(key, {
            let producers = Arc::clone(&producers);
            move |_| {
                producers.fetch_add(1, Ordering::SeqCst);
                Ok(59)
            }
        });
        assert_eq!(*attached.wait().expect("attached result"), 53);
        assert_eq!(*later.wait().expect("later result"), 53);
        assert_eq!(
            producers.load(Ordering::SeqCst),
            1,
            "stale retirement removed a newly owned exact entry"
        );
    }

    #[test]
    fn joining_shared_work_never_waits_with_an_admission_permit() {
        struct Admission(Arc<AtomicUsize>);
        impl Drop for Admission {
            fn drop(&mut self) {
                self.0.fetch_sub(1, Ordering::SeqCst);
            }
        }

        let work = SharedWork::<usize, &'static str>::new(SharedWorkLifetime::ProducerBound);
        let key = delivery(&"1".repeat(64));
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let owner = work.join_or_start(key.clone(), move |_| {
            entered_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            Ok(37)
        });
        entered_rx.recv().unwrap();

        let active_admissions = Arc::new(AtomicUsize::new(1));
        let permit = Admission(Arc::clone(&active_admissions));
        let follower = work.join_or_start(key, |_| unreachable!());
        drop(permit);

        assert_eq!(active_admissions.load(Ordering::SeqCst), 0);
        assert!(matches!(
            follower.snapshot(),
            SharedWorkSnapshot::Running { .. }
        ));
        release_tx.send(()).unwrap();
        assert_eq!(*owner.wait().expect("owner result"), 37);
        assert_eq!(*follower.wait().expect("follower result"), 37);
    }

    #[test]
    fn exact_shared_work_keys_fanout_cancellation_and_retirement_are_one_contract() {
        exact_key_vocabulary_covers_delivery_index_provider_and_runtime();
        typed_provider_and_runtime_keys_reject_weak_identity_and_remain_exact();
        one_producer_serves_many_exact_key_followers_and_fans_out_the_result();
        different_exact_keys_do_not_share_and_failure_is_fanned_out();
        producer_spawn_failure_is_terminal_for_the_leader_and_attached_follower();
        follower_cancellation_does_not_cancel_a_live_owner();
        owner_bound_work_is_cancelled_when_the_last_owner_leaves();
        owner_attach_racing_last_owner_drop_observes_one_retiring_producer();
        cancelled_attempt_retires_before_a_replacement_producer_starts();
        owner_cancellation_cannot_be_lost_between_predicate_check_and_wait();
        terminal_retirement_cannot_remove_an_entry_while_a_new_owner_attaches();
        joining_shared_work_never_waits_with_an_admission_permit();
    }
}
