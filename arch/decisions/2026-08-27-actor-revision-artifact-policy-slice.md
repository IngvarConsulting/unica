---
id: DEC.2026-08-27.ACTOR-REVISION-ARTIFACT-POLICY-SLICE
status: active
governs: product
realized:
  - crates/unica-coder/src/infrastructure/revision_artifact_policy.rs::platform_xml_revision_artifact_profile_is_closed_and_legacy_is_unchanged
  - crates/unica-coder/src/infrastructure/revision_artifact_policy.rs::actor_revision_policy_has_no_raw_issuer_or_scoped_service_bypass
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::active_platform_actor_cannot_select_the_legacy_revision_corpus
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::actor_revision_service_construction_retains_the_validated_root_across_substitution
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
  - crates/unica-coder/src/infrastructure/source_revision.rs::actor_revision_projection_uses_capture_byte_limits_and_final_batch_accounting
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::actor_revision_lookalike_resource_is_rejected_before_publication
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::actor_revision_external_resource_drift_rotates_subsequent_admission
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::actor_revision_policy_migrates_old_scoped_record_once_then_is_restart_stable
  - crates/unica-coder/src/infrastructure/source_revision.rs::actor_revision_ignores_huge_unrelated_binary_while_bounding_targeted_resource
  - crates/unica-coder/src/infrastructure/source_revision.rs::actor_revision_targeted_resources_honor_cancellation_deadline_and_limits
  - crates/unica-coder/src/infrastructure/source_revision.rs::actor_revision_ambient_targeted_content_uses_retained_bounds_and_checkpoints
  - crates/unica-coder/src/infrastructure/source_revision.rs::actor_revision_incremental_targeted_content_uses_retained_bounds_and_checkpoints
  - crates/unica-coder/src/infrastructure/source_revision.rs::actor_revision_incremental_targeted_content_honors_mid_read_stop
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::actor_revision_platform_commit_preserves_legacy_and_surface_contracts
supersedes: []
superseded-by: null
establishes: [CTR.SOURCE.REVISION-ARTIFACT-PROFILE, INV.SOURCE.REVISION-PROJECTION-CAPTURE-EQUALITY]
design: docs/design/2026-08-27-actor-revision-artifact-policy-design.md
---

# Actor владеет единым профилем артефактов ревизии

**Решение.** Actor выдаёт одну неподделываемую authority, которая связывает
retained root, state scope и доказанные kind, format и profile выбранного source
set. Из неё одновременно строятся закрытый `RevisionArtifactPolicy` и scoped
revision service; raw production-конструкторов этих частей нет. Одна политика
определяет корпус initial ambient scan, retained scan, incremental reconciliation
и retained-apply projection. Legacy-конструктор сохраняет прежний v0.12 корпус.

Platform XML 8.3.27 format 2.20 включает содержимое известных XDTO, support,
template, help и form-item ресурсов. Прямая membership поставок `.cf` влияет
присутствием и именем без чтения payload. Неизвестный или presence-only staged
файл отклоняется до публикации.

Все три capture-фазы хешируют content общим bounded chunked механизмом с
одинаковым per-file/aggregate accounting и cancellation/deadline checkpoints.

Типизированные manifest kinds сохраняют старые значения directory/content и
добавляют presence. Алгоритм, record schema, wire shape, transaction participants,
порядок публикации и rollback не меняются.

**Почему.** Проектор и scanner не могут независимо определять ревизию: иначе
успешно спланированный postimage невоспроизводим при публикации или restart.

**Цена.** Новые Platform XML ресурсы обязаны войти в закрытый профиль отдельным
архитектурным изменением; локальная операция не может расширить корпус сама.
