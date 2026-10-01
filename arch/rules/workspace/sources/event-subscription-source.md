---
id: INV.APP.EVENT-SOURCE
check:
  - crates/unica-coder/src/infrastructure/metadata_operations.rs::meta_add_event_subscription_source_replace_needs_no_catalog_and_round_trips
  - crates/unica-coder/src/application/metadata.rs::source_parser_accepts_the_closed_nonempty_logical_event_source_algebra
  - crates/unica-coder/src/infrastructure/native_operations/meta/edit.rs::typed_event_source_semantic_noop_is_order_insensitive_and_preserves_exact_bytes
---

# Источник подписки задаётся логическим типом, а не произвольным XML

Типизированное описание источника различает объект, менеджер, набор записей,
определяемый тип и семейство объектов. Например, `catalogObject` означает
объекты справочников и не требует наличия отдельного справочника.

Для менеджера константы явно выбирается `constantManager` или
`constantValueManager`: один адрес даёт два разных класса с разными событиями.
Для остальных менеджеров дополнительный выбор класса запрещён.

Типизированная запись сохраняет выбор в XML, а чтение возвращает тот же
источник. Перестановка уже выбранных источников ничего не меняет и сохраняет
байты файла. Проверки относятся к внутреннему типизированному контракту;
они не возвращают прежний публичный инструмент `unica.meta.edit`.
