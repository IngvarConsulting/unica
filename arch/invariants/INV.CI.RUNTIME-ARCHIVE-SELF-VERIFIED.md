---
id: INV.CI.RUNTIME-ARCHIVE-SELF-VERIFIED
status: active
governs: process
decision: DEC.2026-08-18.CARRIED-RULES
check: tests/ci/test_unica_workflow.py::test_runtime_matrix_builds_verifies_and_exports_narrow_artifacts
scope: [ci, pkg]
---

# Каждая цель проверяет свой архив runtime до передачи

Один платформенный job собирает, упаковывает и проверяет архив своей цели до
его выгрузки в межзадачное хранилище.
