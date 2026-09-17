---
id: INV.SOURCE.REVISION-PROJECTION-CAPTURE-EQUALITY
status: active
governs: product
decision: DEC.2026-08-27.ACTOR-REVISION-ARTIFACT-POLICY-SLICE
check:
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::actor_revision_platform_resource_projection_matches_live_capture
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::actor_revision_unknown_staged_artifact_is_rejected_before_publication
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::actor_revision_late_failure_rolls_back_targeted_resource_without_receipt
  - crates/unica-coder/src/infrastructure/source_revision.rs::actor_revision_projection_uses_capture_byte_limits_and_final_batch_accounting
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
scope: [app, cache, platform, source]
---

# Projected revision воспроизводится retained capture

Каждый staged content postimage получает ту же классификацию, manifest kind и
digest, что две финальные retained capture. Staged presence или ignored path
отклоняется до публикации; успешный кандидат воспроизводится следующим
admission и после rebuild actor.
