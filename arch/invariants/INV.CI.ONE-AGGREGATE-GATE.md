---
id: INV.CI.ONE-AGGREGATE-GATE
status: active
governs: process
decision: DEC.2026-08-18.CARRIED-RULES
check: tests/ci/test_unica_workflow.py::test_every_pull_request_gets_a_stable_aggregate_gate
scope: [ci]
---

# Каждый pull request закрывает один агрегирующий шлюз

У pull request ровно один шлюз с устойчивым именем, и он сводит итоги
остальных заданий.
