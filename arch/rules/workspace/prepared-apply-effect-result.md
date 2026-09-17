---
id: INV.CACHE.RETAINED-APPLY-EFFECT-RESULT
check:
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::prepared_apply_effects_are_retained_from_planner_to_result
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::prepared_apply_dry_run_returns_projected_effect_receipt_without_any_write
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::prepared_apply_success_returns_committed_effect_receipt_after_one_commit
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::retained_apply_effect_failure_matrix_rolls_back_and_returns_no_receipt
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::retained_apply_effect_races_never_publish_or_return_effects
---

# Результат применения соответствует подготовленному пакету

Подготовленный пакет правок сохраняет события изменений в исходном порядке
и рассчитанный по ним отчёт о влиянии на кеш. Внутренний результат применения
использует эти же события и отчёт.

Предпросмотр возвращает их как ожидаемые (`Projected`) без записи исходников,
кеша и сведений о ревизии. Если пакет содержит изменения, подтвердить их как
применённые (`Committed`) можно только после единственной успешной публикации.
При отказе подтверждение применения не возвращается.
