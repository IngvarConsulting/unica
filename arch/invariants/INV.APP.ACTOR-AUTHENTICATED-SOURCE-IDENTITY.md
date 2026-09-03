---
id: INV.APP.ACTOR-AUTHENTICATED-SOURCE-IDENTITY
status: active
governs: product
decision: DEC.2026-08-26.ACTOR-AUTHENTICATED-SOURCE-PROFILE-SLICE
check:
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
scope: [app, cache, source]
---

# Актор сохраняет полный typed source profile

Реестр повторно использует актор только при совпадении workspace, provider
profile и всех полей каждого упорядоченного source set: имени, retained root,
kind, source format и точного platform/serialization profile. Изменение любого
поля вращает actor instance и durable workspace identity. Unsupported source
map не получает синтетическую Platform XML identity.
