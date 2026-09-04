---
id: DEC.2026-08-27.RETAINED-APPLY-REVISION-TRANSIENT-AUTHORITY-SLICE
status: active
governs: product
realized:
  - crates/unica-coder/src/infrastructure/source_revision.rs::actor_revision_projection_rejects_entry_overflow_before_publication
  - crates/unica-coder/src/infrastructure/source_revision.rs::actor_revision_projection_preserves_final_entry_accounting
  - crates/unica-coder/src/infrastructure/source_revision.rs::actor_revision_projection_counts_new_parent_topology
  - crates/unica-coder/src/infrastructure/source_revision.rs::actor_revision_planning_requires_stable_ignored_entry_accounting
  - crates/unica-coder/src/infrastructure/source_revision.rs::actor_revision_projection_matches_capture_depth_boundary
  - crates/unica-coder/src/infrastructure/source_revision.rs::actor_revision_replacement_commit_at_entry_limit_survives_owned_backup
  - crates/unica-coder/src/infrastructure/source_revision.rs::actor_revision_new_leaf_commit_at_entry_limit_survives_owned_backup
  - crates/unica-coder/src/infrastructure/native_operations/compile_transaction.rs::retained_apply_revision_transient_authority_is_borrowed_sealed_and_single_issuer
  - crates/unica-coder/src/infrastructure/source_revision.rs::actor_revision_multiple_recoveries_across_parents_preserve_exact_entry_limit
  - crates/unica-coder/src/infrastructure/source_revision.rs::actor_revision_remove_create_batch_at_entry_limit_preserves_final_tree_accounting
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::actor_revision_recovery_identity_swap_is_rejected_before_revision_install
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::actor_revision_recovery_hard_link_alias_is_never_discounted_or_restored
  - crates/unica-coder/src/infrastructure/source_revision.rs::actor_revision_exact_limit_late_failure_reaches_phase_and_rolls_back_without_receipt
  - crates/unica-coder/src/infrastructure/source_revision.rs::retained_apply_revision_transient_spoofs_still_consume_capacity
  - crates/unica-coder/src/infrastructure/source_revision.rs::retained_apply_revision_transient_create_only_and_restart_are_exact
  - crates/unica-coder/src/infrastructure/source_revision.rs::retained_apply_revision_transient_cleanup_failure_does_not_persist_authority
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::revision_transient_stop_causes_preserve_rollback
supersedes: []
superseded-by: null
establishes: [INV.SOURCE.RETAINED-APPLY-TRANSIENT-ENTRY-AUTHORITY]
design: docs/design/2026-08-27-retained-apply-revision-transient-authority-design.md
---

# Journal владеет временным допуском retained revision capture

**Решение.** Planning capture сохраняет полный счётчик перечисленных entries;
retained-apply projection применяет к нему итоговую топологию batch и до
публикации проверяет entry/depth bounds живого scanner.

Во время postpublication validation journal выдаёт один sealed borrowed batch,
связывающий retained root с точными parent, recovery name и single-link regular
file capabilities вытесненных preimages. Оба enumeration pass обеих capture
могут не учитывать только доказанные этим batch физические entries, ровно по
одному разу. Произвольные ignored entries продолжают расходовать лимит.

Authority не клонируется, не сериализуется и прекращает существовать до
изменения journal, rollback или cleanup. Rollback повторно проверяет single-link
identity до восстановления. Остаток после cleanup никогда не получает authority
при следующем admission или restart.

**Почему.** Final tree на точном entry limit должен воспроизводиться, пока
journal сохраняет rollback preimage, но неаутентифицированный запас или prefix
ослабил бы retained bound для чужих файлов.

**Цена.** Scanner и transaction journal связаны внутренним borrowed proof;
каждый новый вид временного entry потребует отдельного доказанного расширения,
а не нового имени или allowance.
