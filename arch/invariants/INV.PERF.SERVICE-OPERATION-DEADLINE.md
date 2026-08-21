---
id: INV.PERF.SERVICE-OPERATION-DEADLINE
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/infrastructure/workspace_services.rs::service_request_kind_deadline_matrix_is_closed
scope: [app, product]
---

# Повтор транспорта не сбрасывает срок операции

Менеджер сервиса передаёт поздней повторной попытке остаток исходного крайнего
срока и не начинает бюджет публичной операции заново.
