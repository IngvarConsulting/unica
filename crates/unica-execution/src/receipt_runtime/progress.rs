//! Transient observations, never lifecycle authority or provider output.
use crate::invocation_store::EpochMillisClock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use unica_execution_contract::invocation::TaskId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum InvocationPhase {
    Executing,
    ResolvingRunner,
    PreviewingProvider,
    WaitingForRunner,
    ValidatingResult,
}

impl InvocationPhase {
    pub const fn description(self) -> &'static str {
        match self {
            Self::Executing => "Executing invocation",
            Self::ResolvingRunner => "Resolving bundled runner",
            Self::PreviewingProvider => "Waiting for provider preview",
            Self::WaitingForRunner => "Waiting for runner result; platform phase is unknown",
            Self::ValidatingResult => "Validating runner result",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TaskProgress {
    pub phase: InvocationPhase,
    pub phase_started_at_epoch_ms: u64,
    pub execution_elapsed_ms: u64,
    pub phase_elapsed_ms: u64,
}

struct Observation {
    phase: InvocationPhase,
    phase_started_at_epoch_ms: u64,
    started: Instant,
    phase_started: Instant,
}

struct State {
    observation: Mutex<Observation>,
    clock: Arc<dyn EpochMillisClock>,
}

/// A producer can report only a closed phase, never argv, paths or free text.
#[derive(Clone, Default)]
pub struct InvocationProgress(Option<Arc<State>>);

impl InvocationProgress {
    pub fn phase(&self, phase: InvocationPhase) {
        let Some(state) = &self.0 else { return };
        let mut observation = state
            .observation
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if observation.phase != phase {
            observation.phase = phase;
            observation.phase_started = Instant::now();
            observation.phase_started_at_epoch_ms = state.clock.now_epoch_millis();
        }
    }

    fn snapshot(&self) -> Option<TaskProgress> {
        let state = self.0.as_ref()?;
        let observation = state
            .observation
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let now = Instant::now();
        let millis =
            |start| u64::try_from(now.duration_since(start).as_millis()).unwrap_or(u64::MAX);
        Some(TaskProgress {
            phase: observation.phase,
            phase_started_at_epoch_ms: observation.phase_started_at_epoch_ms,
            execution_elapsed_ms: millis(observation.started),
            phase_elapsed_ms: millis(observation.phase_started),
        })
    }
}

#[derive(Default)]
pub(super) struct LiveProgress(Mutex<HashMap<TaskId, InvocationProgress>>);

impl LiveProgress {
    pub(super) fn start(
        &self,
        task_id: Option<TaskId>,
        clock: Arc<dyn EpochMillisClock>,
    ) -> ProgressGuard<'_> {
        let reporter = if let Some(task_id) = task_id {
            let now = Instant::now();
            let reporter = InvocationProgress(Some(Arc::new(State {
                observation: Mutex::new(Observation {
                    phase: InvocationPhase::Executing,
                    phase_started_at_epoch_ms: clock.now_epoch_millis(),
                    started: now,
                    phase_started: now,
                }),
                clock,
            })));
            self.0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .insert(task_id, reporter.clone());
            reporter
        } else {
            InvocationProgress::default()
        };
        ProgressGuard {
            registry: self,
            task_id,
            reporter,
        }
    }

    pub(super) fn snapshot(&self, task_id: TaskId) -> Option<TaskProgress> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&task_id)?
            .snapshot()
    }
}

pub(super) struct ProgressGuard<'a> {
    registry: &'a LiveProgress,
    task_id: Option<TaskId>,
    pub(super) reporter: InvocationProgress,
}

impl Drop for ProgressGuard<'_> {
    fn drop(&mut self) {
        if let Some(task_id) = self.task_id {
            self.registry
                .0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .remove(&task_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    struct Clock(AtomicU64);
    impl EpochMillisClock for Clock {
        fn now_epoch_millis(&self) -> u64 {
            self.0.load(Ordering::Relaxed)
        }
    }

    #[test]
    fn phases_are_not_poll_heartbeats_and_late_reporters_cannot_republish() {
        let clock = Arc::new(Clock(AtomicU64::new(100)));
        let registry = LiveProgress::default();
        let first = TaskId::new();
        let second = TaskId::new();
        let guard = registry.start(Some(first), clock.clone());
        let other = registry.start(Some(second), clock.clone());
        guard.reporter.phase(InvocationPhase::WaitingForRunner);
        let before = registry.snapshot(first).unwrap();
        clock.0.store(200, Ordering::Relaxed);
        guard.reporter.phase(InvocationPhase::WaitingForRunner);
        std::thread::sleep(std::time::Duration::from_millis(3));
        let after = registry.snapshot(first).unwrap();
        assert_eq!(
            before.phase_started_at_epoch_ms,
            after.phase_started_at_epoch_ms
        );
        assert!(after.execution_elapsed_ms > before.execution_elapsed_ms);
        assert!(after.phase_elapsed_ms > before.phase_elapsed_ms);
        assert_eq!(
            registry.snapshot(second).unwrap().phase,
            InvocationPhase::Executing
        );
        assert!(registry.snapshot(TaskId::new()).is_none());
        let late = guard.reporter.clone();
        drop(guard);
        late.phase(InvocationPhase::ValidatingResult);
        assert!(registry.snapshot(first).is_none());
        assert!(registry.snapshot(second).is_some());
        drop(other);
        assert!(registry.0.lock().unwrap().is_empty());
        assert!(LiveProgress::default().snapshot(first).is_none());
    }

    #[test]
    fn unwind_and_inline_attempts_leave_no_observation() {
        let registry = LiveProgress::default();
        let id = TaskId::new();
        let clock = Arc::new(Clock(AtomicU64::new(100)));
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = registry.start(Some(id), clock.clone());
            panic!("provider panic");
        }));
        assert!(registry.snapshot(id).is_none());
        let inline = registry.start(None, clock);
        inline.reporter.phase(InvocationPhase::WaitingForRunner);
        assert!(registry.0.lock().unwrap().is_empty());
    }
}
