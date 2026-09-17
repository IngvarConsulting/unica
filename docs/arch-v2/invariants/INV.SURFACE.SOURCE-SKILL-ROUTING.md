---
id: INV.SURFACE.SOURCE-SKILL-ROUTING
status: active
governs: product
decision: DEC.2026-09-04.SKILLS-CANONICAL-SURFACE
check: tests/ci/test_unica_skills.py::test_source_access_skill_routes_reads_and_sends_writes_to_apply
scope: [wire]
---

# Скилл доступа к источнику разделяет чтение и запись

Скилл читает узел каноническим `unica.view` и находит цель `unica.search`, а
изменение BSL отправляет в `unica.apply` с предпросмотром до применения.
Пишущего входа рядом с чтением исходников он не называет.
