//! Application wire projection of delivery-owned missing-engine facts.

use unica_artifact_delivery::engine::MissingEngine;
use unica_execution_contract::invocation::DomainResult;
use unica_refusal::RefusalDetail;

pub(crate) trait MissingEngineRefusal {
    fn refusal_data(&self) -> serde_json::Value;
    fn refusal(&self, at: Option<String>) -> DomainResult;
    fn refusal_with_cause(&self, at: Option<String>, cause: &str) -> DomainResult;
    fn refusal_with_message(&self, at: Option<String>, message: String) -> DomainResult;
}

impl MissingEngineRefusal for MissingEngine {
    /// Поля отказа «нет поставляемого движка» в `data`.
    ///
    /// Кода здесь нет: он один и едет в `diagnostics`. Повтор как есть движка
    /// не вернёт, поэтому `retryable` всегда `false`.
    fn refusal_data(&self) -> serde_json::Value {
        serde_json::json!({
            "tool": self.tool,
            "path": (!self.expected_path.is_empty()).then_some(&self.expected_path),
            "target": self.target,
            "pinnedVersion": self.pinned_version,
            "stage": self.stage.as_str(),
            "retryable": false,
        })
    }

    /// Единственный отказ «нет поставляемого движка» для всех маршрутов:
    /// `provider_unavailable` с уточнением `provider_absent` и полями движка
    /// в `data`.
    fn refusal(&self, at: Option<String>) -> DomainResult {
        self.refusal_with_message(at, self.message())
    }

    /// Тот же отказ, но с исходной причиной после общего текста: запуск,
    /// который не удался, называет, что ответила ОС. Вызывающий убирает из
    /// причины путь к движку и закрывает секреты.
    fn refusal_with_cause(&self, at: Option<String>, cause: &str) -> DomainResult {
        self.refusal_with_message(at, format!("{}; cause: {cause}", self.message()))
    }

    fn refusal_with_message(&self, at: Option<String>, message: String) -> DomainResult {
        let mut result =
            DomainResult::canonical_rejection_detailed(at, RefusalDetail::ProviderAbsent, message);
        result.data = Some(self.refusal_data());
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use unica_artifact_delivery::engine::{EngineStage, InstallMode};

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

    /// #549: отказ один на все маршруты, и поля движка в нём названы, а не
    /// спрятаны в тексте.
    #[test]
    fn the_missing_engine_refusal_carries_its_fields_in_data_and_one_code() {
        for stage in EngineStage::ALL {
            for mode in [InstallMode::Source, InstallMode::Marketplace] {
                assert_eq!(engine(stage, mode).refusal_data()["stage"], stage.as_str());
            }
        }
        let refusal = engine(EngineStage::Checksum, InstallMode::Marketplace)
            .refusal(Some("main:CommonModule.Пример".to_string()));

        assert!(!refusal.ok);
        assert_eq!(refusal.diagnostics.len(), 1);
        assert_eq!(refusal.diagnostics[0]["code"], "provider_unavailable");
        assert_eq!(refusal.diagnostics[0]["detailCode"], "provider_absent");
        assert_eq!(refusal.diagnostics[0]["outcome"], "needsHuman");
        assert_eq!(
            refusal.data,
            Some(serde_json::json!({
                "tool": "bsl-analyzer",
                "path": "/plugins/unica/bin/linux-x64/bsl-analyzer",
                "target": "linux-x64",
                "pinnedVersion": "0.2.86",
                "stage": "checksum",
                "retryable": false,
            }))
        );
    }
}
