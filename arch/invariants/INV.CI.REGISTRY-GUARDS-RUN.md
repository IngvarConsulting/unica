---
id: INV.CI.REGISTRY-GUARDS-RUN
status: active
governs: process
decision: DEC.2026-08-19.REGISTRY-GUARDS-RUN
check: tests/ci/test_unica_workflow.py::test_registry_guards_run_in_the_source_contour
scope: [ci]
---

# Стражи реестра входят в контур источника

Задание `verify-source` прогоняет `tests/arch` наравне с `tests/ci` и
`tests/dev` и компилирует `scripts/arch`. Утверждение об этом лежит вне
`tests/arch`, иначе оно проверяло бы само себя.
