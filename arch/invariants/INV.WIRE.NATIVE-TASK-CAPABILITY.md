---
id: INV.WIRE.NATIVE-TASK-CAPABILITY
status: active
governs: product
decision: DEC.2026-08-24.NATIVE-TASK-PROJECTION-SLICE
check: crates/unica-coder/src/interfaces/mcp.rs::native_task_projection_contract_is_capability_gated_durable_and_replay_free
scope: [app, wire]
---

# Native Task требует V13, новый протокол и capability одного запроса

Только hidden V13 и `2026-07-28` request/session с явно объявленным Tasks
capability получает `CreateTaskResult` и `tasks/*`. V12, старый протокол и новый
протокол без capability native Task не получают; request metadata не повышает
legacy initialized session. Projection, get, update, cancel и polling не
повторяют domain execution.
