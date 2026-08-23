---
id: INV.WIRE.COMMON-MODULE-ORDINARY-CLIENT
status: active
governs: product
decision: DEC.2026-08-22.COMMON-MODULE-ORDINARY-CLIENT
check: crates/unica-coder/src/application/metadata.rs::common_module_ordinary_client_property_is_shared_by_add_and_edit
scope: [wire]
---

# Add и edit разделяют свойство обычного клиента

Закрытые схемы `unica.meta.add` и `unica.meta.edit` публикуют и принимают
boolean-свойство `ClientOrdinaryApplication` для `CommonModule`.
