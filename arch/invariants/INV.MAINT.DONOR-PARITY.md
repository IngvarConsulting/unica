---
id: INV.MAINT.DONOR-PARITY
status: active
governs: process
decision: DEC.2026-08-18.CARRIED-RULES
check: tests/ci/test_unica_mcp_script_parity.py::test_mcp_calls_match_unica_reference_models
scope: [ci]
---

# Донорские сценарии сравниваются с настоящими MCP-вызовами

Паритетный набор исполняет опубликованные инструменты через MCP и сопоставляет
наблюдения с принятыми отношениями к донорским моделям.
