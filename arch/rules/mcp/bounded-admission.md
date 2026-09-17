---
id: INV.WIRE.BOUNDED-ADMISSION
check:
  - crates/unica-coder/src/interfaces/mcp.rs::admission_is_bounded_and_reusable
  - crates/unica-coder/src/interfaces/mcp.rs::overloaded_dispatcher_returns_deterministic_json_rpc_error
---

# MCP ограничивает число одновременных вызовов

Один MCP frontend принимает ограниченное число одновременно выполняющихся
`tools/call`. Когда все места заняты, следующий вызов получает JSON-RPC-ошибку
перегрузки. Освободившееся место доступно следующему вызову.

Это предел выполняющихся запросов frontend, а не числа фоновых заданий демона.
