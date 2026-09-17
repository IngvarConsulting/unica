---
id: INV.CI.INTERMEDIATE-RETENTION
status: active
governs: process
decision: DEC.2026-08-18.CARRIED-RULES
check: tests/ci/test_unica_workflow.py::test_intermediate_non_marketplace_artifacts_expire_after_one_day
scope: [ci]
---

# Промежуточный отчёт оценки живёт один день

Межзадачный артефакт релизной оценки имеет срок хранения в одни сутки и не
становится долговременной поставкой.
