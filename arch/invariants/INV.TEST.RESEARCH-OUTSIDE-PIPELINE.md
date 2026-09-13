---
id: INV.TEST.RESEARCH-OUTSIDE-PIPELINE
status: active
governs: process
decision: DEC.2026-09-05.RESEARCH-IS-NOT-A-TEST
check: tests/ci/test_research_policy.py::test_research_lives_outside_the_test_plan
scope: [ci]
---

# Исследование — не тест, и в конвейере его нет

Работа, доказывающая свойство, которое не меняется от правок кода, —
`research`: проводится один раз, результат закрепляется артефактом в дереве
и записью в реестре. В дереве она живёт инструментом — `scripts/research/`
с вопросом, методом и командой, — а её код собирается только с фичей
`research`, так что ни одни ворота конвейера и ни один план прогона её не
видят. Тест под `#[ignore]` для этого не годится: он остаётся в плане и в
отчёте как отключённый.
