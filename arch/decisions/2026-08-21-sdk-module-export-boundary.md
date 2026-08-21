---
id: DEC.2026-08-21.SDK-MODULE-EXPORT-BOUNDARY
status: active
governs: process
realized: tests/ci/test_product_contracts.py::test_rmcp_module_exports_only_run_stdio
supersedes: []
superseded-by: null
establishes: [INV.WIRE.SDK-MODULE-EXPORTS]
---

# Граница SDK-модуля выражается экспортами, а не локальным резолвером имён

**Решение.** Чтобы запрет выхода SDK-типов и макросов оставался
фальсифицируемым без частичного резолвера имён Rust, транспортный модуль
экспортирует ровно одну точку входа бинарного приложения — корневую
`pub fn run_stdio()`; все внутренние элементы остаются приватными, а
экспортируемые макросы запрещены.
