---
id: INV.SOURCE.BORROWED-OBJECT-IDENTITY
check:
  - crates/unica-coder/src/infrastructure/native_operations/cfe_borrow_object.rs::exchange_plan_identity_is_fresh_then_stable_after_untransferred_parent_changes
  - crates/unica-coder/src/infrastructure/native_operations/cfe_borrow_object.rs::extension_identity_damage_is_refused_instead_of_repaired
  - crates/unica-coder/src/infrastructure/native_operations/cfe_borrow_object.rs::refresh_updates_transferred_properties_preserving_local_xml_and_noop_bytes
  - crates/unica-coder/src/infrastructure/native_operations/apply_families/borrow.rs::canonical_borrow_preview_refresh_and_noop_keep_local_identity_and_events_honest
---

# Повторное заимствование сохраняет идентификаторы объекта

При первом заимствовании объект в расширении получает свои идентификаторы.
Повторное заимствование того же объекта сохраняет его UUID, идентификатор
узла плана обмена (`ThisNode`) и существующие пары `TypeId`/`ValueId`
созданных типов (`GeneratedType`), в том числе после изменения родителя.
Иначе ссылки на прежний объект могут перестать работать.

Если обязательный идентификатор отсутствует из-за повреждения, применяется
[правило восстановления](damaged-object-recovery.md), а не выпуск нового
идентификатора вместо утраченного.
