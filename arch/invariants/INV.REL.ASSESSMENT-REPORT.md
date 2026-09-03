---
id: INV.REL.ASSESSMENT-REPORT
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: tests/ci/test_release_assessment.py::test_scenario_runner_records_success_metrics_and_json_lines
scope: [ci, product]
---

# Релизная оценка выдаёт машиночитаемые результаты сценариев

Отчёт называет сценарии, их статусы и длительности и сохраняется как JSON и
JSON Lines вместе с итоговым статусом.
