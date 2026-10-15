---
id: INV.APP.EVENT-SOURCE
check:
  - crates/unica-coder/src/infrastructure/metadata_operations.rs::meta_add_event_subscription_source_replace_needs_no_catalog_and_round_trips
  - crates/unica-coder/src/application/metadata.rs::source_parser_accepts_the_closed_nonempty_logical_event_source_algebra
  - crates/unica-coder/src/infrastructure/native_operations/meta/edit.rs::typed_event_source_semantic_noop_is_order_insensitive_and_preserves_exact_bytes
  - crates/unica-coder/src/infrastructure/native_operations/meta/edit.rs::typed_event_source_add_remove_preserve_old_order_bytes_and_repeat_as_noop
  - crates/unica-coder/src/infrastructure/native_operations/meta/edit.rs::typed_event_source_incremental_edits_keep_aliases_type_ids_and_distinct_manager_classes
  - crates/unica-coder/src/infrastructure/native_operations/meta/edit.rs::typed_event_source_incremental_add_declares_namespaces_and_preserves_bom_eol
  - crates/unica-coder/src/infrastructure/native_operations/meta/edit.rs::typed_event_source_incremental_edits_refuse_invalid_and_empty_results_without_writes
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

Точечные `relation.add` и `relation.remove` со связью `source` сохраняют
байты и относительный порядок прежних узлов, кроме явно удаляемых.
Новые источники сортируются только между собой и дополняют свою группу XML:
`Type` перед `TypeSet`/`TypeId`. Добавление уже имеющихся и удаление отсутствующих
целей не меняет файл. Полная замена сохраняет прежний детерминированный порядок
при изменении состава.

Точечная правка требует читаемого состава в логическом наборе источников.
Повреждённый или неподдержанный состав и удаление последнего логического
источника отклоняются до записи; сохранённые `TypeId` не заменяют логическую
цель своего набора исходников.
