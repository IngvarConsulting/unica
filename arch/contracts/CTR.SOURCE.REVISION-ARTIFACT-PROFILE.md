---
id: CTR.SOURCE.REVISION-ARTIFACT-PROFILE
status: active
governs: product
decision: DEC.2026-08-27.ACTOR-REVISION-ARTIFACT-POLICY-SLICE
check:
  - crates/unica-coder/src/infrastructure/revision_artifact_policy.rs::platform_xml_revision_artifact_profile_is_closed_and_legacy_is_unchanged
  - crates/unica-coder/src/infrastructure/revision_artifact_policy.rs::actor_revision_policy_has_no_raw_issuer_or_scoped_service_bypass
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::active_platform_actor_cannot_select_the_legacy_revision_corpus
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::actor_revision_service_construction_retains_the_validated_root_across_substitution
  - crates/unica-coder/src/infrastructure/source_revision.rs::projected_revision_artifacts_equal_retained_postpublication_capture
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
scope: [platform, source]
version: 1
producer: crates/unica-coder/src/infrastructure/revision_artifact_policy.rs
consumers: [platform, review]
---

# Профиль артефактов ревизии actor-owned source set

Legacy v0.12 индексирует прежний набор расширений. Actor Platform XML 8.3.27
format 2.20 добавляет только закрытые XDTO, support, template, help и form-item
пути; остальные файлы не получают manifest entry и не расходуют byte budget.
Для configuration/extension путь ресурса начинается с корня известной
коллекции и её непосредственного owner, для external processor/report — с
единственного непосредственного owner. Произвольный префикс или смешанная
цепочка `Forms`/`Templates` не совпадает с профилем.

Контентные ресурсы входят путём, kind и digest. Прямые vendor `.cf` входят
путём, presence kind и retained identity без чтения payload.
Ambient, retained и incremental capture читают `Content` одним chunked
механизмом с одинаковыми per-file/aggregate limits и checkpoint между chunks;
`Ignored` payload не входит в aggregate byte accounting.
