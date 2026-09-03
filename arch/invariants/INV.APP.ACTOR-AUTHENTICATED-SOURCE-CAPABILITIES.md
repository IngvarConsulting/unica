---
id: INV.APP.ACTOR-AUTHENTICATED-SOURCE-CAPABILITIES
status: active
governs: product
decision: DEC.2026-08-26.ACTOR-AUTHENTICATED-SOURCE-PROFILE-SLICE
check:
  - crates/unica-coder/src/infrastructure/daemon/server.rs::actor_read_source_capability_is_sealed_after_binding
  - crates/unica-coder/src/infrastructure/daemon/server.rs::actor_read_authority_builder_uses_only_actor_bound_semantics
  - crates/unica-coder/src/infrastructure/daemon/server.rs::provider_binding_and_actor_bound_invocation_cannot_substitute_kind_or_profile
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::capabilities_do_not_cross_distinct_actor_instances_with_equal_identity
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::workspace_actor_capabilities_enforce_identity_physical_and_bounded_publication
scope: [app, platform, source]
---

# Binding определяет весь исполняемый профиль источника

Actor-issued provider binding несёт source-set name, retained root, kind,
source format и exact platform/serialization profile. Daemon admission,
logical-read lease и reader не принимают параллельный kind или profile от
caller. Instance identity, physical-root validation, bounded revision fence и
publication confirmation остаются частью той же закрытой capability.
