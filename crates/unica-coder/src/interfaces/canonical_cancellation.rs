//! Manual cancellation and its exact daemon control acknowledgement.
use crate::domain::cancellation::CancellationToken;
use std::sync::{Arc, Condvar, Mutex};

type Control = Arc<dyn Fn() -> Result<(), ControlFailure> + Send + Sync>;

#[derive(Clone, Debug)]
pub(super) enum ControlFailure {
    BeforeSend(String),
    Uncertain(String),
}

impl std::fmt::Display for ControlFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BeforeSend(message) => write!(f, "control was not sent: {message}"),
            Self::Uncertain(message) => write!(f, "control outcome is unknown: {message}"),
        }
    }
}

#[derive(Clone)]
pub(super) struct CanonicalCancellation {
    token: CancellationToken,
    state: Arc<(Mutex<State>, Condvar)>,
}

#[derive(Default)]
struct State {
    requested: bool,
    control: Option<Control>,
    started: bool,
    settled: Option<Result<(), ControlFailure>>,
}

#[derive(Clone)]
pub(super) struct ControlJob {
    cancellation: CanonicalCancellation,
    control: Control,
}

impl Default for CanonicalCancellation {
    fn default() -> Self {
        Self::from(CancellationToken::new())
    }
}

impl From<CancellationToken> for CanonicalCancellation {
    fn from(token: CancellationToken) -> Self {
        Self {
            token,
            state: Arc::new((Mutex::new(State::default()), Condvar::new())),
        }
    }
}

impl CanonicalCancellation {
    pub(super) fn token(&self) -> CancellationToken {
        self.token.clone()
    }

    pub(super) fn request(&self) -> Option<ControlJob> {
        let mut state = self
            .state
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.requested = true;
        self.token.cancel();
        self.claim_control(&mut state)
    }

    fn claim_control(&self, state: &mut State) -> Option<ControlJob> {
        if !state.requested || state.started {
            return None;
        }
        let control = state.control.clone()?;
        state.started = true;
        Some(ControlJob {
            cancellation: self.clone(),
            control,
        })
    }

    /// Publishing the key precedes both the test observer and Submit. A manual
    /// request already present here must be durably confirmed before Submit.
    pub(super) fn bind_control(&self, control: Control) -> Result<(), ControlFailure> {
        let job = {
            let mut state = self
                .state
                .0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.control = Some(control);
            self.claim_control(&mut state)
        };
        if let Some(job) = job {
            job.run()?;
        }
        let mut state = self
            .state
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while state.requested && state.settled.is_none() {
            state = self
                .state
                .1
                .wait(state)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        state.settled.clone().unwrap_or(Ok(()))
    }

    pub(super) fn requested(&self) -> bool {
        self.state
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .requested
    }

    pub(super) fn pending_control(&self) -> bool {
        let state = self
            .state
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.requested && state.control.is_some() && state.settled.is_none()
    }
}

impl ControlJob {
    pub(super) fn run(self) -> Result<(), ControlFailure> {
        let mut completion = ControlCompletion {
            job: self.clone(),
            finished: false,
        };
        let result = (self.control)();
        self.settle(result.clone());
        completion.finished = true;
        result
    }

    pub(super) fn fail_before_send(&self, message: String) {
        self.settle(Err(ControlFailure::BeforeSend(message)));
    }

    fn settle(&self, result: Result<(), ControlFailure>) {
        self.cancellation
            .state
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .settled = Some(result);
        self.cancellation.state.1.notify_all();
    }
}

struct ControlCompletion {
    job: ControlJob,
    finished: bool,
}

impl Drop for ControlCompletion {
    fn drop(&mut self) {
        if !self.finished {
            self.job.settle(Err(ControlFailure::Uncertain(
                "control worker ended before confirming its outcome".to_owned(),
            )));
        }
    }
}
