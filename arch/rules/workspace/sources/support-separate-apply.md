---
id: INV.SOURCE.SUPPORT-SEPARATE-APPLY
check:
  - crates/unica-coder/src/infrastructure/native_operations/apply_families/mod.rs::support_state_classification_preserves_names_across_targets_and_profiles
  - crates/unica-coder/src/infrastructure/native_operations/apply_families/mod.rs::support_and_ordinary_edits_require_separate_batches_under_every_policy
  - crates/unica-coder/src/infrastructure/native_operations/apply_families/mod.rs::support_removal_and_object_removal_require_separate_batches_under_every_policy
  - crates/unica-coder/src/infrastructure/native_operations/apply_families/mod.rs::locked_vendor_objects_refuse_every_family_before_staging
  - tests/ci/test_acceptance_scenarios.py::AcceptanceCorpusRunTests.test_every_wire_answers_its_frozen_classes
---

# Поддержка меняется отдельным применением

Операции изменения поддержки не смешиваются с обычными правками в одном
`apply`, даже если человек согласовал обе части. Если правка требует снятия
запрета, сначала отдельно применяется изменение поддержки, затем — правка
объекта. Отказ этой правки не отменяет уже применённого изменения поддержки.

Это ограничение состава запроса действует независимо от `deny`, `warn`
и `off`; сама [политика реакции на запрет](../support-policy.md) сохраняется.
