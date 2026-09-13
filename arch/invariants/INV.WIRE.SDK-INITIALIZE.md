---
id: INV.WIRE.SDK-INITIALIZE
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/interfaces/mcp.rs::initialize_uses_single_public_server_name_and_negotiates_version
scope: [wire]
---

# Legacy initialize публикует имя, версию сборки и согласованный протокол

Ответ `initialize` на предложение `2025-06-18` содержит имя сервера `unica`,
версию Cargo-пакета и согласованную версию протокола `2025-06-18`.
