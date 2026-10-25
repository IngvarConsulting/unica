---
id: INV.SOURCE.VENDOR-OBJECT-REMOVAL
check:
  - crates/unica-coder/src/infrastructure/native_operations/apply_families/mod.rs::vendor_object_removal_keeps_editable_objects_on_support_under_every_policy
  - crates/unica-coder/src/infrastructure/native_operations/apply_families/mod.rs::vendor_object_removal_of_locked_objects_follows_the_support_policy
  - crates/unica-coder/src/infrastructure/native_operations/apply_families/mod.rs::vendor_object_removal_with_locked_children_follows_the_support_policy
  - crates/unica-coder/src/infrastructure/native_operations/apply_families/mod.rs::vendor_object_removal_preserves_allowed_support_states
  - crates/unica-coder/src/infrastructure/native_operations/apply_families/mod.rs::support_removal_and_object_removal_require_separate_batches_under_every_policy
  - tests/ci/test_acceptance_scenarios.py::AcceptanceCorpusRunTests.test_every_wire_answers_its_frozen_classes
---

# Удаление объекта поставщика следует его правилу поддержки

Объект поставщика с правилом «редактируется с сохранением поддержки»
удаляется без снятия с поддержки при любой
[политике проекта](../support-policy.md), как в конфигураторе.

Если у самого объекта правило «не редактируется» или объект на поддержке
и такое правило у его дочернего объекта — реквизита, табличной части,
формы, макета, команды, — реакция определяется политикой. При `deny`
удаление отказывает до записи, называет закрытый объект и подсказывает
снять удаляемый объект с поддержки `supportRule.set` с правилом
`off-support`. Снятие выполняется
[отдельным применением](support-separate-apply.md); снятый с поддержки
объект удаляется, даже если его дочерние объекты остались закрытыми.
При `warn` объект удаляется, а результат несёт предупреждение
`support_locked`. При `off` объект удаляется без предупреждения.

Удаление не меняет маркер поддержки и не снимает поддержку
автоматически — ни с объекта, ни со всей конфигурации.
