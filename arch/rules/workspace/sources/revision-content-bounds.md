---
id: INV.SOURCE.REVISION-CONTENT-BOUNDS
check:
  - crates/unica-coder/src/infrastructure/native_operations/apply.rs::apply_staged_read_has_one_closed_bound_that_cannot_be_caller_bypassed
  - crates/unica-coder/src/infrastructure/native_operations/apply.rs::reference_scan_refuses_a_file_larger_than_the_per_file_budget
  - crates/unica-coder/src/infrastructure/native_operations/apply.rs::reference_scan_entry_budget_stops_incrementally_at_a_test_limit
  - crates/unica-coder/src/infrastructure/native_operations/apply.rs::reference_scan_depth_budget_stops_before_recursive_descent
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::apply_plan_fence_is_targeted_and_binds_read_only_inputs
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::prepared_apply_rejects_stale_revision_source_change_cancellation_and_deadline_before_publication
  - crates/unica-coder/src/infrastructure/workspace_actor.rs::retained_apply_stop_causes_preserve_rollback_at_every_late_gate
---

# Чтение входов плана ограничено размером и сроком операции

Подготовка `apply` читает только входы, необходимые операциям партии.
Для файла действует предел размера; планировщик может выбрать более
строгий предел, но не увеличить общий предел чтения. Перечисление нужного
поддерева ограничено количеством элементов и глубиной вложенности.

Подготовка и публикация проверяют отмену и срок операции. Превышение
лимита, отмена или истечение срока дают отказ. Частично прочитанные
входы не становятся успешно подготовленным планом; поздний отказ
откатывает уже опубликованные изменения.

Для привязки предпросмотра не вычисляется ревизия всех исходников.
Непрочитанные файлы не расходуют бюджет чтения и не меняют привязку плана.
