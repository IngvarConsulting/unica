---
id: INV.WIRE.BOUNDED-ADMISSION
check:
  - crates/unica-coder/src/application/code_intelligence.rs::coordinator_enforces_budget_when_provider_ignores_deadline_and_cancellation
  - crates/unica-coder/src/interfaces/mcp.rs::admission_is_bounded_and_reusable
  - crates/unica-coder/src/interfaces/mcp.rs::overloaded_dispatcher_returns_deterministic_json_rpc_error
gap: https://github.com/IngvarConsulting/unica/issues/983
---

# MCP ограничивает число одновременных вызовов

Один MCP frontend принимает не больше 32 одновременно выполняющихся
`tools/call`. Когда все места заняты, следующий вызов получает JSON-RPC-ошибку
`-32603` с `overloaded`. Освободившееся место доступно следующему вызову.

Это предел выполняющихся запросов frontend, а не числа фоновых заданий демона.

Каждый поставщик анализа кода также удерживает не больше 32 исполнителей.
Истечение срока не освобождает место, пока исполнитель фактически не завершён.
Существующие проверки MCP защищают насыщение и повторное использование мест,
но берут размер из реализации. Независимая проверка границы 32 для frontend
и поставщиков остаётся в `gap`.
