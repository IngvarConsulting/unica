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
    pub(crate) task_hosts: Arc<std::sync::Mutex<TaskHosts>>,
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

// resource-limit: mcp.remembered-task-hosts
// class: retention
// unit: count
// scope: process
// value: 1024
// resource: Сколько заданий одного frontend помнят папку и roots вызова, начавшего задание.
// status: unproven
// origin: прототип IngvarConsulting/unica#1420
// problem: Память на каждое задание долгоживущего frontend; вопрос задаёт обычно последнее задание.
// on-reach: Забывается самое давнее задание; ответ на его вопрос исполняется в папке отвечающего запроса.
const REMEMBERED_TASK_HOSTS: usize = 1024;

/// Host context of the most recent Tasks, oldest forgotten first.
#[derive(Default)]
pub(crate) struct TaskHosts {
    hosts: std::collections::HashMap<String, TaskHost>,
    order: std::collections::VecDeque<String>,
}

impl TaskHosts {
    pub(crate) fn remember(&mut self, task_id: String, host: TaskHost) {
        if self.hosts.insert(task_id.clone(), host).is_none() {
            self.order.push_back(task_id);
        }
        while self.order.len() > REMEMBERED_TASK_HOSTS {
            if let Some(oldest) = self.order.pop_front() {
                self.hosts.remove(&oldest);
            }
        }
    }

    pub(crate) fn get(&self, task_id: &str) -> Option<&TaskHost> {
        self.hosts.get(task_id)
    }
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
