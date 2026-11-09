//! Typed MCP request ownership through execution, control and response flush.
use super::{CanonicalCancellation, InFlightGuard};
use rmcp::model::RequestId;
use std::collections::HashMap;
use std::io::Write;
use std::sync::{Arc, Condvar, Mutex};

#[derive(Default)]
pub(super) struct ManualCalls {
    calls: Mutex<CallOwners>,
    changed: Condvar,
    transport_closed: std::sync::atomic::AtomicBool,
    close_notice: tokio::sync::Notify,
    #[cfg(test)]
    before_handler: Mutex<Option<Arc<HandlerGate>>>,
}

#[derive(Default)]
struct CallOwners {
    current: HashMap<RequestId, Arc<ManualCall>>,
    retiring: Vec<(RequestId, Arc<ManualCall>)>,
}

impl CallOwners {
    fn entries(&self) -> impl Iterator<Item = (&RequestId, &Arc<ManualCall>)> {
        self.current
            .iter()
            .chain(self.retiring.iter().map(|(id, call)| (id, call)))
    }
}

#[cfg(test)]
#[derive(Default)]
pub(super) struct HandlerGate {
    entered: std::sync::atomic::AtomicBool,
    released: std::sync::atomic::AtomicBool,
    entered_notice: tokio::sync::Notify,
    released_notice: tokio::sync::Notify,
}

#[cfg(test)]
impl HandlerGate {
    pub(super) async fn wait(&self) -> bool {
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            loop {
                let notified = self.entered_notice.notified();
                if self.entered.load(std::sync::atomic::Ordering::Acquire) {
                    break;
                }
                notified.await;
            }
        })
        .await
        .is_ok()
    }

    pub(super) fn release(&self) {
        self.released
            .store(true, std::sync::atomic::Ordering::Release);
        self.released_notice.notify_waiters();
    }

    async fn hold(&self) {
        self.entered
            .store(true, std::sync::atomic::Ordering::Release);
        self.entered_notice.notify_waiters();
        loop {
            let notified = self.released_notice.notified();
            if self.released.load(std::sync::atomic::Ordering::Acquire) {
                break;
            }
            notified.await;
        }
    }
}

#[cfg(test)]
impl ManualCalls {
    pub(super) fn install_handler_gate(&self, gate: Arc<HandlerGate>) {
        *self.before_handler.lock().unwrap() = Some(gate);
    }

    pub(super) async fn before_handler(&self) {
        let gate = self.before_handler.lock().unwrap().clone();
        if let Some(gate) = gate {
            gate.hold().await;
        }
    }
}

#[cfg(not(test))]
impl ManualCalls {
    pub(super) async fn before_handler(&self) {}
}

pub(super) struct ManualCall {
    pub(super) cancellation: CanonicalCancellation,
    state: Mutex<CallState>,
}

#[derive(Default)]
struct CallState {
    handler_started: bool,
    handler_done: bool,
    delivery_done: bool,
    response_seen: bool,
    dispatch_done: bool,
    admission: Option<InFlightGuard>,
}

pub(super) struct DispatchOwner {
    registry: Arc<ManualCalls>,
    id: RequestId,
    call: Arc<ManualCall>,
}

pub(super) struct HandlerOwner {
    registry: Arc<ManualCalls>,
    id: RequestId,
    call: Arc<ManualCall>,
}

impl ManualCalls {
    pub(super) fn transport_closed(&self) {
        self.closed();
        self.transport_closed
            .store(true, std::sync::atomic::Ordering::Release);
        self.close_notice.notify_waiters();
    }

    pub(super) async fn wait_transport_closed(&self) {
        loop {
            let closed = self.close_notice.notified();
            if self
                .transport_closed
                .load(std::sync::atomic::Ordering::Acquire)
            {
                break;
            }
            closed.await;
        }
    }

    pub(super) async fn initialization_failed(&self) {
        self.wait_transport_closed().await;
        let entries: Vec<_> = {
            let calls = self
                .calls
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            calls
                .entries()
                .map(|(id, call)| {
                    let mut state = call
                        .state
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    state.dispatch_done = true;
                    (id.clone(), call.clone())
                })
                .collect()
        };
        for (id, call) in entries {
            self.retire(&id, &call);
        }
    }

    /// A modern sender may reuse an ID after receiving its complete response,
    /// even while the previous send/control still owns its real completion.
    /// Keep those older owners separate from the new cancellation target.
    pub(super) fn admit(&self, id: RequestId, track_call: bool) -> bool {
        let mut calls = self
            .calls
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(previous) = calls.current.get(&id) {
            if !previous
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .response_seen
            {
                return false;
            }
        }
        if let Some(previous) = calls.current.remove(&id) {
            calls.retiring.push((id.clone(), previous));
        }
        if track_call {
            calls.current.insert(
                id,
                Arc::new(ManualCall {
                    cancellation: CanonicalCancellation::default(),
                    state: Mutex::new(CallState::default()),
                }),
            );
        }
        self.changed.notify_all();
        true
    }

    pub(super) fn get(&self, id: &RequestId) -> Option<Arc<ManualCall>> {
        self.calls
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .current
            .get(id)
            .cloned()
    }

    pub(super) fn dispatch(self: &Arc<Self>, id: RequestId) -> Option<DispatchOwner> {
        let call = self.get(&id)?;
        Some(DispatchOwner {
            registry: self.clone(),
            id,
            call,
        })
    }

    pub(super) fn enter(
        self: &Arc<Self>,
        id: RequestId,
        admission: InFlightGuard,
    ) -> Result<HandlerOwner, InFlightGuard> {
        let calls = self
            .calls
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(call) = calls.current.get(&id).cloned() else {
            return Err(admission);
        };
        {
            let mut state = call
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.handler_started = true;
            state.admission = Some(admission);
        }
        self.changed.notify_all();
        drop(calls);
        Ok(HandlerOwner {
            registry: self.clone(),
            id,
            call,
        })
    }

    pub(super) fn cancel(self: &Arc<Self>, id: &RequestId) -> bool {
        let (call, job) = {
            let calls = self
                .calls
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let Some(call) = calls.current.get(id).cloned() else {
                return false;
            };
            let job = call.cancellation.request();
            (call, job)
        };
        if let Some(job) = job {
            let registry = self.clone();
            let id = id.clone();
            let worker_job = job.clone();
            let worker_call = call.clone();
            let worker_id = id.clone();
            let spawned = std::thread::Builder::new()
                .name("unica-manual-control".to_owned())
                .spawn(move || {
                    let outcome =
                        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| worker_job.run()));
                    if let Ok(Err(error)) = outcome {
                        let _ = writeln!(
                            std::io::stderr().lock(),
                            "unica manual cancellation outcome is unconfirmed: {error}"
                        );
                    }
                    registry.retire(&worker_id, &worker_call);
                });
            if let Err(error) = spawned {
                job.fail_before_send(error.to_string());
                let _ = writeln!(
                    std::io::stderr().lock(),
                    "unica manual control could not start: {error}"
                );
                self.retire(&id, &call);
            }
        }
        true
    }

    pub(super) fn delivery_done(&self, id: &RequestId, call: &Arc<ManualCall>) {
        let calls = self
            .calls
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        call.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .delivery_done = true;
        self.changed.notify_all();
        drop(calls);
        self.retire(id, call);
    }

    pub(super) fn response_seen(&self, call: &Arc<ManualCall>) {
        let _calls = self
            .calls
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        call.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .response_seen = true;
        self.changed.notify_all();
    }

    /// A manual request accepted before EOF still owns its actual frontend
    /// dispatch/control. Unrelated daemon background work is not waited here.
    pub(super) fn wait_manual_settled(&self) {
        let mut calls = self
            .calls
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        loop {
            let pending = calls.entries().any(|(_, call)| {
                if !call.cancellation.requested() {
                    return false;
                }
                let state = call
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                (!state.handler_started && !state.response_seen && !state.dispatch_done)
                    || (state.handler_started && !state.handler_done)
                    || call.cancellation.pending_control()
            });
            if !pending {
                return;
            }
            calls = self
                .changed
                .wait(calls)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
    }

    pub(super) fn closed(&self) {
        let calls: Vec<_> = self
            .calls
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .entries()
            .map(|(id, call)| (id.clone(), call.clone()))
            .collect();
        for (id, call) in calls {
            self.delivery_done(&id, &call);
        }
    }

    fn retire(&self, id: &RequestId, call: &Arc<ManualCall>) {
        let mut calls = self
            .calls
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let admission = {
            let mut state = call
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if !state.delivery_done
                || (!state.handler_started && !state.response_seen && !state.dispatch_done)
                || (state.handler_started && !state.handler_done)
                || call.cancellation.pending_control()
            {
                self.changed.notify_all();
                return;
            }
            state.admission.take()
        };
        if calls
            .current
            .get(id)
            .is_some_and(|current| Arc::ptr_eq(current, call))
        {
            calls.current.remove(id);
        }
        calls
            .retiring
            .retain(|(_, retired)| !Arc::ptr_eq(retired, call));
        drop(calls);
        drop(admission);
        self.changed.notify_all();
    }
}

impl HandlerOwner {
    pub(super) fn cancellation(&self) -> CanonicalCancellation {
        self.call.cancellation.clone()
    }
}

impl Drop for HandlerOwner {
    fn drop(&mut self) {
        let calls = self
            .registry
            .calls
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.call
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .handler_done = true;
        self.registry.changed.notify_all();
        drop(calls);
        self.registry.retire(&self.id, &self.call);
    }
}

impl Drop for DispatchOwner {
    fn drop(&mut self) {
        let calls = self
            .registry
            .calls
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.call
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .dispatch_done = true;
        self.registry.changed.notify_all();
        drop(calls);
        self.registry.retire(&self.id, &self.call);
    }
}
