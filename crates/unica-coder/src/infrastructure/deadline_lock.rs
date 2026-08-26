use crate::domain::cancellation::{cancelled_error, CancellationToken};
use crate::domain::code_intelligence::ProviderDeadline;
use std::sync::{Mutex, MutexGuard, PoisonError, TryLockError};
use std::time::Duration;

#[cfg(test)]
use std::cell::RefCell;

const WAIT_SLICE: Duration = Duration::from_millis(10);

#[cfg(test)]
thread_local! {
    static TEST_AFTER_DEADLINE_ERROR_HOOK: RefCell<Option<Box<dyn FnOnce()>>> = const { RefCell::new(None) };
}

#[cfg(test)]
pub(super) fn set_after_deadline_error_hook_for_test(hook: impl FnOnce() + 'static) {
    TEST_AFTER_DEADLINE_ERROR_HOOK.with(|slot| slot.replace(Some(Box::new(hook))));
}

/// One synchronous ownership lane whose contention is bounded by the caller's
/// monotonic deadline and cancellation signal.
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
                    let remaining = deadline.remaining();
                    if remaining.is_zero() {
                        return Err(deadline_error(operation));
                    }
                    std::thread::sleep(remaining.min(WAIT_SLICE));
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
    if deadline.remaining().is_zero() {
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
