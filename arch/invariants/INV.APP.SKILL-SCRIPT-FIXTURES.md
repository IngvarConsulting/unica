---
id: INV.APP.SKILL-SCRIPT-FIXTURES
status: active
governs: process
decision: DEC.2026-08-18.CARRIED-RULES
check: tests/ci/test_unica_skills.py::test_unica_reference_models_are_test_only_fixtures
scope: [app]
---

# Каталог reference models имеет закрытую тестовую форму

Внутри `unica_reference_models` Python-модели покрывают ожидаемый набор скиллов,
файлы имеют только разрешённые суффиксы, а путь не содержит `__pycache__`.
