---
id: INV.PKG.PACKAGED-PUBLIC-SURFACE
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: tests/ci/test_unica_workflow.py::test_packaged_bootstrap_is_smoked_on_every_supported_host
scope: [host, pkg, product, wire]
---

# Финальный пакет проверяет полный MCP на каждой поддерживаемой цели

Релизный gate запускает bootstrap из собранного тонкого пакета на Linux,
Windows и macOS. После доставки закреплённого runtime успешная проверка
подтверждает пакет, runtime и канонический набор инструментов MCP до
потребительского продвижения.
