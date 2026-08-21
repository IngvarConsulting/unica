---
id: INV.WIRE.SDK-TRANSPORT
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/interfaces/mcp.rs::initialize_uses_single_public_server_name_and_negotiates_version
scope: [wire]
---

# Инициализация согласует версию протокола

Публичный stdio-транспорт отвечает именем `unica` и согласованной версией
протокола, предложенной поддерживаемым legacy-клиентом.
