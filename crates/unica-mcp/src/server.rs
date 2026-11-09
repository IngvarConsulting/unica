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
    /// Тестовый каталог вместо канонического: постраничный обход `tools/list`
    /// проверяется на каталоге больше одной страницы.
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) listed_tools_for_test: Option<Arc<Vec<Tool>>>,
}
