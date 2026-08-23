use crate::domain::cancellation::{cancelled_error, CancellationToken};
use crate::domain::code_intelligence::ProviderDeadline;
use std::sync::{Mutex, MutexGuard, PoisonError, TryLockError};
use std::time::Duration;

const WAIT_SLICE: Duration = Duration::from_millis(10);

/// One synchronous ownership lane whose contention is bounded by the caller's
/// monotonic deadline and cancellation signal.
pub(super) trait PoisonPolicy {
    fn resolve<'a>(
        &self,
        poisoned: PoisonError<MutexGuard<'a, ()>>,
    ) -> Result<MutexGuard<'a, ()>, String>;
}

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
    ) -> Result<MutexGuard<'a, ()>, String> {
        Err(self.error.to_string())
    }
}

pub(super) struct Recover;

impl PoisonPolicy for Recover {
    fn resolve<'a>(
        &self,
        poisoned: PoisonError<MutexGuard<'a, ()>>,
    ) -> Result<MutexGuard<'a, ()>, String> {
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
    ) -> Result<MutexGuard<'_, ()>, String> {
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
) -> Result<(), String> {
    if cancellation.is_cancelled() {
        return Err(cancelled_error(format!("{operation} stopped")));
    }
    if deadline.remaining().is_zero() {
        return Err(deadline_error(operation));
    }
    Ok(())
}

fn deadline_error(operation: &str) -> String {
    format!("{operation} deadline exceeded")
}
