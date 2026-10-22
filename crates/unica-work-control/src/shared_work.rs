use std::collections::HashMap;
use std::hash::Hash;
use std::io;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, Weak};
use std::time::{Duration, Instant};

/// Exact identity supplied by the owner; joining never interprets the key.
pub trait WorkKey: Clone + Eq + Hash + Send + Sync + 'static {}
impl<K: Clone + Eq + Hash + Send + Sync + 'static> WorkKey for K {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SharedWorkLifetime {
    /// The last observer owns cancellation of the producer.
    OwnerBound,
    /// The process owns the producer; observers may leave without stopping it.
    ProducerBound,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SharedWorkProgress {
    pub completed: u64,
    pub total: Option<u64>,
}

#[derive(Debug)]
pub enum SharedWorkError<E> {
    Producer(Arc<E>),
    ProducerPanicked,
    ProducerSpawnFailed,
}

impl<E> SharedWorkError<E> {
    #[cfg(test)]
    pub fn producer(&self) -> Option<&E> {
        match self {
            Self::Producer(error) => Some(error),
            Self::ProducerPanicked | Self::ProducerSpawnFailed => None,
        }
    }
}

#[derive(Debug)]
pub enum SharedWorkSnapshot<R, E> {
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
    #[cfg(test)]
    after_cancel: Mutex<Option<Box<dyn FnOnce() + Send + 'static>>>,
}

impl ProducerSignal {
    fn new() -> Self {
        Self {
            cancelled: AtomicBool::new(false),
            changed: Mutex::new(()),
            wake: Condvar::new(),
            #[cfg(test)]
            before_cancel_wait: Mutex::new(None),
            #[cfg(test)]
            after_cancel: Mutex::new(None),
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
        #[cfg(test)]
        {
            let hook = self
                .after_cancel
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            if let Some(hook) = hook {
                hook();
            }
        }
    }
}

pub struct SharedWorkProducer {
    signal: Arc<ProducerSignal>,
    progress: Arc<Mutex<SharedWorkProgress>>,
}

impl SharedWorkProducer {
    #[allow(dead_code)] // Owner-bound Task 11 producers need a prompt cancellation wait seam.
    pub fn is_cancelled(&self) -> bool {
        self.signal.cancelled.load(Ordering::Acquire)
    }

    #[allow(dead_code)] // Owner-bound Task 11 producers need a prompt cancellation wait seam.
    pub fn wait_cancelled(&self) {
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

    pub fn report(&self, completed: u64, total: Option<u64>) {
        *self
            .progress
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            SharedWorkProgress { completed, total };
    }
}

struct SharedWorkEntry<K: WorkKey, R, E> {
    key: K,
    registry: Weak<SharedWorkRegistry<K, R, E>>,
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

impl<K: WorkKey, R, E> SharedWorkEntry<K, R, E> {
    fn new(
        key: K,
        registry: Weak<SharedWorkRegistry<K, R, E>>,
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

type SharedWorkEntries<K, R, E> = Mutex<HashMap<K, Weak<SharedWorkEntry<K, R, E>>>>;

struct SharedWorkRegistry<K: WorkKey, R, E> {
    entries: SharedWorkEntries<K, R, E>,
    #[cfg(test)]
    producer_spawner: Mutex<Option<Box<ProducerSpawner>>>,
    #[cfg(test)]
    before_existing_owner_attach: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
    #[cfg(test)]
    before_terminal_retire_registry: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
}

impl<K: WorkKey, R, E> Default for SharedWorkRegistry<K, R, E> {
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

impl<K: WorkKey, R, E> SharedWorkRegistry<K, R, E> {
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
pub struct SharedWork<K: WorkKey, R, E> {
    lifetime: SharedWorkLifetime,
    registry: Arc<SharedWorkRegistry<K, R, E>>,
}

impl<K: WorkKey, R, E> SharedWork<K, R, E>
where
    R: Send + Sync + 'static,
    E: Send + Sync + 'static,
{
    pub fn new(lifetime: SharedWorkLifetime) -> Self {
        Self {
            lifetime,
            registry: Arc::new(SharedWorkRegistry::default()),
        }
    }

    /// Running producers and retained terminal leases keep their entry alive.
    /// An unreadable registry cannot prove that its owner is safe to release.
    pub fn has_live_entries(&self) -> bool {
        self.registry
            .entries
            .lock()
            .map(|entries| entries.values().any(|entry| entry.strong_count() > 0))
            .unwrap_or(true)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn poison_registry_for_test(&self) {
        let poisoned = catch_unwind(AssertUnwindSafe(|| {
            let _entries = self.registry.entries.lock().unwrap();
            panic!("poison shared-work registry for retention test");
        }));
        assert!(poisoned.is_err());
    }

    pub fn join_or_start<W>(&self, key: K, work: W) -> SharedWorkLease<K, R, E>
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

pub struct SharedWorkLease<K: WorkKey, R, E> {
    entry: Arc<SharedWorkEntry<K, R, E>>,
    started_here: bool,
}

impl<K: WorkKey, R, E> SharedWorkLease<K, R, E> {
    #[cfg(any(test, feature = "test-support"))]
    pub fn snapshot(&self) -> SharedWorkSnapshot<R, E> {
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
    pub fn wait(self) -> Result<Arc<R>, Arc<SharedWorkError<E>>> {
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

    pub fn started_here(&self) -> bool {
        self.started_here
    }

    pub fn wait_timeout(&self, timeout: Duration) -> SharedWorkSnapshot<R, E> {
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

impl<K: WorkKey, R, E> Drop for SharedWorkLease<K, R, E> {
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
    use super::{SharedWork, SharedWorkLifetime, SharedWorkSnapshot};
    use std::io;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{mpsc, Arc, Barrier};
    use std::time::Duration;
    type TestKey = (String, String, String, String, &'static str);
    fn delivery(sha256: &str) -> TestKey {
        (
            "rlm-tools-bsl".into(),
            "1.33.0".into(),
            "aarch64-apple-darwin".into(),
            sha256.into(),
            "archive",
        )
    }
    #[test]
    fn one_producer_serves_many_exact_key_followers_and_fans_out_the_result() {
        let work = Arc::new(SharedWork::<TestKey, usize, &'static str>::new(
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
        let work = SharedWork::<TestKey, usize, &'static str>::new(SharedWorkLifetime::OwnerBound);
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
        let work = Arc::new(SharedWork::<TestKey, usize, &'static str>::new(
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
        let work = SharedWork::<TestKey, usize, &'static str>::new(SharedWorkLifetime::OwnerBound);
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
        let work = SharedWork::<TestKey, usize, &'static str>::new(SharedWorkLifetime::OwnerBound);
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
        let work = Arc::new(SharedWork::<TestKey, usize, &'static str>::new(
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

        let (finished_tx, finished_rx) = mpsc::channel();
        work.set_before_terminal_retire_registry_for_test(move || {
            finished_tx.send(()).unwrap();
        });
        let (cancel_after_finish_tx, cancel_after_finish_rx) = mpsc::channel();
        *owner.entry.signal.after_cancel.lock().unwrap() = Some(Box::new(move || {
            // Only the producer can reach terminal retirement while this Drop
            // is held here, after cancellation and before its own retirement.
            finished_rx.recv().expect("cancelled producer finished");
            cancel_after_finish_tx.send(()).unwrap();
        }));

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
        // The follower holds the registry until the controller releases it.
        // Dropping synchronously here would deadlock once the producer finishes.
        let dropping_owner = std::thread::spawn(move || drop(owner));
        cancel_after_finish_rx
            .recv()
            .expect("owner cancellation observed the finished producer");
        resume_attach.wait();

        dropping_owner.join().expect("last owner drop");
        let follower = follower.join().expect("racing follower");
        assert!(!follower.started_here());
        assert_eq!(
            follower.wait().unwrap_err().producer(),
            Some(&"cancelled"),
            "the racing owner joins the retiring attempt instead of overlapping it"
        );
        assert_eq!(producers.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn cancelled_attempt_retires_before_a_replacement_producer_starts() {
        let work = SharedWork::<TestKey, usize, &'static str>::new(SharedWorkLifetime::OwnerBound);
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
        let work = SharedWork::<TestKey, usize, &'static str>::new(SharedWorkLifetime::OwnerBound);
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
        let work = Arc::new(SharedWork::<TestKey, usize, &'static str>::new(
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

        let work =
            SharedWork::<TestKey, usize, &'static str>::new(SharedWorkLifetime::ProducerBound);
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
}
