---
id: INV.WIRE.ONE-SERVER
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: tests/ci/test_package_unica_plugin.py::test_source_mcp_declares_single_unica_orchestrator
scope: [wire]
---

# Плагин объявляет один публичный MCP-сервер

В исходном `.mcp.json` есть ровно один сервер с именем `unica`; встроенные
движки и серверы-адаптеры отдельными MCP-серверами не объявляются.
