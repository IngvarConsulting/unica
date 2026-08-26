---
id: INV.SOURCE.READER-MIGRATION
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/application/tool_contracts.rs::subject_reader_migration_inventory_is_complete
scope: [source]
---

# Режим миграции читателя объявлен явно

Единый инвентарь объявляет тринадцать предметных читателей в режиме `bridge` и
единственный `directSwitch` для `unica.code.diagnostics`; каждый мост сохраняет
две взаимоисключающие ветви схемы, а прямой переход не публикует старые поля.
