---
id: DEC.2026-08-22.COMMON-MODULE-ORDINARY-CLIENT
status: active
governs: product
realized: crates/unica-coder/src/application/metadata.rs::common_module_ordinary_client_property_is_shared_by_add_and_edit
changes: [CTR.WIRE.TOOL-SURFACE]
establishes: [INV.WIRE.COMMON-MODULE-ORDINARY-CLIENT]
design: docs/design/2026-08-22-common-module-ordinary-client-property-design.md
---

# Обычный клиент входит в свойства общего модуля

**Решение.** Закрытые схемы `unica.meta.add` и `unica.meta.edit` принимают
необязательное boolean-свойство `ClientOrdinaryApplication` только для
`CommonModule`. Оба инструмента используют один реестр свойств для схемы,
валидации и записи Platform XML; профиль чтения `meta.info` остаётся
независимым от writer allowlist.

Неизвестное свойство отклоняется с точным полем и перечнем поддерживаемых для
вида владельца альтернатив из того же реестра.
