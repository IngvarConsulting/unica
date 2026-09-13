---
id: INV.REL.BLOCKING-ASSESSMENT
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: tests/ci/test_release_assessment.py::test_a_failed_blocking_scenario_fails_the_assessment_summary
scope: [ci, product]
---

# Блокирующий сценарий влияет на итог релизной оценки

Неуспешный блокирующий сценарий увеличивает счётчик блокирующих отказов и
переводит итог релизной оценки в состояние `failed`.
