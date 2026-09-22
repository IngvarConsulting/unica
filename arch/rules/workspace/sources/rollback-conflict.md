---
id: INV.SOURCE.ROLLBACK-DIAGNOSTIC-CLASS
check:
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::prepared_apply_cleanup_race_surfaces_a_relative_actor_diagnostic
  - crates/unica-coder/src/infrastructure/native_operations/compile_transaction.rs::registration_rollback_preserves_same_name_recovery_decoy_after_parent_swap
  - crates/unica-coder/src/infrastructure/native_operations/compile_transaction.rs::registration_rollback_validation_reports_preserved_quarantine
  - crates/unica-coder/src/infrastructure/native_operations/compile_transaction.rs::removal_rollback_preserves_concurrent_file_and_recovery_artifact
  - crates/unica-coder/src/infrastructure/native_operations/compile_transaction.rs::removal_rollback_preserves_concurrent_empty_directory_and_recovery_tree
  - crates/unica-coder/src/infrastructure/native_operations/compile_transaction.rs::successful_registration_cleanup_warns_and_preserves_decoy_after_parent_swap
  - crates/unica-coder/src/infrastructure/native_operations/compile_transaction.rs::commit_failure_kind_does_not_depend_on_message_wording
---

# Откат сохраняет чужую замену и сообщает, что восстановить не удалось

Если опубликованный путь или путь восстановления подменён, транзакция
не перезаписывает чужой файл или каталог. Она сохраняет доступные байты
для восстановления и указывает их местонахождение в ошибке.

Неудача восстановления уже опубликованного изменения получает тип
`RollbackFailed` и диагностику `rollback encountered:`. Неудалённый временный
остаток после восстановления получает `cleanup encountered:` и не меняет
исходный тип ошибки. После успешной публикации проблема очистки сообщает
предупреждение и не откатывает опубликованные данные.

Проверка подмены при очистке выполняется на ОС, допускающих замену удерживаемого имени.
