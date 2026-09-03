---
id: INV.SURFACE.SOURCE-SKILL-ROUTING
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: tests/ci/test_unica_skills.py::test_source_access_skill_routes_reads_and_sends_writes_to_code_patch
scope: [wire]
---

# Скилл доступа к источнику разделяет чтение и запись

Скилл исследует ресурсы через `unica.source.*`, а изменение BSL отправляет в
`unica.code.patch` с предпросмотром до применения.
