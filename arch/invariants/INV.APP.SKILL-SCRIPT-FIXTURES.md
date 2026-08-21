---
id: INV.APP.SKILL-SCRIPT-FIXTURES
status: active
governs: process
decision: DEC.2026-08-18.CARRIED-RULES
check: tests/ci/test_unica_skills.py::test_unica_reference_models_are_test_only_fixtures
scope: [app]
---

# Эталонные скрипты остаются тестовыми моделями

Адаптированные модели операций находятся только в каталоге тестовых фикстур,
покрывают ожидаемый набор скиллов и не содержат runtime-кеша Python.
