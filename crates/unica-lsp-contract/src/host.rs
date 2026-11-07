//! Порт службы LSP: что транспорт вызывает и что отдаёт службе.

use crate::protocol::{
    V5LspCall, V5LspCapabilities, V5LspFailure, V5LspNotification, V5LspOpening, V5LspOutcome,
    V5LspServerNotification, V5LspSessionId, V5LspWorkspaceFolder, V5LspWorkspaceSpace,
};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Instant;

/// Уведомлений, ждущих редактора, на одну сессию.
// resource-limit: lsp.session-outbox
// class: concurrency
// unit: count
// scope: session
// value: 256
// resource: Уведомлений демона, ждущих редактора, на одну сессию LSP.
// status: unproven
// origin: #2173
// problem: неизвестно
// on-reach: служба получает false и уведомление не ставится в очередь
// issue: #2173
pub const LSP_OUTBOX_CAPACITY: usize = 256;

/// Служба LSP за транспортом. Реализуется продуктом; транспорт знает только этот порт.
pub trait LspHost: Send + Sync + 'static {
    /// Открыть сессию для рабочих папок: назвать готовые возможности и
    /// пространство или отказ для каждой папки, в порядке папок.
    fn open(
        &self,
        session: &V5LspSessionId,
        folders: &[V5LspWorkspaceFolder],
        client: LspClientChannel,
        deadline: Instant,
    ) -> Result<V5LspOpening, V5LspFailure>;

    /// Уведомление о документе. Исполняется на соединении сессии по порядку,
    /// и следующее ждёт его: ответ нужен до `deadline`.
    fn notify(
        &self,
        session: &V5LspSessionId,
        notification: V5LspNotification,
        deadline: Instant,
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
        folders: &[V5LspWorkspaceFolder],
        _: LspClientChannel,
        _: Instant,
    ) -> Result<V5LspOpening, V5LspFailure> {
        Ok(V5LspOpening {
            capabilities: V5LspCapabilities::NONE,
            workspaces: folders
                .iter()
                .map(|folder| V5LspWorkspaceSpace::refused(&folder.uri, V5LspFailure::NotSupported))
                .collect(),
        })
    }

    fn notify(
        &self,
        _: &V5LspSessionId,
        _: V5LspNotification,
        _: Instant,
    ) -> Result<(), V5LspFailure> {
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
    /// Контекст запроса, который транспорт отменяет через `cancelled`.
    pub fn new(cancelled: Arc<AtomicBool>, deadline: Instant) -> Self {
        Self {
            cancelled,
            deadline,
        }
    }

    /// A context no one cancels, for driving a service without the lane.
    #[cfg(any(test, feature = "test-support"))]
    pub fn for_test(deadline: Instant) -> Self {
        Self::new(Arc::new(AtomicBool::new(false)), deadline)
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire) || Instant::now() >= self.deadline
    }

    pub fn deadline(&self) -> Instant {
        self.deadline
    }

    /// The flag the transport sets when it cancels the request, for work
    /// that watches it through its own cancellation token.
    pub fn cancellation_signal(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.cancelled)
    }
}

/// Очередь уведомлений сессии к редактору, сторона службы: только ставит.
/// Закрывается вместе с сессией.
#[derive(Clone)]
pub struct LspClientChannel(Arc<Outbox>);

/// Та же очередь, сторона транспорта: забирает уведомления и закрывает сессию.
#[derive(Clone)]
pub struct LspClientInbox(Arc<Outbox>);

struct Outbox {
    state: Mutex<OutboxState>,
    arrived: Condvar,
}

struct OutboxState {
    items: VecDeque<V5LspServerNotification>,
    closed: bool,
}

impl LspClientChannel {
    /// Новая очередь сессии: канал отдаётся службе, приёмник остаётся у транспорта.
    pub fn pair() -> (Self, LspClientInbox) {
        let outbox = Arc::new(Outbox {
            state: Mutex::new(OutboxState {
                items: VecDeque::new(),
                closed: false,
            }),
            arrived: Condvar::new(),
        });
        (Self(Arc::clone(&outbox)), LspClientInbox(outbox))
    }

    /// A channel no editor reads, for driving a service without the lane.
    #[cfg(any(test, feature = "test-support"))]
    pub fn detached_for_test() -> Self {
        Self::pair().0
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
}

impl LspClientInbox {
    /// Забрать всё накопленное или ждать первого уведомления до `deadline`.
    pub fn take_before(
        &self,
        deadline: Instant,
    ) -> Result<Vec<V5LspServerNotification>, V5LspFailure> {
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

    /// Закрыть очередь: служба больше не ставит, ожидающий забор кончается.
    pub fn close(&self) {
        if let Ok(mut state) = self.0.state.lock() {
            state.closed = true;
            state.items.clear();
        }
        self.0.arrived.notify_all();
    }
}
