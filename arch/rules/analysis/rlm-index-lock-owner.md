---
id: INV.RLM.INDEX-LOCK-OWNER
check:
  - crates/unica-coder/src/infrastructure/workspace_index.rs::cleanup_does_not_remove_lock_replaced_by_new_owner
  - crates/unica-coder/src/infrastructure/workspace_index.rs::heartbeat_does_not_overwrite_lock_replaced_by_new_owner
gap: https://github.com/IngvarConsulting/unica/issues/988
---

# Обслуживание индекса не меняет блокировку другого владельца

Обновление и очистка относятся только к своей блокировке построителя.
Заменившая её запись другого владельца остаётся неизменной.

Проверки заменяют настоящий файл блокировки между действиями владельца.
Восстановление при живом процессе ещё требует
проверки, указанной в `gap`. Устаревший файл сам по себе не доказывает,
что процесс построителя завершился.
