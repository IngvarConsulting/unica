---
id: INV.APP.SKILL-REFERENCE-REACHABILITY
status: active
governs: process
decision: DEC.2026-08-18.CARRIED-RULES
check: tests/ci/test_reference_reachability.py::test_shipped_reference_documents_are_reachable_from_a_skill
scope: [app]
---

# Новая справка достижима из скилла

Поставляемый справочный документ не увеличивает зафиксированный список
документов, недостижимых от корней скиллов.
