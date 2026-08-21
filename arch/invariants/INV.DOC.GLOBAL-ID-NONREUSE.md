---
id: INV.DOC.GLOBAL-ID-NONREUSE
status: active
governs: process
decision: DEC.2026-08-18.REGISTRY-SHAPE
check: tests/arch/test_registry.py::test_record_ids_are_globally_unique_and_not_reused
scope: [docs]
---

# Символ записи глобально уникален и не переезжает

Действующие продуктовые и процессные записи не делят символ, а символ принятой
записи не появляется по другому пути после удаления или переноса.
