---
id: INV.RLM.INDEX-LOCK-OWNER
check:
  - crates/unica-coder/src/infrastructure/workspace_index.rs::cleanup_does_not_remove_lock_replaced_by_new_owner
  - crates/unica-coder/src/infrastructure/workspace_index.rs::heartbeat_does_not_overwrite_lock_replaced_by_new_owner
  - crates/unica-coder/src/infrastructure/workspace_index.rs::stale_recovery_does_not_overwrite_an_unexpected_held_record
  - crates/unica-coder/src/infrastructure/workspace_index.rs::stale_recovery_preserves_the_matching_record_identity_and_fields
  - crates/unica-coder/src/infrastructure/workspace_index.rs::stale_recovery_with_expected_identity_preserves_unreadable_records
gap: https://github.com/IngvarConsulting/unica/issues/988
---

# Обслуживание индекса не меняет блокировку другого владельца

Обновление и очистка относятся только к своей блокировке построителя.
Заменившая её запись другого владельца остаётся неизменной.

Проверки заменяют настоящий файл блокировки между действиями владельца.
Восстановление при живом процессе ещё требует
проверки, указанной в `gap`. Устаревший файл сам по себе не доказывает,
что процесс построителя завершился.
