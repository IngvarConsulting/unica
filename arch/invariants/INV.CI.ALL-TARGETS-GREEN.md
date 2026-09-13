---
id: INV.CI.ALL-TARGETS-GREEN
status: active
governs: process
decision: DEC.2026-08-18.CARRIED-RULES
check: tests/ci/test_unica_workflow.py::test_every_supported_target_must_pass_before_publication
scope: [ci, pkg]
---

# Поставка закрывает все поддерживаемые цели

Linux, Windows и macOS входят в матрицы сборки и проверки. Упаковка и
публикация зависят от полной сборки, а потребительские пробы зависят от
упакованных или опубликованных байтов соответствующего контура.
