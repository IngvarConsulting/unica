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
  - crates/unica-coder/src/infrastructure/daemon/server.rs::conflicting_preview_plans_cannot_both_publish
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::saved_apply_shared_cache_preimage_refuses_second_token_without_overwriting_either_source
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::saved_apply_rebinds_nested_namespace_guards_to_the_execution_budget
---

# Применение проверяет только входы и результаты своего плана

[Сохранённый план](../apply-revision-fence.md) связывает применение
с удержанными корнями, прочитанными байтами, доказанным отсутствием файлов
и ожидаемыми результатами записи. Вход, который план только читает,
защищается так же, как изменяемый файл. Если операция перечисляет каталог
для проверки ссылок или удаления содержимого, его состав также входит
в план. Проверка отсутствующего входа сама по себе не создаёт каталогов.

Входы сверяются перед публикацией и на её границах. Изменение прочитанных
байтов даёт `stale_revision` и требует нового плана; изменившийся состав
каталога также вызывает отказ, в том числе с откатом уже сделанных записей.
Чужие изменения не перезаписываются.

Допуск, подготовка и исполнение проверяют зависимости плана без полного
обхода исходников ради общей ревизии. Изменение файла вне входов плана
не инвалидирует план. Независимые планы могут выполняться последовательно:
их входы и выходы не пересекаются и сохраняются ожидаемые результаты.
Независимость учитывает файлы исходников и кеша, прочитанные зависимости
и проверяемый состав каталогов; разных адресов правки для неё недостаточно.
При общем изменяемом кеше допустим явный отказ, как требует
[правило влияния событий на кеш](../cache-event-impact.md).
