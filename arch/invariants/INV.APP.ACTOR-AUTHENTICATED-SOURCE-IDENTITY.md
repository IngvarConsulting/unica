---
id: INV.APP.ACTOR-AUTHENTICATED-SOURCE-IDENTITY
status: active
governs: product
decision: DEC.2026-08-26.ACTOR-AUTHENTICATED-SOURCE-PROFILE-SLICE
check: crates/unica-coder/src/infrastructure/daemon/server.rs::actor_authenticated_source_profile_contract_is_complete
scope: [app, cache, source]
---

# Актор сохраняет полный typed source profile

Реестр повторно использует актор только при совпадении workspace, provider
profile и всех полей каждого упорядоченного source set: имени, retained root,
kind, source format и точного platform/serialization profile. Изменение любого
поля вращает actor instance и durable workspace identity. Unsupported source
map не получает синтетическую Platform XML identity.
