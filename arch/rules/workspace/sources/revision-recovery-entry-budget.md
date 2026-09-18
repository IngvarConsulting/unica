---
id: INV.SOURCE.RETAINED-APPLY-TRANSIENT-ENTRY-AUTHORITY
check:
  - crates/unica-coder/src/infrastructure/source_revision.rs::actor_revision_replacement_commit_at_entry_limit_survives_owned_backup
  - crates/unica-coder/src/infrastructure/source_revision.rs::actor_revision_new_leaf_commit_at_entry_limit_survives_owned_backup
  - crates/unica-coder/src/infrastructure/source_revision.rs::actor_revision_multiple_recoveries_across_parents_preserve_exact_entry_limit
  - crates/unica-coder/src/infrastructure/source_revision.rs::actor_revision_remove_create_batch_at_entry_limit_preserves_final_tree_accounting
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::actor_revision_recovery_identity_swap_is_rejected_before_revision_install
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::actor_revision_recovery_hard_link_alias_is_never_discounted_or_restored
  - crates/unica-coder/src/infrastructure/source_revision.rs::actor_revision_exact_limit_late_failure_reaches_phase_and_rolls_back_without_receipt
  - crates/unica-coder/src/infrastructure/source_revision.rs::retained_apply_revision_transient_spoofs_still_consume_capacity
  - crates/unica-coder/src/infrastructure/source_revision.rs::retained_apply_revision_transient_create_only_and_restart_are_exact
  - crates/unica-coder/src/infrastructure/source_revision.rs::retained_apply_revision_transient_cleanup_failure_does_not_persist_authority
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::revision_transient_stop_causes_preserve_rollback
---

# Временная копия для отката не занимает место в лимите конечного дерева

При проверке записанных изменений `apply` не учитывает собственные живые
копии прежних файлов в лимите числа элементов. Принадлежность текущей
операции подтверждается точным каталогом, именем и физической идентичностью;
у файла должна быть одна жёсткая ссылка. Это позволяет записать итоговое
дерево, которое уже занимает весь допустимый лимит.

Похожее имя не даёт исключения. Посторонние файлы расходуют обычный лимит,
даже если их содержимое не входит в ревизию. Подмена копии для отката
или появление второй жёсткой ссылки запрещают её исключение и восстановление;
сохранённые исходные байты не удаляются ради завершения операции.

После завершения операции исключение перестаёт действовать. Остатки
неудачной очистки учитываются при следующем допуске и после пересоздания
актора. Отмена или истечение срока во время проверки вызывают откат
исходников, кеша и состояния ревизии.
