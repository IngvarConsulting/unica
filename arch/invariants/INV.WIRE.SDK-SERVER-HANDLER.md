---
id: INV.WIRE.SDK-SERVER-HANDLER
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/interfaces/mcp.rs::unica_server_implements_official_rmcp_server_handler
scope: [wire]
---

# Публичный сервер реализует транспортный trait официального SDK

Компилятор принимает `UnicaServer` как реализацию внешнего
`::rmcp::ServerHandler`.
