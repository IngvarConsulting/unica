---
id: INV.SOURCE.RETAINED-APPLY-WRITE-FREE
status: active
governs: product
decision: DEC.2026-08-26.RETAINED-APPLY-TRANSACTION-FOUNDATION-SLICE
check: crates/unica-coder/src/infrastructure/workspace_actor.rs::apply_admission_and_dry_run_revision_observation_are_cache_tree_write_free
scope: [app, cache, source]
---

# Apply observation и dry run не публикуют cache или revision state

Apply admission, planning и dry run сохраняют полную topology и bytes cache
tree, включая изначально отсутствующие cache root и `source-revisions`, не
меняют source bytes и возвращают admitted revision без commit.
