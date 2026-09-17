---
id: INV.CI.NARROW-TARGET-ARTIFACTS
status: active
governs: process
decision: DEC.2026-08-18.CARRIED-RULES
check: tests/ci/test_unica_workflow.py::test_runtime_matrix_builds_verifies_and_exports_narrow_artifacts
scope: [ci, pkg]
---

# Межзадачные артефакты разделены по цели и назначению

Матрица передаёт метаданные runtime, bootstrap и целевые архивы, не выгружает
каталог сборки Cargo и удерживает промежуточные материалы один день.
