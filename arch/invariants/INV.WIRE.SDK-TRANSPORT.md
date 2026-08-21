---
id: INV.WIRE.SDK-TRANSPORT
status: active
governs: process
decision: DEC.2026-08-18.CARRIED-RULES
check: tests/ci/test_product_contracts.py::test_rmcp_transport_is_confined_to_mcp_interface
scope: [wire]
---

# Официальный SDK изолирован в транспортном модуле

Продуктивные ссылки на `rmcp` остаются в `interfaces/mcp.rs`, и этот модуль
реализует `ServerHandler` для публичного сервера `UnicaServer`.
