---
id: INV.WIRE.SDK-MODULE-EXPORTS
status: active
governs: process
decision: DEC.2026-08-18.CARRIED-RULES
check: tests/ci/test_product_contracts.py::test_rmcp_module_exports_only_run_stdio
scope: [wire]
---

# Транспортный модуль экспортирует только запуск stdio

После исключения элементов с точным `#[cfg(test)]` единственный публичный
элемент `interfaces/mcp.rs` — корневая, не обобщённая функция
`pub fn run_stdio()` без параметров и возвращаемого типа. Публичные поля,
restricted-visibility items, re-export, type alias и экспортируемые макросы
границу не пересекают.
