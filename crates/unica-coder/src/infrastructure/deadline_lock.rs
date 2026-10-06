use crate::domain::cancellation::{cancelled_error, CancellationToken};
use crate::domain::code_intelligence::ProviderDeadline;
use std::sync::{Mutex, MutexGuard, PoisonError, TryLockError};
use std::time::Duration;

#[cfg(test)]
use std::cell::RefCell;

const WAIT_SLICE: Duration = Duration::from_millis(10);

#[cfg(test)]
thread_local! {
    static TEST_BEFORE_WAIT_HOOK: RefCell<Option<Box<dyn FnOnce()>>> = const { RefCell::new(None) };
    static TEST_AFTER_DEADLINE_ERROR_HOOK: RefCell<Option<Box<dyn FnOnce()>>> = const { RefCell::new(None) };
}

#[cfg(test)]
pub(super) fn set_after_deadline_error_hook_for_test(hook: impl FnOnce() + 'static) {
    TEST_AFTER_DEADLINE_ERROR_HOOK.with(|slot| slot.replace(Some(Box::new(hook))));
}

#[cfg(test)]
pub(super) fn set_before_wait_hook_for_test(hook: impl FnOnce() + 'static) {
    TEST_BEFORE_WAIT_HOOK.with(|slot| slot.replace(Some(Box::new(hook))));
}

#[cfg(test)]
fn run_before_wait_hook_for_test() {
    if let Some(hook) = TEST_BEFORE_WAIT_HOOK.with(|slot| slot.borrow_mut().take()) {
        hook();
    }
}

/// One synchronous ownership lane that observes the caller's cancellation
/// signal and an explicit monotonic deadline when one is present.
pub(super) trait PoisonPolicy {
    fn resolve<'a>(
        &self,
        poisoned: PoisonError<MutexGuard<'a, ()>>,
    ) -> Result<MutexGuard<'a, ()>, DeadlineLockError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DeadlineLockErrorKind {
    Cancelled,
    Deadline,
    Poisoned,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DeadlineLockError {
    kind: DeadlineLockErrorKind,
    message: String,
}

impl DeadlineLockError {
    fn new(kind: DeadlineLockErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub(super) fn kind(&self) -> DeadlineLockErrorKind {
        self.kind
    }
}

impl std::fmt::Display for DeadlineLockError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.message.fmt(formatter)
    }
}

impl std::error::Error for DeadlineLockError {}

pub(super) struct FailClosed {
    error: &'static str,
}

impl FailClosed {
    const fn new(error: &'static str) -> Self {
        Self { error }
    }
}

impl PoisonPolicy for FailClosed {
    fn resolve<'a>(
        &self,
        _poisoned: PoisonError<MutexGuard<'a, ()>>,
    ) -> Result<MutexGuard<'a, ()>, DeadlineLockError> {
        Err(DeadlineLockError::new(
            DeadlineLockErrorKind::Poisoned,
            self.error,
        ))
    }
}

pub(super) struct Recover;

impl PoisonPolicy for Recover {
    fn resolve<'a>(
        &self,
        poisoned: PoisonError<MutexGuard<'a, ()>>,
    ) -> Result<MutexGuard<'a, ()>, DeadlineLockError> {
        Ok(poisoned.into_inner())
    }
}

pub(super) struct DeadlineLock<P: PoisonPolicy> {
    inner: Mutex<()>,
    poison_policy: P,
}

impl Default for DeadlineLock<Recover> {
    fn default() -> Self {
        Self {
            inner: Mutex::new(()),
            poison_policy: Recover,
        }
    }
}

impl DeadlineLock<FailClosed> {
    pub(super) const fn fail_closed(error: &'static str) -> Self {
        Self {
            inner: Mutex::new(()),
            poison_policy: FailClosed::new(error),
        }
    }
}

impl<P: PoisonPolicy> DeadlineLock<P> {
    pub(super) fn acquire_before(
        &self,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
        operation: &'static str,
    ) -> Result<MutexGuard<'_, ()>, DeadlineLockError> {
        loop {
            checkpoint(deadline, cancellation, operation)?;
            match self.inner.try_lock() {
                Ok(guard) => {
                    checkpoint(deadline, cancellation, operation)?;
                    return Ok(guard);
                }
                Err(TryLockError::Poisoned(error)) => {
                    checkpoint(deadline, cancellation, operation)?;
                    return self.poison_policy.resolve(error);
                }
                Err(TryLockError::WouldBlock) => {
                    if deadline.is_elapsed() {
                        return Err(deadline_error(operation));
                    }
                    #[cfg(test)]
                    run_before_wait_hook_for_test();
                    std::thread::sleep(deadline.wait_slice(WAIT_SLICE));
                }
            }
        }
    }

    #[cfg(test)]
    pub(super) fn hold_for_test(&self) -> MutexGuard<'_, ()> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

fn checkpoint(
    deadline: ProviderDeadline,
    cancellation: &CancellationToken,
    operation: &'static str,
) -> Result<(), DeadlineLockError> {
    if cancellation.is_cancelled() {
        return Err(DeadlineLockError::new(
            DeadlineLockErrorKind::Cancelled,
            cancelled_error(format!("{operation} stopped")),
        ));
    }
    if deadline.is_elapsed() {
        return Err(deadline_error(operation));
    }
    Ok(())
}

fn deadline_error(operation: &str) -> DeadlineLockError {
    let error = DeadlineLockError::new(
        DeadlineLockErrorKind::Deadline,
        format!("{operation} deadline exceeded"),
    );
    #[cfg(test)]
    if let Some(hook) = TEST_AFTER_DEADLINE_ERROR_HOOK.with(|slot| slot.borrow_mut().take()) {
        hook();
    }
    error
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{mpsc, Arc};

    #[test]
    fn no_deadline_waits_on_occupied_lane_then_acquires_after_release() {
        let lock = Arc::new(DeadlineLock::<Recover>::default());
        std::thread::scope(|scope| {
            let held = lock.hold_for_test();
            let (entered_tx, entered_rx) = mpsc::channel();
            let (release_tx, release_rx) = mpsc::channel();
            let (result_tx, result_rx) = mpsc::channel();
            let worker_lock = Arc::clone(&lock);
            let worker = scope.spawn(move || {
                set_before_wait_hook_for_test(move || {
                    entered_tx.send(()).unwrap();
                    release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
                });
                let acquired = worker_lock.acquire_before(
                    ProviderDeadline::no_deadline(),
                    &CancellationToken::new(),
                    "no deadline lane",
                );
                result_tx.send(acquired.is_ok()).unwrap();
            });
            entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            assert!(matches!(
                result_rx.try_recv(),
                Err(mpsc::TryRecvError::Empty)
            ));
            drop(held);
            release_tx.send(()).unwrap();
            assert!(result_rx.recv_timeout(Duration::from_secs(5)).unwrap());
            worker.join().unwrap();
        });
    }

    #[test]
    fn no_deadline_occupied_lane_still_observes_explicit_cancellation() {
        let lock = Arc::new(DeadlineLock::<Recover>::default());
        let cancellation = CancellationToken::new();
        std::thread::scope(|scope| {
            let _held = lock.hold_for_test();
            let (entered_tx, entered_rx) = mpsc::channel();
            let (release_tx, release_rx) = mpsc::channel();
            let (result_tx, result_rx) = mpsc::channel();
            let worker_lock = Arc::clone(&lock);
            let worker_cancellation = cancellation.clone();
            let worker = scope.spawn(move || {
                set_before_wait_hook_for_test(move || {
                    entered_tx.send(()).unwrap();
                    release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
                });
                let result = worker_lock.acquire_before(
                    ProviderDeadline::no_deadline(),
                    &worker_cancellation,
                    "cancel no deadline lane",
                );
                result_tx.send(result.err().unwrap().kind()).unwrap();
            });
            entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            cancellation.cancel();
            release_tx.send(()).unwrap();
            assert_eq!(
                result_rx.recv_timeout(Duration::from_secs(5)).unwrap(),
                DeadlineLockErrorKind::Cancelled
            );
            worker.join().unwrap();
        });
    }
}
