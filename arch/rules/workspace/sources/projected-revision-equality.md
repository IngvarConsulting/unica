---
id: INV.SOURCE.REVISION-PROJECTION-CAPTURE-EQUALITY
check:
  - crates/unica-coder/src/infrastructure/native_operations/apply.rs::reference_scan_keeps_content_guard_without_retaining_every_body
  - crates/unica-coder/src/infrastructure/native_operations/apply.rs::reference_guard_rejects_same_inode_same_size_mutation_without_writing
  - crates/unica-coder/src/infrastructure/native_operations/apply.rs::reference_guard_rechecks_identity_after_stream_and_rolls_back
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::apply_plan_fence_is_targeted_and_binds_read_only_inputs
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::disjoint_apply_plans_publish_without_a_global_revision_conflict
  - crates/unica-coder/src/infrastructure/native_operations/apply.rs::read_only_absence_below_missing_parent_is_guarded_without_creating_it
  - crates/unica-coder/src/infrastructure/native_operations/apply.rs::enumerated_payload_changes_refuse_before_write_and_roll_back_late_writes
  - crates/unica-coder/src/infrastructure/native_operations/apply_families/metadata.rs::object_remove_retains_unchanged_reference_and_subsystem_inputs
---

# Предпросмотр связывает применение с прочитанными входами плана

`apply` связывает `executionToken` с конкретным планом предпросмотра: операциями и их
аргументами, прочитанными входами, доказанным отсутствием файлов и ожидаемыми
результатами записи. Вход, который план только читает, защищается так же,
как изменяемый файл. Изменение любого из этих условий требует нового
предпросмотра. Если операция перечисляет каталог для проверки ссылок или
удаления содержимого, состав этого каталога также входит в план. Проверка
отсутствующего входа сама по себе не создаёт каталогов.

Подготовка и исполнение проверяют только зависимости плана; полный обход
исходников ради общей ревизии не выполняется. Изменение непрочитанного файла
не меняет план. Две независимые партии могут примениться последовательно, если их прочитанные
входы и ожидаемые результаты сохраняются.
