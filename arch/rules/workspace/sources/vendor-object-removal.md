---
id: INV.SOURCE.VENDOR-OBJECT-REMOVAL
check:
  - crates/unica-coder/src/infrastructure/native_operations/apply_families/mod.rs::vendor_object_removal_requires_separate_off_support_apply
  - crates/unica-coder/src/infrastructure/native_operations/apply_families/mod.rs::vendor_object_removal_preserves_allowed_support_states
  - crates/unica-coder/src/infrastructure/native_operations/apply_families/mod.rs::support_removal_and_object_removal_require_separate_batches_under_every_policy
  - tests/ci/test_acceptance_scenarios.py::AcceptanceCorpusRunTests.test_every_wire_answers_its_frozen_classes
---

# Перед удалением объект поставщика снимается с поддержки

При политике `deny` разрешения редактировать объект поставщика недостаточно
для его удаления. Unica объясняет необходимость снять сам объект с поддержки
и требует явной операции. Снятие поддержки и удаление выполняются
[отдельными применениями](support-separate-apply.md).

Unica не снимает поддержку автоматически или со всей конфигурации вместо
объекта. Режимы `warn` и `off` сохраняют действующую
[политику проекта](../support-policy.md).
