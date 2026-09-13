---
id: INV.REL.ASSESSMENT-WORKFLOW-GATE
status: active
governs: product
decision: DEC.2026-08-21.AFFECTED-RELEASE-ASSESSMENT
check: tests/ci/test_unica_workflow.py::test_release_assessment_uses_affected_mechanism_contour
scope: [ci, product]
---

# Изменение релизного контура запускает оценку BSP

Workflow классифицирует затронутый механизм, передаёт оценке собранный Linux
runtime и запускает её как отдельный обязательный контур релиз-кандидата.
