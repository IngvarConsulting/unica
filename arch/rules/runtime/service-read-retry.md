---
id: INV.RUNTIME.SERVICE-READ-RETRY
check:
  - crates/unica-coder/src/infrastructure/workspace_services.rs::manager_retries_bsl_request_once_after_transport_reset
  - crates/unica-coder/src/infrastructure/workspace_services.rs::manager_stops_after_one_transport_retry
  - crates/unica-coder/src/infrastructure/workspace_services.rs::manager_does_not_retry_unknown_bsl_tool_after_ambiguous_reset
  - crates/unica-coder/src/infrastructure/workspace_services.rs::manager_does_not_retry_typed_bsl_failure
  - crates/unica-coder/src/infrastructure/workspace_services.rs::manager_passes_remaining_budget_and_fresh_token_to_retry_spawn
  - crates/unica-coder/src/infrastructure/workspace_services.rs::transport_retry_classifier_excludes_terminal_errors
---

# Повтор чтения после разрыва связи ограничен одной попыткой

Менеджер внутреннего сервиса может один раз повторить запрос BSL `diagnostics`
или `graph` после транспортного сбоя. Перед повтором он заново находит сервис
и использует его текущий токен. Повтор расходует оставшийся срок исходного
вызова.

Неизвестный инструмент, предметная ошибка сервиса, неверный ответ протокола,
отмена или истечение срока не запускают повтор. Проверки проходят настоящий
менеджер с управляемыми соединением и запуском процесса.
