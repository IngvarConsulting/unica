---
id: DEC.2026-08-26.RETAINED-APPLY-EFFECT-PUBLICATION-SLICE
status: active
governs: product
realized:
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
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::retained_apply_effect_result_contract_is_complete
supersedes: []
superseded-by: null
establishes: [INV.CACHE.RETAINED-APPLY-EFFECT-RESULT]
---

# Actor-owned apply публикует один типизированный effect receipt

**Решение.** Один actor-admitted prepared apply один раз потребляет
`PlannedApplyEffects`, сохраняет их stable first-occurrence order и выводит из
этих же событий один `CacheReport` внутри существующей закрытой retained
transaction.

После всех dry-run gates prepared subject становится `Projected` без записи.
После единственного успешного retained commit тот же subject становится
`Committed`; до успеха commit такая disposition недоступна. Любая ошибка
уничтожает prepared subject и не возвращает события или успешный cache report.

**Почему.** B1b planner и B2a transaction foundation уже владеют нужными
типизированными значениями, но без удерживаемого actor receipt terminal result
теряет причинную связь между планом и публикацией.

**Цена.** Result остаётся crate-private; daemon routing, уведомления и публичная
форма принадлежат последующим slices.
