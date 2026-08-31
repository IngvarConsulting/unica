---
id: INV.CACHE.ACTOR-AUTHENTICATED-STATE-SCOPE
status: active
governs: product
decision: DEC.2026-08-26.ACTOR-AUTHENTICATED-SOURCE-PROFILE-SLICE
check: crates/unica-coder/src/infrastructure/daemon/server.rs::actor_authenticated_source_profile_contract_is_complete
scope: [app, cache, source]
---

# Generic state scope включает полный профиль источников

Domain-separated bounded state scope кодирует stable typed discriminants kind,
source format и exact platform/serialization profile вместе с canonical
workspace, упорядоченными именами/root и provider profile. Разные scope не
делят revision, index, provider cache, coordination или background state.
Только явный v0.12 compatibility adapter сохраняет `LegacyPhysical` namespace.
