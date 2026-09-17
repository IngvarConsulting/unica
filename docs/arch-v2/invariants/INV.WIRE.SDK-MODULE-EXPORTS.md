---
id: INV.WIRE.SDK-MODULE-EXPORTS
status: active
governs: product
decision: DEC.2026-08-21.SDK-MODULE-EXPORT-BOUNDARY
check: tests/ci/test_product_contracts.py::test_rmcp_module_preserves_legacy_public_exports_only
scope: [wire]
---

# Транспортный модуль сохраняет точный legacy Rust API

После исключения элементов с точным `#[cfg(test)]` корневые public items
`interfaces/mcp.rs` ограничены `MCP_MAX_TOOL_WORKERS`, `UnicaServer`,
`tool_definitions()` и `run_stdio()`. Публичные поля, restricted-visibility
items, re-export, type alias, вложенные public functions и экспортируемые
макросы запрещены; `run_stdio()` не обобщена, не принимает аргументов и
возвращает unit.
