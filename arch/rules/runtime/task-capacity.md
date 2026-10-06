---
id: INV.APP.DAEMON-TASK-CAPACITY
check:
  - crates/unica-coder/src/infrastructure/task_store_v5.rs::production_catalog_creates_past_4096_and_reopens_exactly
  - crates/unica-coder/src/infrastructure/task_store_v5.rs::inspection_cleans_staging_past_former_directory_bound
  - crates/unica-coder/src/infrastructure/task_store_v5.rs::terminal_records_remain_until_exact_retirement_and_not_found_is_typed
  - crates/unica-coder/src/infrastructure/task_lifecycle_link_store_v5.rs::production_links_reserve_and_materialize_past_4096_and_reopen
  - crates/unica-coder/src/infrastructure/task_lifecycle_link_store_v5.rs::real_snapshot_over_8mib_can_reopen_mutate
  - crates/unica-coder/src/infrastructure/task_lifecycle_link_store_v5.rs::inspection_cleans_links_staging_past_former_directory_bound
  - crates/unica-coder/src/infrastructure/task_lifecycle_link_store_v5.rs::corrupt_snapshot_preserves_verified_orphan_before_cleanup
  - crates/unica-coder/src/infrastructure/task_lifecycle_link_store_v5.rs::catalog_schema_rejects_duplicate_unknown_missing_and_trailing_fields
  - crates/unica-coder/src/infrastructure/task_lifecycle_link_store_v5.rs::streaming_catalog_preserves_canonical_bytes_for_all_states_and_id_orders
  - crates/unica-coder/tests/daemon_receipt_ledger.rs::task_store_capacity_after_reservation_is_invariant_violation_and_fail_stops
  - crates/unica-coder/tests/daemon_receipt_ledger.rs::task_bind_direct_ack_and_receipt_terminal_expiry_release_exact_quota
  - crates/unica-coder/src/infrastructure/task_lifecycle_link_store_v5.rs::reservation_consumes_task_store_slot_before_materialization_and_reopens_exactly
  - crates/unica-coder/src/infrastructure/task_lifecycle_link_store_v5.rs::task_bound_terminal_and_retirement_transitions_are_exact_cas_and_reopen_stable
  - crates/unica-coder/src/infrastructure/task_store_v5.rs::corrupt_committed_task_preserves_orphan_before_cleanup
  - crates/unica-coder/src/infrastructure/task_store_v5.rs::second_pass_refuses_replaced_task_staging_without_deleting_either_file
  - crates/unica-coder/src/infrastructure/task_store_v5.rs::second_pass_refuses_task_staging_symlink_and_preserves_external_bytes
  - crates/unica-coder/src/infrastructure/task_lifecycle_link_store_v5.rs::second_pass_refuses_replaced_link_staging_without_deleting_either_file
  - crates/unica-coder/src/infrastructure/task_lifecycle_link_store_v5.rs::second_pass_refuses_link_staging_symlink_and_preserves_external_bytes
  - crates/unica-coder/tests/daemon_receipt_ledger.rs::task_handoff_before_begun_materializes_past_former_link_quota_without_callback
  - crates/unica-coder/tests/daemon_receipt_ledger.rs::staged_handoff_materializes_past_former_link_quota_and_reopens_exact_winner
  - crates/unica-coder/tests/daemon_receipt_ledger.rs::task_store_4097_materialization_preserves_existing_tasks_listener_and_recovery
  - crates/unica-coder/tests/daemon_receipt_ledger.rs::retained_receipt_and_task_catalogs_are_independent_after_restart
  - crates/unica-coder/tests/daemon_receipt_ledger.rs::historical_receipt_owned_capacity_state_can_complete_and_reopen_without_task_store
gap: https://github.com/IngvarConsulting/unica/issues/1119
---

# Передача в фоновое задание сохраняет точную связь и накопленные записи

Перед созданием задания сохраняется резервация его точной связи с квитанцией.
Число заданий, резерваций и связей, суммарный размер каталога и число
осиротевших staging-файлов не ограничены искусственной квотой. Данные
снимка читаются и записываются последовательно; каталоги идентичности
остаются в памяти. Это не обещание постоянного расхода памяти всего хранилища.

Хранилище не удаляет завершённое задание при чтении или ради нового задания,
даже если срок хранения прошёл. Удалением управляет общий жизненный цикл
задачи и её квитанции. После передачи остаётся одна активная связь того же
вызова; смена стадии не сохраняет вторую активную квитанцию.

Повторное открытие сохраняет записи, номера версий, индексы идентичности
и намерение отмены. Некорректная схема, противоречащая идентичность,
арифметическое переполнение и настоящая ошибка диска или доступа остаются
ошибками. Неопределённый исход публикации не разрешает потерять подготовленный
результат, освободить его владельца или повторить предметную операцию.

Прежняя ошибка `Capacity` из TaskStore после успешной резервации по-прежнему
считается нарушением внутренней гарантии: демон сохраняет намерение передачи,
закрывает приём и завершается. Проверка использует явно внесённый отказ;
продуктивное хранилище больше не выдаёт его из-за числа записей.

Предел одной связи 1 КиБ и предел сохранённого результата задачи пока
пересматриваются отдельно в #1119. Их снятие требует соответствующего пути
переноса данных; агрегатные пороги не возвращаются под другим именем.
