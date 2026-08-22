---
id: INV.SURFACE.META-TOOLSET
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: tests/ci/test_meta_surface_contract.py::test_registry_is_exactly_the_four_typed_metadata_handlers
scope: [wire]
---

# Группа meta содержит четыре типизированных обработчика

Публичный реестр метаданных содержит ровно `unica.meta.info`,
`unica.meta.add`, `unica.meta.edit` и `unica.meta.remove`.
