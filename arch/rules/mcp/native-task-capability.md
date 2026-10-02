---
id: INV.WIRE.NATIVE-TASK-CAPABILITY
check:
  - crates/unica-coder/src/interfaces/mcp.rs::native_task_projection_contract_is_capability_gated_durable_and_replay_free
---

# Встроенные задания MCP доступны только клиенту с Tasks capability

В канонической поверхности v0.13 ответ `CreateTaskResult` и методы `tasks/*`
доступны только при протоколе `2026-07-28` и явно объявленной клиентом
Tasks capability. Для прямого первого запроса это объявление относится
к самому запросу; после `initialize` действует согласованная версия сессии.
Метаданные отдельного запроса не повышают legacy-сессию до нового протокола.

Проекция задания, его получение, обновление, отмена и опрос не запускают
предметную операцию повторно.
