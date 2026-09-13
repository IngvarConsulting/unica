---
id: INV.REL.ASSESSMENT-PIN
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: tests/ci/test_release_assessment.py::test_non_default_bsp_ref_is_recorded_in_actual_report
scope: [ci, product]
---

# Релизная оценка закрепляет версию настоящей конфигурации

Оценка использует явную версию BSP вместо плавающей ветки и записывает
запрошенную ссылку в отчёт.
