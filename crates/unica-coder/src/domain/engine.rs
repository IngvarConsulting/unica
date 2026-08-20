//! Чего не хватает инструменту, чтобы запуститься.

use serde::Serialize;

/// Устойчивый код отказа: вызывающий отличает отсутствующий движок от всего
/// остального, не разбирая текст.
pub const BUNDLED_TOOL_MISSING: &str = "bundled_tool_missing";

/// Откуда движок берётся в этой установке.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum InstallMode {
    /// Исходный чекаут: инструменты собираются на месте, доставке взяться
    /// неоткуда.
    Source,
    /// Опубликованная поставка: артефакт приезжает по манифесту.
    Marketplace,
}

/// Движок, которого нет на диске.
#[derive(Debug, Clone, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MissingEngine {
    pub code: &'static str,
    /// Имя инструмента, как оно записано в поставке.
    pub tool: String,
    pub target: String,
    /// Где его ждут.
    pub expected_path: String,
    /// Версия из `third-party/tools.lock.json`, если она там названа.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pinned_version: Option<String>,
    pub install_mode: InstallMode,
    /// Что сделать, чтобы движок появился.
    pub next_step: String,
}

impl MissingEngine {
    pub fn new(
        tool: impl Into<String>,
        target: impl Into<String>,
        expected_path: impl Into<String>,
        pinned_version: Option<String>,
        install_mode: InstallMode,
    ) -> Self {
        let tool = tool.into();
        let target = target.into();
        let next_step = match install_mode {
            InstallMode::Source => format!(
                "build it from third-party/tools.lock.json: python3 scripts/ci/build-unica-tools.py --target {target}"
            ),
            InstallMode::Marketplace => format!(
                "it is delivered by the next call that needs {tool}; reinstall the Unica plugin if it never arrives"
            ),
        };
        Self {
            code: BUNDLED_TOOL_MISSING,
            tool,
            target,
            expected_path: expected_path.into(),
            pinned_version,
            install_mode,
            next_step,
        }
    }

    /// Строка для того, кто читает ошибки, а не разбирает поля.
    pub fn message(&self) -> String {
        format!(
            "{}: {} is not on this machine for {} (expected at {}); {}",
            self.code, self.tool, self.target, self.expected_path, self.next_step
        )
    }
}
