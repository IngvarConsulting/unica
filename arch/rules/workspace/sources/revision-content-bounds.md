---
id: INV.SOURCE.REVISION-CONTENT-BOUNDS
check:
  - crates/unica-workspace/src/apply.rs::apply_stages_a_file_larger_than_the_former_read_bound
  - crates/unica-coder/src/infrastructure/daemon/apply_large_input_tests.rs::object_remove_scans_a_module_larger_than_the_former_bound
  - crates/unica-coder/src/infrastructure/daemon/apply_large_input_tests.rs::apply_edits_a_descriptor_larger_than_the_former_staging_bound
  - crates/unica-workspace/src/apply.rs::reference_scan_entry_budget_stops_incrementally_at_a_test_limit
  - crates/unica-workspace/src/apply.rs::reference_scan_depth_budget_stops_before_recursive_descent
  - crates/unica-workspace/src/apply.rs::reference_guard_preserves_finite_deadline_after_capture_without_writing
  - crates/unica-coder/src/infrastructure/workspace_actor_contract_tests.rs::apply_plan_fence_is_targeted_and_binds_read_only_inputs
  - crates/unica-coder/src/infrastructure/workspace_actor_contract_tests.rs::prepared_apply_rejects_stale_revision_source_change_cancellation_and_deadline_before_publication
  - crates/unica-coder/src/infrastructure/workspace_actor_contract_tests.rs::retained_apply_stop_causes_preserve_rollback_at_every_late_gate
---

# Чтение входов плана ограничено сроком операции, а не объёмом конфигурации

Подготовка `apply` читает только входы, необходимые операциям партии.
Размер файла и число элементов перечисленного поддерева задаёт
конфигурация, а не `apply`: правка крупного модуля или описания и удаление
объекта в большой конфигурации не отказывают по объёму. Планировщик может
задать предел для своего входа, и превышение такого предела даёт отказ.
Глубина вложенности перечисления ограничена: это защита от циклов.

Подготовка и публикация проверяют отмену и срок операции. Превышение
предела планировщика или глубины, отмена или истечение срока дают отказ.
Частично прочитанные входы не становятся успешно подготовленным планом;
поздний отказ откатывает уже опубликованные изменения.

Для привязки предпросмотра не вычисляется ревизия всех исходников.
Непрочитанные файлы не расходуют бюджет чтения и не меняют привязку плана.
