---
id: INV.PERF.SERVICE-CALLER-BUDGET
check:
  - crates/unica-coder/src/infrastructure/workspace_services.rs::rlm_request_preserves_caller_budget_above_service_default
  - crates/unica-coder/src/infrastructure/workspace_services.rs::rlm_readiness_preserves_caller_budget_above_service_default
  - crates/unica-coder/src/infrastructure/workspace_services.rs::rlm_request_preserves_budget_beyond_u64_max_milliseconds
  - crates/unica-coder/src/infrastructure/workspace_services.rs::bsl_request_preserves_budget_beyond_u64_max_milliseconds
  - crates/unica-coder/src/infrastructure/workspace_services.rs::rlm_readiness_protocol_roundtrips_full_positive_i64_seconds
---

# Внутренний сервис получает оставшееся время без обрезания

Менеджер сервисов передаёт рабочим запросам RLM и анализатора BSL
оставшееся время вызывающей стороны. Внутреннее умолчание 120 секунд
не сокращает больший явно переданный срок. Это относится и к проверке
готовности RLM.

Передача длительности сохраняет секунды и наносекунды; преобразование
в миллисекунды не должно округлять или ограничивать значение. Проверки
охватывают менеджер и протокол, а не длительное выполнение поставщика.
