---
id: INV.SURFACE.META-TOOLSET
status: superseded
governs: product
decision: DEC.2026-09-03.V0-13-LEGACY-BATCH-2
check: tests/ci/test_meta_surface_contract.py::test_registry_is_exactly_the_three_typed_metadata_handlers
scope: [wire]
---

# Группа meta содержит четыре типизированных обработчика

Публичный реестр метаданных содержит ровно `unica.meta.info`,
`unica.meta.add`, `unica.meta.edit` и `unica.meta.remove`.
