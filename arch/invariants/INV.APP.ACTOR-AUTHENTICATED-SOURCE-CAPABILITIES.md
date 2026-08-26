---
id: INV.APP.ACTOR-AUTHENTICATED-SOURCE-CAPABILITIES
status: active
governs: product
decision: DEC.2026-08-26.ACTOR-AUTHENTICATED-SOURCE-PROFILE-SLICE
check: crates/unica-coder/src/infrastructure/workspace_actor.rs::workspace_actor_capabilities_enforce_identity_physical_and_bounded_publication
scope: [app, platform, source]
---

# Binding определяет весь исполняемый профиль источника

Actor-issued provider binding несёт source-set name, retained root, kind,
source format и exact platform/serialization profile. Daemon admission,
logical-read lease и reader не принимают параллельный kind или profile от
caller. Instance identity, physical-root validation, bounded revision fence и
publication confirmation остаются частью той же закрытой capability.
