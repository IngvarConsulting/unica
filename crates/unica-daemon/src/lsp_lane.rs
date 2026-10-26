//! Полоса LSP: запросы редактора исполняются своим узким пулом.
//!
//! Вызовы MCP идут через квитанции и в эту очередь не попадают, поэтому
//! поток запросов редактора не ставит их в ожидание. Полоса ограничена по
//! ширине и длине очереди; запрос, отменённый до ответа, свой результат
//! не выдаёт. Паника службы остаётся внутри полосы: демон и MCP её не видят.

use crate::protocol_v5::lsp::{
    V5LspCall, V5LspCapabilities, V5LspFailure, V5LspNotification, V5LspOutcome,
    V5LspServerNotification, V5LspSessionId, V5LspWorkspaceFolder,
};
use std::collections::{HashMap, HashSet, VecDeque};
use std::io;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Condvar, Mutex, MutexGuard};
use std::thread;
use std::time::Instant;

/// Исполнителей LSP. Узкая полоса оставляет процессор вызовам MCP.
pub const MAX_LSP_LANE_WORKERS: usize = 2;
/// Запросов, ждущих исполнителя, на весь демон.
pub const LSP_LANE_QUEUE_CAPACITY: usize = 64;
/// Уведомлений, ждущих редактора, на одну сессию.
pub const LSP_OUTBOX_CAPACITY: usize = 256;
/// Отмен, пришедших раньше своего запроса, на одну сессию.
const MAX_EARLY_CANCELLATIONS: usize = 256;

/// Служба LSP за полосой. Реализуется продуктом; демон знает только этот порт.
pub trait LspHost: Send + Sync + 'static {
    /// Открыть сессию для рабочих папок и назвать готовые возможности.
    fn open(
        &self,
        session: &V5LspSessionId,
        folders: &[V5LspWorkspaceFolder],
        client: LspClientChannel,
    ) -> Result<V5LspCapabilities, V5LspFailure>;

    /// Уведомление о документе. Исполняется на соединении сессии по порядку,
    /// поэтому должно быть быстрым.
    fn notify(
        &self,
        session: &V5LspSessionId,
        notification: V5LspNotification,
    ) -> Result<(), V5LspFailure>;

    /// Ответить на запрос. Долгая работа проверяет `context.is_cancelled()`.
    fn call(
        &self,
        session: &V5LspSessionId,
        call: V5LspCall,
        context: &LspCallContext,
    ) -> V5LspOutcome;

    fn close(&self, session: &V5LspSessionId);
}

/// Служба, у которой нет ни одной возможности: демон без продукта LSP.
pub struct NoLspHost;

impl LspHost for NoLspHost {
    fn open(
        &self,
        _: &V5LspSessionId,
        _: &[V5LspWorkspaceFolder],
        _: LspClientChannel,
    ) -> Result<V5LspCapabilities, V5LspFailure> {
        Ok(V5LspCapabilities::NONE)
    }

    fn notify(&self, _: &V5LspSessionId, _: V5LspNotification) -> Result<(), V5LspFailure> {
        Ok(())
    }

    fn call(&self, _: &V5LspSessionId, _: V5LspCall, _: &LspCallContext) -> V5LspOutcome {
        V5LspOutcome::Failed {
            failure: V5LspFailure::NotSupported,
        }
    }

    fn close(&self, _: &V5LspSessionId) {}
}

/// Состояние одного запроса для службы: отменён ли он и до какого срока
/// его ответ ещё нужен.
pub struct LspCallContext {
    cancelled: Arc<AtomicBool>,
    deadline: Instant,
}

impl LspCallContext {
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire) || Instant::now() >= self.deadline
    }

    pub fn deadline(&self) -> Instant {
        self.deadline
    }
}

/// Очередь уведомлений сессии к редактору. Закрывается вместе с сессией.
#[derive(Clone)]
pub struct LspClientChannel(Arc<Outbox>);

struct Outbox {
    state: Mutex<OutboxState>,
    arrived: Condvar,
}

struct OutboxState {
    items: VecDeque<V5LspServerNotification>,
    closed: bool,
}

impl LspClientChannel {
    fn new() -> Self {
        Self(Arc::new(Outbox {
            state: Mutex::new(OutboxState {
                items: VecDeque::new(),
                closed: false,
            }),
            arrived: Condvar::new(),
        }))
    }

    /// Поставить уведомление в очередь. `false` — сессия закрыта или
    /// редактор не забирает уведомления и очередь полна.
    pub fn push(&self, notification: V5LspServerNotification) -> bool {
        let Ok(mut state) = self.0.state.lock() else {
            return false;
        };
        if state.closed || state.items.len() >= LSP_OUTBOX_CAPACITY {
            return false;
        }
        state.items.push_back(notification);
        drop(state);
        self.0.arrived.notify_all();
        true
    }

    fn take_before(&self, deadline: Instant) -> Result<Vec<V5LspServerNotification>, V5LspFailure> {
        let mut state = self.0.state.lock().map_err(|_| V5LspFailure::Unavailable)?;
        loop {
            if state.closed {
                return Err(V5LspFailure::SessionUnknown);
            }
            if !state.items.is_empty() {
                return Ok(state.items.drain(..).collect());
            }
            let Some(wait) = deadline
                .checked_duration_since(Instant::now())
                .filter(|wait| !wait.is_zero())
            else {
                return Ok(Vec::new());
            };
            state = self
                .0
                .arrived
                .wait_timeout(state, wait)
                .map_err(|_| V5LspFailure::Unavailable)?
                .0;
        }
    }

    fn close(&self) {
        if let Ok(mut state) = self.0.state.lock() {
            state.closed = true;
            state.items.clear();
        }
        self.0.arrived.notify_all();
    }
}

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
    client: LspClientChannel,
    in_flight: HashMap<u64, Arc<AtomicBool>>,
    cancelled_early: HashSet<u64>,
}

struct Job {
    session: V5LspSessionId,
    call: V5LspCall,
    cancelled: Arc<AtomicBool>,
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
    ) -> Result<(V5LspSessionId, V5LspCapabilities), V5LspFailure> {
        let session = V5LspSessionId::new();
        let client = LspClientChannel::new();
        let capabilities = guarded(|| self.shared.host.open(&session, folders, client.clone()))
            .unwrap_or(Err(V5LspFailure::Unavailable))?;
        self.state()?.sessions.insert(
            session.clone(),
            SessionEntry {
                client,
                in_flight: HashMap::new(),
                cancelled_early: HashSet::new(),
            },
        );
        Ok((session, capabilities))
    }

    pub fn notify(
        &self,
        session: &V5LspSessionId,
        notification: V5LspNotification,
    ) -> Result<(), V5LspFailure> {
        if !self.state()?.sessions.contains_key(session) {
            return Err(V5LspFailure::SessionUnknown);
        }
        guarded(|| self.shared.host.notify(session, notification))
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
        let cancelled = Arc::new(AtomicBool::new(false));
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
            if entry.cancelled_early.remove(&request) {
                return V5LspOutcome::Cancelled {};
            }
            if entry.in_flight.contains_key(&request) {
                return failed(V5LspFailure::DuplicateRequest);
            }
            if queue_full {
                return failed(V5LspFailure::Busy);
            }
            entry.in_flight.insert(request, Arc::clone(&cancelled));
            state.jobs.push_back(Job {
                session: session.clone(),
                call,
                cancelled: Arc::clone(&cancelled),
                deadline,
                reply,
            });
        }
        self.shared.queued.notify_one();
        let outcome = match answer.recv_timeout(deadline.saturating_duration_since(Instant::now()))
        {
            Ok(outcome) => outcome,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                cancelled.store(true, Ordering::Release);
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
            _ if cancelled.load(Ordering::Acquire) => V5LspOutcome::Cancelled {},
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
            Some(cancelled) => cancelled.store(true, Ordering::Release),
            None => {
                if entry.cancelled_early.len() >= MAX_EARLY_CANCELLATIONS {
                    entry.cancelled_early.clear();
                }
                entry.cancelled_early.insert(request);
            }
        }
        Ok(())
    }

    pub fn poll(
        &self,
        session: &V5LspSessionId,
        deadline: Instant,
    ) -> Result<Vec<V5LspServerNotification>, V5LspFailure> {
        let client = self
            .state()?
            .sessions
            .get(session)
            .map(|entry| entry.client.clone())
            .ok_or(V5LspFailure::SessionUnknown)?;
        client.take_before(deadline)
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
        for cancelled in entry.in_flight.values() {
            cancelled.store(true, Ordering::Release);
        }
        entry.client.close();
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
        let context = LspCallContext {
            cancelled: job.cancelled,
            deadline: job.deadline,
        };
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
