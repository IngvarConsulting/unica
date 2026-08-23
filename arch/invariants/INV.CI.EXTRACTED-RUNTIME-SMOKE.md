---
id: INV.CI.EXTRACTED-RUNTIME-SMOKE
status: active
governs: process
decision: DEC.2026-08-18.CARRIED-RULES
check: tests/ci/test_unica_workflow.py::test_mcp_smoke_runs_against_extracted_deterministic_runtime
scope: [ci, pkg]
---

# Дымовая проверка исполняет извлечённый архив

MCP smoke запускает бинарник из заново извлечённого детерминированного архива
runtime с явным общим бюджетом, а не файл из каталога сборки.
