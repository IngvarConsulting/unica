---
id: INV.SOURCE.REVISION-PROJECTION-CAPTURE-EQUALITY
check:
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::actor_revision_platform_resource_projection_matches_live_capture
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::actor_revision_unknown_staged_artifact_is_rejected_before_publication
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::actor_revision_lookalike_resource_is_rejected_before_publication
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::actor_revision_policy_migrates_old_scoped_record_once_then_is_restart_stable
  - crates/unica-coder/src/infrastructure/source_revision.rs::actor_revision_projection_uses_capture_byte_limits_and_final_batch_accounting
  - crates/unica-coder/src/infrastructure/source_revision.rs::actor_revision_projection_rebuilds_wire_slash_paths_in_native_manifest_encoding
  - crates/unica-coder/src/infrastructure/source_revision.rs::actor_revision_projection_rejects_entry_overflow_before_publication
  - crates/unica-coder/src/infrastructure/source_revision.rs::actor_revision_projection_preserves_final_entry_accounting
  - crates/unica-coder/src/infrastructure/source_revision.rs::actor_revision_projection_counts_new_parent_topology
  - crates/unica-coder/src/infrastructure/source_revision.rs::actor_revision_planning_requires_stable_ignored_entry_accounting
  - crates/unica-coder/src/infrastructure/source_revision.rs::actor_revision_projection_matches_capture_depth_boundary
---

# Подготовленная ревизия совпадает с ревизией записанного дерева

Подготовка `apply` и последующее чтение дерева одинаково определяют пути,
виды файлов и хеши содержимого. Ревизия успешной записи воспроизводится
следующим допуском и после пересоздания актора. Файл, содержимое которого не учитывается ревизией, в том числе только
похожий путём на ресурс, отклоняется до публикации.

Подготовленное дерево должно укладываться в те же пределы размера файлов,
общего объёма, числа элементов и глубины, что и реальный обход.
Считается итог всей партии: удаление освобождает место, замена не добавляет
элемент, новые родительские каталоги учитываются один раз. Посторонний файл
расходует лимит числа элементов, даже если его содержимое не хешируется.
Изменившееся между проверками число таких элементов даёт отказ.
