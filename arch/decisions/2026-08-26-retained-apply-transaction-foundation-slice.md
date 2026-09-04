---
id: DEC.2026-08-26.RETAINED-APPLY-TRANSACTION-FOUNDATION-SLICE
status: active
governs: product
realized:
  - crates/unica-coder/src/infrastructure/native_operations/apply.rs::retained_transaction_roles_require_explicit_roots_and_cache_authority
  - crates/unica-coder/src/infrastructure/native_operations/apply.rs::arbitrary_second_transaction_cannot_masquerade_as_actor_cache_authority
  - crates/unica-coder/src/infrastructure/native_operations/apply.rs::closed_transaction_rejects_physical_alias_and_second_cache_participant
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::apply_admission_rejects_source_inside_cache
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::workspace_root_source_allows_exact_generated_cache_descendant
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::workspace_root_source_and_missing_cache_publish_through_disjoint_shared_anchor
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::prepared_apply_success_publishes_source_cache_record_and_state_as_one_revision
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::retained_apply_failures_restore_source_cache_and_revision_machine_exactly
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::prepared_apply_observer_sees_source_eager_revision_and_state_marker_order
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::retained_apply_observer_sees_exact_reverse_rollback_after_state_marker
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::retained_apply_final_cancellation_gate_rolls_back_all_participants
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::retained_apply_late_deadline_after_all_writes_rolls_back_all_participants
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::retained_apply_trust_epoch_race_rolls_back_without_overwriting_foreign_state
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::apply_admission_and_dry_run_revision_observation_are_cache_tree_write_free
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::actor_scoped_logical_revision_service_keeps_the_platform_fence_capability
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::prepared_apply_cleanup_race_surfaces_a_relative_actor_diagnostic
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::rollback_incomplete_failure_has_a_typed_non_success_category
supersedes: []
superseded-by: null
establishes: [INV.APP.RETAINED-APPLY-CLOSED-PARTICIPANTS, INV.CACHE.RETAINED-APPLY-REVISION-ROLLBACK, INV.CACHE.RETAINED-APPLY-DETERMINISTIC-ORDER, INV.SOURCE.RETAINED-APPLY-WRITE-FREE]
---

# Retained apply связывает source, cache и revision до публичного v0.13 cutover

**Решение.** Скрытый apply получает от одного workspace actor закрытые
`Source` и `WorkspaceCache` participants под одной writer authority. Cache
authority удерживает существующий root либо ближайшего существующего предка с
точной missing chain; свободный root или третий participant не принимается.
Exact `.build/unica` может быть потомком workspace-root source set, потому что
Source role не адресует ни один `.build` component, а revision manifest его
исключает; обратное вложение и совпадение logical roots запрещены.

Source postimages публикуются первыми, eager cache metadata следующими,
revision record затем и `state.json` последним. Revision candidate готовится
без записи, проверяется по временно опубликованному retained source и становится
видимым в памяти только после postimage и final actor/revision gates. Ошибка до
этой точки откатывает journal в обратном порядке вместе с batch-owned пустыми
каталогами; cleanup после успеха остаётся bounded diagnostic.

Apply admission, planning и dry run не создают cache tree, не пишут revision
record и не продвигают revision machine. Existing logical-read fence capability
и v0.12 routing не меняются.

**Почему.** Task 15B2a нужен самостоятельно проверяемый transaction foundation,
не активирующий широкое v0.13 wire/runtime решение до daemon integration.

**Цена.** Контур остаётся crate-private; B1b events/result projection и
публичный one-commit result принадлежат следующему slice.
