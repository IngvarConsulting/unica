//! Полоса LSP: запросы редактора исполняются своим узким пулом.
//!
//! Вызовы MCP идут через квитанции и в эту очередь не попадают, поэтому
//! поток запросов редактора не ставит их в ожидание. Полоса ограничена по
//! ширине и длине очереди; запрос, отменённый до ответа, свой результат
//! не выдаёт. Паника службы остаётся внутри полосы: демон и MCP её не видят.

use std::collections::{HashMap, VecDeque};
use std::io;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{mpsc, Arc, Condvar, Mutex, MutexGuard};
use std::thread;
use std::time::Instant;
use unica_lsp_contract::host::{LspCallContext, LspClientChannel, LspClientInbox, LspHost};
use unica_lsp_contract::protocol::{
    V5LspCall, V5LspFailure, V5LspNotification, V5LspOpening, V5LspOutcome,
    V5LspServerNotification, V5LspSessionId, V5LspWorkspaceFolder,
};
use unica_work_control::cancellation::CancellationToken;

/// Исполнителей LSP. Узкая полоса оставляет процессор вызовам MCP.
// resource-limit: lsp.lane-workers
// class: concurrency
// unit: count
// scope: daemon
// value: 2
// resource: Исполнителей полосы LSP в демоне.
// status: unproven
// origin: #2173
// problem: неизвестно
// on-reach: следующие запросы ждут в очереди полосы; вызовы MCP эту очередь не проходят
// issue: #2173
pub const MAX_LSP_LANE_WORKERS: usize = 2;
/// Запросов, ждущих исполнителя, на весь демон.
// resource-limit: lsp.lane-queue
// class: concurrency
// unit: count
// scope: daemon
// value: 64
// resource: Запросов LSP, ждущих исполнителя, на весь демон.
// status: unproven
// origin: #2173
// problem: неизвестно
// on-reach: новый запрос получает отказ busy (RequestFailed unica: lsp_busy)
// issue: #2173
pub const LSP_LANE_QUEUE_CAPACITY: usize = 64;
/// Отмен, пришедших раньше своего запроса, на одну сессию.
// resource-limit: lsp.early-cancellations
// class: retention
// unit: count
// scope: session
// value: 256
// resource: Отмен, пришедших раньше своего запроса, которые помнит сессия LSP.
// status: unproven
// origin: #2173
// problem: неизвестно
// on-reach: память ранних отмен очищается; запрос, отменённый раньше, исполняется, но редактор уже получил RequestCancelled
// issue: #2173
const MAX_EARLY_CANCELLATIONS: usize = 256;

pub struct LspLane {
    shared: Arc<Shared>,
    workers: Vec<thread::JoinHandle<()>>,
}

struct Shared {
    host: Arc<dyn LspHost>,
    state: Mutex<LaneState>,
    queued: Condvar,
}

#[derive(Default)]
struct LaneState {
    jobs: VecDeque<Job>,
    sessions: HashMap<V5LspSessionId, SessionEntry>,
    stopping: bool,
}

struct SessionEntry {
    inbox: LspClientInbox,
    in_flight: HashMap<u64, CancellationToken>,
    /// Oldest first: when full, the oldest early cancellation is forgotten.
    cancelled_early: VecDeque<u64>,
}

struct Job {
    session: V5LspSessionId,
    call: V5LspCall,
    cancellation: CancellationToken,
    deadline: Instant,
    reply: mpsc::SyncSender<V5LspOutcome>,
}

impl LspLane {
    pub fn start(host: Arc<dyn LspHost>) -> io::Result<Self> {
        let shared = Arc::new(Shared {
            host,
            state: Mutex::new(LaneState::default()),
            queued: Condvar::new(),
        });
        let mut lane = Self {
            shared,
            workers: Vec::with_capacity(MAX_LSP_LANE_WORKERS),
        };
        for index in 0..MAX_LSP_LANE_WORKERS {
            let shared = Arc::clone(&lane.shared);
            lane.workers.push(
                thread::Builder::new()
                    .name(format!("unica-lsp-lane-{index}"))
                    .spawn(move || run_worker(&shared))?,
            );
        }
        Ok(lane)
    }

    pub fn open(
        &self,
        folders: &[V5LspWorkspaceFolder],
        diagnostics_refresh: bool,
        deadline: Instant,
    ) -> Result<(V5LspSessionId, V5LspOpening), V5LspFailure> {
        let session = V5LspSessionId::new();
        let (client, inbox) = LspClientChannel::pair(diagnostics_refresh);
        let opening = guarded(|| {
            self.shared
                .host
                .open(&session, folders, client.clone(), deadline)
        })
        .unwrap_or(Err(V5LspFailure::Unavailable))?;
        // One answer per folder, in the editor's order: the editor maps its
        // folders to spaces by position as well as by uri.
        let answers_every_folder = opening.workspaces.len() == folders.len()
            && opening
                .workspaces
                .iter()
                .zip(folders)
                .all(|(space, folder)| space.folder == folder.uri && space.validate().is_ok());
        if !answers_every_folder {
            let _ = guarded(|| self.shared.host.close(&session));
            return Err(V5LspFailure::Unavailable);
        }
        self.state()?.sessions.insert(
            session.clone(),
            SessionEntry {
                inbox,
                in_flight: HashMap::new(),
                cancelled_early: VecDeque::new(),
            },
        );
        Ok((session, opening))
    }

    pub fn notify(
        &self,
        session: &V5LspSessionId,
        notification: V5LspNotification,
        deadline: Instant,
    ) -> Result<(), V5LspFailure> {
        if !self.state()?.sessions.contains_key(session) {
            return Err(V5LspFailure::SessionUnknown);
        }
        guarded(|| self.shared.host.notify(session, notification, deadline))
            .unwrap_or(Err(V5LspFailure::Unavailable))
    }

    /// Исполнить запрос на полосе и дождаться ответа не дольше `deadline`.
    pub fn call(
        &self,
        session: &V5LspSessionId,
        request: u64,
        call: V5LspCall,
        deadline: Instant,
    ) -> V5LspOutcome {
        let cancellation = CancellationToken::new();
        let (reply, answer) = mpsc::sync_channel(1);
        {
            let mut state = match self.state() {
                Ok(state) => state,
                Err(failure) => return V5LspOutcome::Failed { failure },
            };
            let queue_full = state.jobs.len() >= LSP_LANE_QUEUE_CAPACITY;
            let Some(entry) = state.sessions.get_mut(session) else {
                return failed(V5LspFailure::SessionUnknown);
            };
            if let Some(index) = entry
                .cancelled_early
                .iter()
                .position(|early| *early == request)
            {
                entry.cancelled_early.remove(index);
                return V5LspOutcome::Cancelled {};
            }
            if entry.in_flight.contains_key(&request) {
                return failed(V5LspFailure::DuplicateRequest);
            }
            if queue_full {
                return failed(V5LspFailure::Busy);
            }
            entry.in_flight.insert(request, cancellation.clone());
            state.jobs.push_back(Job {
                session: session.clone(),
                call,
                cancellation: cancellation.clone(),
                deadline,
                reply,
            });
        }
        self.shared.queued.notify_one();
        let outcome = match answer.recv_timeout(deadline.saturating_duration_since(Instant::now()))
        {
            Ok(outcome) => outcome,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                cancellation.cancel();
                failed(V5LspFailure::DeadlineExceeded)
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => failed(V5LspFailure::Unavailable),
        };
        if let Ok(mut state) = self.state() {
            if let Some(entry) = state.sessions.get_mut(session) {
                entry.in_flight.remove(&request);
            }
        }
        match outcome {
            V5LspOutcome::Failed { .. } => outcome,
            // A cancellation that won the race with the answer still withholds it.
            _ if cancellation.is_cancelled() => V5LspOutcome::Cancelled {},
            // Nobody cancelled it: the host stopped because the deadline passed.
            V5LspOutcome::Cancelled {} => failed(V5LspFailure::DeadlineExceeded),
            answer => answer,
        }
    }

    /// Отменить запрос сессии. Отмена раньше самого запроса запоминается.
    pub fn cancel(&self, session: &V5LspSessionId, request: u64) -> Result<(), V5LspFailure> {
        let mut state = self.state()?;
        let entry = state
            .sessions
            .get_mut(session)
            .ok_or(V5LspFailure::SessionUnknown)?;
        match entry.in_flight.get(&request) {
            Some(cancellation) => cancellation.cancel(),
            None => {
                if entry.cancelled_early.len() >= MAX_EARLY_CANCELLATIONS {
                    entry.cancelled_early.pop_front();
                }
                entry.cancelled_early.push_back(request);
            }
        }
        Ok(())
    }

    pub fn poll(
        &self,
        session: &V5LspSessionId,
        deadline: Instant,
    ) -> Result<Vec<V5LspServerNotification>, V5LspFailure> {
        let inbox = self
            .state()?
            .sessions
            .get(session)
            .map(|entry| entry.inbox.clone())
            .ok_or(V5LspFailure::SessionUnknown)?;
        inbox.take_before(deadline)
    }

    /// Закрыть сессию: её запросы отменяются, уведомления больше не ждут.
    pub fn close(&self, session: &V5LspSessionId) {
        let entry = match self.shared.state.lock() {
            Ok(mut state) => state.sessions.remove(session),
            Err(_) => None,
        };
        let Some(entry) = entry else {
            return;
        };
        for cancellation in entry.in_flight.values() {
            cancellation.cancel();
        }
        entry.inbox.close();
        let _ = guarded(|| self.shared.host.close(session));
    }

    fn state(&self) -> Result<MutexGuard<'_, LaneState>, V5LspFailure> {
        self.shared
            .state
            .lock()
            .map_err(|_| V5LspFailure::Unavailable)
    }
}

impl Drop for LspLane {
    fn drop(&mut self) {
        let sessions = match self.shared.state.lock() {
            Ok(mut state) => {
                state.stopping = true;
                state.sessions.keys().cloned().collect::<Vec<_>>()
            }
            Err(_) => Vec::new(),
        };
        self.shared.queued.notify_all();
        for session in sessions {
            self.close(&session);
        }
        // A worker inside a call that ignores its cancellation must not hold
        // the daemon's exit: it leaves once that call returns.
        self.workers.clear();
    }
}

fn run_worker(shared: &Shared) {
    loop {
        let job = {
            let Ok(mut state) = shared.state.lock() else {
                return;
            };
            loop {
                if state.stopping {
                    return;
                }
                if let Some(job) = state.jobs.pop_front() {
                    break job;
                }
                state = match shared.queued.wait(state) {
                    Ok(state) => state,
                    Err(_) => return,
                };
            }
        };
        let context = LspCallContext::new(job.cancellation, job.deadline);
        let outcome = if context.is_cancelled() {
            V5LspOutcome::Cancelled {}
        } else {
            guarded(|| shared.host.call(&job.session, job.call, &context))
                .unwrap_or(failed(V5LspFailure::Unavailable))
        };
        let _ = job.reply.send(outcome);
    }
}

fn failed(failure: V5LspFailure) -> V5LspOutcome {
    V5LspOutcome::Failed { failure }
}

/// Паника службы обрывает только свой запрос.
fn guarded<T>(work: impl FnOnce() -> T) -> Option<T> {
    catch_unwind(AssertUnwindSafe(work)).ok()
}

#[cfg(test)]
#[path = "../tests/pure/lsp_lane.rs"]
mod tests;
