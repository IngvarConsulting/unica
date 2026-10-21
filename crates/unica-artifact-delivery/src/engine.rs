//! Чего не хватает инструменту, чтобы запуститься.

use serde::Serialize;
use std::path::PathBuf;

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

/// Шаг поставки, на котором движка не оказалось.
///
/// Набор закрыт: шаг называет место в цепочке «манифест — платформа — файл —
/// контрольная сумма — запуск», а не текст ошибки, который перепишут.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum EngineStage {
    /// Манифест поставки не найден, не читается или не называет движок.
    Manifest,
    /// Движок не упакован под эту платформу.
    Target,
    /// Файла движка нет: не скачан, не собран или доставка не удалась.
    Delivery,
    /// Файл есть, но его контрольная сумма не совпала с манифестом.
    Checksum,
    /// Файл прошёл проверку, но процесс движка не запустился.
    Start,
}

impl EngineStage {
    /// Все шаги — для проверок полноты.
    pub const ALL: [Self; 5] = [
        Self::Manifest,
        Self::Target,
        Self::Delivery,
        Self::Checksum,
        Self::Start,
    ];

    /// Имя шага на проводе.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Manifest => "manifest",
            Self::Target => "target",
            Self::Delivery => "delivery",
            Self::Checksum => "checksum",
            Self::Start => "start",
        }
    }

    /// Что случилось, словами для человека.
    const fn reason(self) -> &'static str {
        match self {
            Self::Manifest => "the delivery manifest does not describe it",
            Self::Target => "it is not packaged for this platform",
            Self::Delivery => "its file is not on this machine",
            Self::Checksum => "its file does not match the published checksum",
            Self::Start => "its process could not be started",
        }
    }
}

/// Что сделать, чтобы движок появился: зависит от шага и режима установки.
fn next_step(tool: &str, target: &str, stage: EngineStage, install_mode: InstallMode) -> String {
    match (stage, install_mode) {
        (EngineStage::Start, _) => format!(
            "check that security software and file permissions allow Unica's bundled {tool} to run, then repeat the call"
        ),
        (_, InstallMode::Source) => format!(
            "build it from third-party/tools.lock.json: python3 scripts/ci/build-unica-tools.py --target {target}"
        ),
        (EngineStage::Delivery, InstallMode::Marketplace) => format!(
            "it is delivered by the next call that needs {tool}; reinstall the Unica plugin if it never arrives"
        ),
        (EngineStage::Target, InstallMode::Marketplace) => {
            format!("Unica does not ship {tool} for {target}; use a supported platform")
        }
        (EngineStage::Manifest | EngineStage::Checksum, InstallMode::Marketplace) => {
            "reinstall the Unica plugin".to_string()
        }
    }
}

/// Движок, которого нет на машине или который не годится к запуску.
#[derive(Debug, Clone, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MissingEngine {
    pub code: &'static str,
    /// Имя инструмента, как оно записано в поставке.
    pub tool: String,
    pub target: String,
    /// Где движок ждут, а на шагах `checksum` и `start` — где он лежит.
    /// Пусто, когда корень плагина не найден.
    pub expected_path: String,
    /// Версия из `third-party/tools.lock.json`, если она там названа.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pinned_version: Option<String>,
    pub install_mode: InstallMode,
    pub stage: EngineStage,
    /// Что сделать, чтобы движок появился.
    pub next_step: String,
}

impl MissingEngine {
    /// Движок, которого нет на шаге `delivery`; другой шаг называет
    /// [`Self::at_stage`].
    pub fn new(
        tool: impl Into<String>,
        target: impl Into<String>,
        expected_path: impl Into<String>,
        pinned_version: Option<String>,
        install_mode: InstallMode,
    ) -> Self {
        let tool = tool.into();
        let target = target.into();
        let next_step = next_step(&tool, &target, EngineStage::Delivery, install_mode);
        Self {
            code: BUNDLED_TOOL_MISSING,
            tool,
            target,
            expected_path: expected_path.into(),
            pinned_version,
            install_mode,
            stage: EngineStage::Delivery,
            next_step,
        }
    }

    /// Тот же движок, но отсутствующий на шаге `stage`: следующий шаг
    /// выбирается заново.
    pub fn at_stage(mut self, stage: EngineStage) -> Self {
        self.stage = stage;
        self.next_step = next_step(&self.tool, &self.target, stage, self.install_mode);
        self
    }

    /// Строка для того, кто читает ошибки, а не разбирает поля.
    pub fn message(&self) -> String {
        format!("{}: {}", self.code, self.public_reason())
    }

    /// Причина без кода: движок, платформа, шаг и что сделать. Один текст
    /// на всех маршрутах — и в отказе, и в причине неполного анализа `check`.
    /// Ожидаемого пути здесь нет: он называет каталоги пользователя и едет
    /// только полем `data.path`.
    pub fn public_reason(&self) -> String {
        format!(
            "{} is not available for {}: {}; {}",
            self.tool,
            self.target,
            self.stage.reason(),
            self.next_step
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engine(stage: EngineStage, mode: InstallMode) -> MissingEngine {
        MissingEngine::new(
            "bsl-analyzer",
            "linux-x64",
            "/plugins/unica/bin/linux-x64/bsl-analyzer",
            Some("0.2.86".to_string()),
            mode,
        )
        .at_stage(stage)
    }

    #[test]
    fn every_stage_names_a_next_step_for_its_install_mode() {
        for stage in EngineStage::ALL {
            for mode in [InstallMode::Source, InstallMode::Marketplace] {
                let missing = engine(stage, mode);
                assert!(!missing.next_step.is_empty(), "{stage:?} {mode:?}");
                assert!(
                    missing.message().contains(&missing.next_step),
                    "{stage:?} {mode:?}"
                );
            }
        }
        assert!(engine(EngineStage::Delivery, InstallMode::Source)
            .next_step
            .contains("build-unica-tools.py"));
        assert!(engine(EngineStage::Delivery, InstallMode::Marketplace)
            .next_step
            .contains("delivered"));
        assert!(engine(EngineStage::Checksum, InstallMode::Marketplace)
            .next_step
            .contains("reinstall"));
        assert!(engine(EngineStage::Start, InstallMode::Marketplace)
            .next_step
            .contains("permissions"));
    }
}

#[derive(Debug, Clone)]
pub struct BundledTool {
    pub program: PathBuf,
    pub warnings: Vec<String>,
    /// Заполнено, когда предпросмотр строит план по несуществующему файлу.
    /// Вызывающий обязан это показать: путь к тому, чего нет, выглядит как
    /// готовая к запуску команда, и это худший из возможных ответов.
    pub missing: Option<MissingEngine>,
}

/// Почему поставляемый движок не разрешился.
///
/// Причина типизирована в источнике: шаг поставки, на котором движка не
/// оказалось, называет место отказа, а не фраза, которую вызывающему пришлось
/// бы разбирать (#549).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BundledToolError {
    /// Движка нет или он не годится к запуску; шаг и поля названы.
    Missing(Box<MissingEngine>),
    /// Сбой ввода-вывода, который об отсутствии движка не говорит: файл на
    /// месте, но прочитать его для контрольной суммы не удалось.
    Io(String),
}

impl BundledToolError {
    pub fn missing(engine: MissingEngine) -> Self {
        Self::Missing(Box::new(engine))
    }
}

impl std::fmt::Display for BundledToolError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Missing(engine) => formatter.write_str(&engine.message()),
            Self::Io(message) => formatter.write_str(message),
        }
    }
}

impl From<BundledToolError> for String {
    fn from(error: BundledToolError) -> Self {
        error.to_string()
    }
}
