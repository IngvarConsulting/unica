use crate::domain::cancellation::{cancelled_error, CancellationToken};
use crate::domain::code_intelligence::ProviderDeadline;
use std::sync::{Mutex, MutexGuard, TryLockError};
use std::time::Duration;

const WAIT_SLICE: Duration = Duration::from_millis(10);

/// One synchronous ownership lane whose contention is bounded by the caller's
/// monotonic deadline and cancellation signal.
pub(crate) struct DeadlineLock {
    inner: Mutex<()>,
}

impl Default for DeadlineLock {
    fn default() -> Self {
        Self {
            inner: Mutex::new(()),
        }
    }
}

impl DeadlineLock {
    pub(crate) fn acquire_before(
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
                    let guard = error.into_inner();
                    checkpoint(deadline, cancellation, operation)?;
                    return Ok(guard);
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
    pub(crate) fn hold_for_test(&self) -> MutexGuard<'_, ()> {
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
