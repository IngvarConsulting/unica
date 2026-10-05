---
id: INV.WIRE.BOUNDED-ADMISSION
check:
  - crates/unica-coder/src/interfaces/mcp.rs::admission_is_bounded_and_reusable
  - crates/unica-coder/src/interfaces/mcp.rs::overloaded_dispatcher_returns_deterministic_json_rpc_error
gap: https://github.com/IngvarConsulting/unica/issues/983
---

# MCP ограничивает число одновременных вызовов

Один MCP frontend принимает не больше 32 одновременно выполняющихся
`tools/call`. Когда все места заняты, следующий вызов получает JSON-RPC-ошибку
`-32603` с `overloaded`. Освободившееся место доступно следующему вызову.

Это предел выполняющихся запросов frontend, а не числа фоновых заданий демона.

Допуск исполнителей поставщиков описан в
[правиле конкуренции](../analysis/provider-worker-concurrency.md).
Существующие проверки MCP берут размер из реализации; независимая
проверка прежней границы frontend остаётся в `gap`.
