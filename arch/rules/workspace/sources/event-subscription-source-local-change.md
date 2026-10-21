---
id: INV.SOURCE.EVENT-SUBSCRIPTION-SOURCE-LOCAL-CHANGE
check:
  - crates/unica-coder/src/infrastructure/native_operations/meta/edit.rs::typed_event_source_semantic_noop_is_order_insensitive_and_preserves_exact_bytes
  - crates/unica-coder/src/infrastructure/native_operations/meta/edit.rs::typed_event_source_add_remove_preserve_old_order_bytes_and_repeat_as_noop
  - crates/unica-coder/src/infrastructure/native_operations/meta/edit.rs::typed_event_source_incremental_edits_keep_aliases_type_ids_and_distinct_manager_classes
  - crates/unica-coder/src/infrastructure/native_operations/meta/edit.rs::typed_event_source_incremental_add_declares_namespaces_and_preserves_bom_eol
  - crates/unica-coder/src/infrastructure/native_operations/meta/edit.rs::typed_event_source_incremental_edits_refuse_invalid_and_empty_results_without_writes
  - crates/unica-metadata-designer/src/xml_model.rs::event_source_removal_range_includes_whitespace_before_self_closing_last_element
---

# Правка источников подписки сохраняет нетронутый XML и логический состав

Правило относится к связи `source` подписки на событие с
[логическими типами источников](event-subscription-source.md).
Перестановка уже выбранных источников ничего не меняет и сохраняет
байты файла. Полная замена при изменении состава сохраняет прежний
детерминированный порядок.

Точечные `relation.add` и `relation.remove` сохраняют байты и относительный
порядок прежних узлов, кроме явно удаляемых. Нетронутый XML сохраняет
псевдонимы пространств имён, `TypeId`, BOM и разделители строк. Новые источники сортируются только
между собой и дополняют свою группу XML: `Type` перед `TypeSet`/`TypeId`.
Добавление уже имеющихся и удаление отсутствующих целей не меняет файл.

Точечная правка требует читаемого состава в логическом наборе источников.
Повреждённый или неподдержанный состав отклоняется до записи. После всех
операций одного вызова должен остаться хотя бы один логический источник;
промежуточное пустое состояние допустимо и наружу не публикуется. Сохранённые
`TypeId` не заменяют логическую цель своего набора исходников.
