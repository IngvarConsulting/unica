//! Сервер двери MCP. Снаружи крейта его видят только тесты: производственная
//! поверхность двери — запуск по stdio.

use std::collections::HashSet;
use std::sync::Arc;

#[cfg(any(test, feature = "test-support"))]
use rmcp::model::Tool;

use crate::in_flight::InFlightRegistry;
use crate::manual_calls::ManualCalls;
use crate::surface::SurfaceToolRouter;

pub struct UnicaServer {
    pub(crate) router: SurfaceToolRouter,
    pub(crate) in_flight: Arc<InFlightRegistry>,
    pub(crate) manual_calls: Arc<ManualCalls>,
    pub(crate) structured_tools: HashSet<&'static str>,
    /// О чём рассказать вызывающему при рукопожатии. Обычная сессия платит за
    /// это ноль байтов поверхности: рассказывать нечего.
    pub(crate) startup_notice: Option<String>,
    /// What a person's answer to the question of a Task made of it, by the
    /// Task's identifier: `tasks/get` shows it instead of the question until
    /// this door restarts (INV.MCP.HUMAN-CHOICE).
    pub(crate) answered_tasks:
        Arc<std::sync::Mutex<std::collections::HashMap<String, AnsweredTask>>>,
    /// The directory selection and roots of a call that became a Task, by the
    /// Task's identifier: the answer to its question runs there too.
    pub(crate) task_hosts: Arc<std::sync::Mutex<std::collections::HashMap<String, TaskHost>>>,
    /// Тестовый каталог вместо канонического: постраничный обход `tools/list`
    /// проверяется на каталоге больше одной страницы.
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) listed_tools_for_test: Option<Arc<Vec<Tool>>>,
}

/// Host context of a call that became a Task, kept for its question.
#[derive(Clone, Default)]
pub(crate) struct TaskHost {
    pub(crate) selection: Option<String>,
    pub(crate) roots: unica_bootstrap::ClientRoots,
}

/// What a person's answer to the question of a Task made of it.
#[derive(Clone)]
pub(crate) enum AnsweredTask {
    /// The answer runs into a further question about the same call; it is
    /// the `asked`-th of the call.
    Asks {
        choice: Box<unica_execution_contract::invocation::HumanChoice>,
        refusal: Box<rmcp::model::CallToolResult>,
        asked: usize,
    },
    /// The chosen calls ended with this result.
    Completed(Box<rmcp::model::CallToolResult>),
    /// The chosen call continues as this Task.
    Follows(String),
    /// Nobody chose: the stored result stands.
    Declined,
}
