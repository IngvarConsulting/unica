---
id: INV.SURFACE.SOURCE-SKILL-ROUTING
status: active
governs: product
decision: DEC.2026-09-04.SKILLS-CANONICAL-SURFACE
check: tests/ci/test_unica_skills.py::test_source_access_skill_routes_reads_and_sends_writes_to_code_patch
scope: [wire]
---

# Скилл доступа к источнику разделяет чтение и запись

Скилл читает узел каноническим `unica.view` и находит цель `unica.find`, а
изменение BSL отправляет в `unica.code.patch` с предпросмотром до применения.
Пишущего входа рядом с чтением исходников он не называет.
