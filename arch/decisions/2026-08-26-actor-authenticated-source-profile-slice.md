---
id: DEC.2026-08-26.ACTOR-AUTHENTICATED-SOURCE-PROFILE-SLICE
status: active
governs: product
realized:
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::same_name_root_changed_kind_rotates_actor_and_state_scope
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::same_name_root_changed_format_or_platform_profile_rotates_actor
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::workspace_actor_registry_keys_exact_identity_and_separates_worktrees_and_source_roots
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::duplicate_physical_root_names_are_rejected_as_ambiguous
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::duplicate_source_set_names_with_distinct_roots_are_rejected
  - crates/unica-coder/src/infrastructure/daemon/server.rs::actor_read_source_capability_is_sealed_after_binding
  - crates/unica-coder/src/infrastructure/daemon/server.rs::actor_read_authority_builder_uses_only_actor_bound_semantics
  - crates/unica-coder/src/infrastructure/daemon/server.rs::provider_binding_and_actor_bound_invocation_cannot_substitute_kind_or_profile
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::capabilities_do_not_cross_distinct_actor_instances_with_equal_identity
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::workspace_actor_capabilities_enforce_identity_physical_and_bounded_publication
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::remapped_names_and_profiles_do_not_share_revision_index_or_coordination_state
  - crates/unica-coder/src/infrastructure/daemon/server.rs::subsequent_daemon_invocation_after_same_root_kind_change_gets_new_actor_identity
  - crates/unica-coder/src/infrastructure/daemon/server.rs::v13_daemon_rejects_unproved_edt_invalid_or_empty_platform_fallback
  - crates/unica-coder/src/infrastructure/daemon/server.rs::hidden_v13_logical_lease_survives_the_handoff_window_and_confirms_once
supersedes: [DEC.2026-08-23.WORKSPACE-ACTOR-SLICE]
superseded-by: null
establishes: [INV.APP.ACTOR-AUTHENTICATED-SOURCE-CAPABILITIES, INV.APP.ACTOR-AUTHENTICATED-SOURCE-IDENTITY, INV.CACHE.ACTOR-AUTHENTICATED-STATE-SCOPE]
design: docs/design/2026-08-23-v0-13-execution-surface-design.md
---

# Актор аутентифицирует полный профиль набора исходников

**Решение.** Daemon владеет реестром акторов. Его ключ состоит из
канонического корня workspace, точного provider/runtime profile и
детерминированно упорядоченных наборов `{name, canonical retained root,
SourceSetKind, SourceFormat, exact platform/serialization profile}`. Git
repository identity и source-map digest в ключ не входят. Полностью совпавший
tuple повторно использует актор; изменение любого поля получает другой актор и
другой ограниченный domain-separated state scope.

Одинаковые имена и канонические либо физические aliases корней отклоняются.
Каждый экземпляр удерживает no-follow capability корней. Выданный им provider
binding несёт весь typed tuple и непубликуемую instance identity; daemon,
logical-read lease и reader выводят kind, format и platform profile только из
этого binding. Binding и revision fence нельзя воспроизвести в другом
экземпляре даже при одинаковой структурной identity. Descriptor-relative
чтения, path-based provider/index и publication повторно проверяют actor,
физический root, revision, deadline и cancellation согласно прежней границе.

Generic actor разделяет revision, index, provider cache, coordination и
background state по полному ключу. Канонические пути кодируются стабильными
native bytes. V0.12 workspace-service adapter явно объявляет typed legacy
compatibility identity и сохраняет `LegacyPhysical` namespace. V13 daemon
принимает только обнаруженный Platform XML профиль 8.3.27 / 2.20; EDT, invalid
и пустой набор не превращаются в синтетический Platform XML root. Уже
допущенное read-only чтение может завершиться на удержанном snapshot; финальность
выбора source map для apply остаётся следующим срезом.

**Почему.** Kind, физический формат и точная версия сериализации меняют смысл
планирования не меньше имени и пути; параллельные caller-supplied поля позволяли
исполнить один binding как другой источник.

**Цена.** Семантически изменившаяся source map создаёт новый actor/state scope,
а unsupported workspace теперь получает fail-closed admission вместо
неподтверждённого synthetic fallback.
