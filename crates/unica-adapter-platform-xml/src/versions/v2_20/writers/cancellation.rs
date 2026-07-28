use std::cell::RefCell;

use unica_format_core::ports::OperationCancellation;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CancellationOutcome {
    DuringExecution,
    DuringPublicationRolledBack,
    RecoveryRequired,
}

#[derive(Clone)]
struct ActiveCancellation {
    cancellation: OperationCancellation,
    observed: bool,
    publication_started: bool,
    rollback_completed: Option<bool>,
}

thread_local! {
    static ACTIVE: RefCell<Option<ActiveCancellation>> = const { RefCell::new(None) };
}

pub(crate) fn with_cancellation<T>(
    cancellation: &OperationCancellation,
    action: impl FnOnce() -> T,
) -> T {
    struct Reset(Option<ActiveCancellation>);
    impl Drop for Reset {
        fn drop(&mut self) {
            ACTIVE.with(|slot| {
                slot.replace(self.0.take());
            });
        }
    }

    let previous = ACTIVE.with(|slot| {
        slot.replace(Some(ActiveCancellation {
            cancellation: cancellation.clone(),
            observed: false,
            publication_started: false,
            rollback_completed: None,
        }))
    });
    let _reset = Reset(previous);
    action()
}

pub(crate) fn checkpoint() -> Result<(), String> {
    let cancelled = ACTIVE.with(|slot| {
        let mut slot = slot.borrow_mut();
        let Some(active) = slot.as_mut() else {
            return false;
        };
        if active.cancellation.is_cancelled() {
            active.observed = true;
            true
        } else {
            false
        }
    });
    if cancelled {
        Err("operation cancelled at a safe publication checkpoint".to_string())
    } else {
        Ok(())
    }
}

pub(crate) fn publication_started() {
    ACTIVE.with(|slot| {
        if let Some(active) = slot.borrow_mut().as_mut() {
            active.publication_started = true;
        }
    });
}

pub(crate) fn publication_rollback_completed(recovery_required: bool) {
    ACTIVE.with(|slot| {
        if let Some(active) = slot.borrow_mut().as_mut() {
            if active.observed && active.publication_started {
                active.rollback_completed = Some(!recovery_required);
            }
        }
    });
}

pub(crate) fn outcome() -> Option<CancellationOutcome> {
    ACTIVE.with(|slot| {
        let slot = slot.borrow();
        let active = slot.as_ref()?;
        if !active.observed {
            return None;
        }
        match (active.publication_started, active.rollback_completed) {
            (true, Some(true)) => Some(CancellationOutcome::DuringPublicationRolledBack),
            (true, Some(false)) => Some(CancellationOutcome::RecoveryRequired),
            _ => Some(CancellationOutcome::DuringExecution),
        }
    })
}

#[cfg(test)]
pub(crate) fn cancel_active_for_test() {
    ACTIVE.with(|slot| {
        if let Some(active) = slot.borrow().as_ref() {
            active.cancellation.cancel();
        }
    });
}
