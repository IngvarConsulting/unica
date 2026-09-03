---
id: INV.CACHE.RETAINED-APPLY-REVISION-ROLLBACK
status: active
governs: product
decision: DEC.2026-08-26.RETAINED-APPLY-TRANSACTION-FOUNDATION-SLICE
check: crates/unica-coder/src/infrastructure/workspace_actor.rs::retained_apply_failures_restore_source_cache_and_revision_machine_exactly
scope: [app, cache, platform, source]
---

# Ошибка retained apply восстанавливает source, cache и revision state

Source postimages, все eager cache postimages, revision record, `state.json` и
revision-machine candidate принадлежат одному retained journal. Ошибка на
Source, eager metadata, revision record, `state.json` либо после всех
postimages восстанавливает exact source/cache tree и прежний revision-machine
state.
