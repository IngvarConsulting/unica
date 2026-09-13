---
id: INV.CACHE.RETAINED-APPLY-EFFECT-RESULT
status: active
governs: product
decision: DEC.2026-08-26.RETAINED-APPLY-EFFECT-PUBLICATION-SLICE
check:
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::prepared_apply_effects_are_retained_from_planner_to_result
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::prepared_apply_dry_run_returns_projected_effect_receipt_without_any_write
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::prepared_apply_success_returns_committed_effect_receipt_after_one_commit
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::event_implement_planner_integrates_with_actor_effect_publication_matrix
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::event_implement_op_failure_returns_no_effect_receipt_and_preserves_all_state
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::retained_apply_effect_failure_matrix_rolls_back_and_returns_no_receipt
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::retained_apply_effect_races_never_publish_or_return_effects
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::real_effect_foreign_actor_replay_preserves_both_actor_states
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::real_effect_mutation_lane_cancellation_preserves_exact_state
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::real_effect_mutation_lane_deadline_preserves_exact_state
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::real_effect_mid_scan_cancellation_preserves_exact_state
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::real_effect_mid_scan_deadline_preserves_exact_state
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::real_effect_after_all_postimages_cancellation_rolls_back_exact_state
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::real_effect_after_all_postimages_deadline_rolls_back_exact_state
scope: [app, cache, source]
---

# Retained apply result сохраняет exact planned effect subject

Один actor-owned prepared apply удерживает stable ordered events и один
выведенный из них cache report. Dry run возвращает этот subject как `Projected`
без записи, successful retained commit — как `Committed` только после
публикации, а любой отказ не возвращает receipt. Контур остаётся crate-private.
