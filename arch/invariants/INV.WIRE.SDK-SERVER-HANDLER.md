---
id: INV.WIRE.SDK-SERVER-HANDLER
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: tests/ci/test_product_contracts.py::test_unica_coder_production_library_satisfies_rmcp_handler_bound
scope: [wire]
---

# Публичный сервер реализует транспортный trait официального SDK

Обычная library-сборка без `cfg(test)` принимает приватный production item с
конкретной границей `UnicaServer: ::rmcp::ServerHandler`.
