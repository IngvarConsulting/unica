---
id: INV.APP.ACTOR-AUTHENTICATED-SOURCE-IDENTITY
check:
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::same_name_root_changed_kind_rotates_actor_and_state_scope
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::same_name_root_changed_format_or_platform_profile_rotates_actor
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::workspace_actor_registry_keys_exact_identity_and_separates_worktrees_and_source_roots
  - crates/unica-coder/src/infrastructure/daemon/server.rs::subsequent_daemon_invocation_after_same_root_kind_change_gets_new_actor_identity
  - crates/unica-coder/src/infrastructure/daemon/server.rs::v13_daemon_rejects_unproved_edt_invalid_or_empty_platform_fallback
---

# Актор переиспользуется только для того же профиля исходников

Реестр возвращает прежний `WorkspaceActor` только при полном совпадении
[профиля исходников](source-profile-state-isolation.md). Перестановка
одинаковых наборов в объявлении не создаёт новый актор. Другой рабочий
каталог, переназначенное имя набора, изменённые корень, вид, формат или профиль
платформы требуют другого экземпляра.

Демон не подставляет профиль Platform XML для неподтверждённой карты
исходников. Например, пустой проект, EDT и противоречивые признаки формата
не принимаются как конфигурация Platform XML по умолчанию.
