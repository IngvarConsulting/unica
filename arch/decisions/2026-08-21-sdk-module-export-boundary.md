---
id: DEC.2026-08-21.SDK-MODULE-EXPORT-BOUNDARY
status: active
governs: product
realized: tests/ci/test_product_contracts.py::test_rmcp_module_preserves_legacy_public_exports_only
supersedes: []
superseded-by: null
establishes: [INV.WIRE.SDK-MODULE-EXPORTS]
---

# Граница SDK-модуля сохраняет публичную Rust-совместимость

**Решение.** Транспортный модуль сохраняет ровно четыре существовавших
до перехода точки Rust API: `MCP_MAX_TOOL_WORKERS`, `UnicaServer`,
`tool_definitions()` и корневую `run_stdio()`. Новые public item,
restricted visibility, re-export, type alias и экспортируемые макросы
границу не пересекают. Форма `run_stdio()` остаётся необобщённой,
без аргументов и возвращаемого значения.
