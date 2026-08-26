---
id: INV.CACHE.RETAINED-APPLY-EFFECT-RESULT
status: active
governs: product
decision: DEC.2026-08-26.RETAINED-APPLY-EFFECT-PUBLICATION-SLICE
check: crates/unica-coder/src/infrastructure/workspace_actor.rs::retained_apply_effect_result_contract_is_complete
scope: [app, cache, source]
---

# Retained apply result сохраняет exact planned effect subject

Один actor-owned prepared apply удерживает stable ordered events и один
выведенный из них cache report. Dry run возвращает этот subject как `Projected`
без записи, successful retained commit — как `Committed` только после
публикации, а любой отказ не возвращает receipt. Контур остаётся crate-private.
