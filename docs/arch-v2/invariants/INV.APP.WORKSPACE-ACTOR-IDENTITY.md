---
id: INV.APP.WORKSPACE-ACTOR-IDENTITY
status: superseded
governs: product
decision: DEC.2026-08-23.WORKSPACE-ACTOR-SLICE
check: crates/unica-coder/src/infrastructure/workspace_actor.rs::workspace_actor_registry_keys_exact_identity_and_separates_worktrees_and_source_roots
scope: [app, cache]
---

# Актор сохраняет точную логическую идентичность рабочего пространства

Реестр повторно использует актор только при совпадении канонического корня
workspace, пар имени и канонического корня каждого source set и provider
profile. Разные worktree одного Git-репозитория, переназначенные имена source
set, другой набор корней или другой profile не объединяются.
