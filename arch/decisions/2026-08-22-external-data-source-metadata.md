---
id: DEC.2026-08-22.EXTERNAL-DATA-SOURCE-METADATA
status: active
governs: product
realized: crates/unica-coder/src/application/meta_add_surface_tests.rs::external_data_source_is_created_read_and_edited_through_typed_meta_tools
changes: [CTR.FORMAT.PLATFORM-XML-8-3-27, CTR.WIRE.TOOL-SURFACE]
establishes: [INV.APP.EXTERNAL-DATA-SOURCE-READ-MODEL, INV.SOURCE.EXTERNAL-DATA-SOURCE-ADDRESSING, INV.SOURCE.EXTERNAL-DATA-SOURCE-XML-PROFILE, INV.WIRE.EXTERNAL-DATA-SOURCE-ROLE-RIGHTS]
design: docs/design/2026-08-22-external-data-source-design.md
---

# Внешний источник данных входит в типизированную ветку метаданных

**Решение.** `ExternalDataSource` становится двадцать четвёртым видом
существующих `unica.meta.*`. Общий профиль 8.3.27 / 2.20 обслуживает создание,
чтение, изменение и проверку корневого объекта; логическая адресация продолжается
через `Table`, `Cube` и дочерний только для куба `DimensionTable`.

Писатель ролей принимает только доказанные платформой права источника, таблицы
и поля таблицы. Составные объекты, `CommonForm`, новые коллекции `meta.edit` и
цели `EventSubscription.Source` этим решением не открываются.
