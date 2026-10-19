---
id: INV.RUNTIME.FULL-EXPORT-RETAINED-IDENTITY
check:
  - crates/unica-coder/src/infrastructure/platform/full_dump_publication.rs::private_root_swap_cannot_escape_secret_scrubbing_or_anchor_cleanup
  - crates/unica-coder/src/infrastructure/platform/full_dump_publication.rs::private_recovery_artifacts_require_the_retained_directory_identity
  - crates/unica-coder/src/infrastructure/platform/full_dump_publication.rs::effective_config_preparation_failure_reports_retained_foreign_data
  - crates/unica-coder/src/infrastructure/platform/full_dump_publication.rs::concurrent_target_during_capture_error_quarantine_is_preserved
  - crates/unica-coder/src/infrastructure/platform/full_dump_publication.rs::concurrent_file_during_capture_error_quarantine_is_preserved
  - crates/unica-coder/src/infrastructure/platform/full_dump_publication.rs::concurrent_target_during_capture_error_quarantine_survives_recovery_failure
  - crates/unica-coder/src/infrastructure/platform/full_dump_publication.rs::reappeared_execution_stage_survives_success_and_rollback
  - crates/unica-coder/src/infrastructure/platform/full_dump_publication.rs::reappeared_sealed_stage_survives_success_and_rollback
  - crates/unica-coder/src/infrastructure/platform/full_dump_publication.rs::private_stage_swap_before_seal_precheck_preserves_data_and_scrubs_credentials
  - crates/unica-coder/src/infrastructure/platform/full_dump_publication.rs::private_stage_swap_after_seal_precheck_preserves_data_and_scrubs_credentials
  - crates/unica-coder/src/infrastructure/platform/full_dump_publication.rs::cleanup_open_replacement_preserves_owned_and_foreign_trees
  - crates/unica-coder/src/infrastructure/platform/full_dump_publication.rs::cleanup_open_preserves_unknown_descendants_of_owned_tree
  - crates/unica-coder/src/infrastructure/platform/full_dump_publication.rs::windows_private_stage_cleanup_retries_after_compartment_anchor_release
  - crates/unica-coder/src/infrastructure/platform/full_dump_publication.rs::unconfirmed_runner_retains_private_stage_for_late_writer
  - crates/unica-coder/src/infrastructure/platform/full_dump_publication.rs::runner_unwind_retains_private_stage_for_late_writer
  - crates/unica-coder/src/infrastructure/platform/full_dump_publication.rs::confirmed_spawn_failure_cleans_unused_private_stage
  - crates/unica-coder/src/infrastructure/platform/full_dump_publication.rs::windows_pending_writer_scrub_failure_preserves_data_and_retries_without_unlink
  - crates/unica-coder/src/infrastructure/platform/full_dump_publication.rs::confirmed_spawn_failure_preserves_unexpected_staged_output
  - crates/unica-coder/src/infrastructure/platform/full_dump_publication.rs::invalid_staged_xml_never_changes_visible_target
  - crates/unica-coder/src/infrastructure/platform/full_dump_publication.rs::platform_executable_or_install_swap_after_runner_prevents_publication
  - crates/unica-coder/src/infrastructure/platform/full_dump_publication.rs::staged_replacement_after_completion_capture_remains_unowned
  - crates/unica-coder/src/infrastructure/platform/full_dump_publication.rs::output_appearing_after_absent_completion_capture_remains_unowned
  - crates/unica-coder/src/infrastructure/platform/full_dump_publication.rs::canonical_preparation_failure_reports_retained_projection_and_private_data
  - crates/unica-coder/src/infrastructure/platform/full_dump_publication.rs::canonical_selected_parent_abort_preserves_a_replaced_parent_identity
  - crates/unica-coder/src/infrastructure/platform/full_dump_publication.rs::runner_project_projection_foreign_local_in_the_gap_is_preserved_with_the_null_guard
  - crates/unica-coder/src/infrastructure/platform/full_dump_publication.rs::stage_name_replacement_before_publish_never_becomes_visible
  - crates/unica-coder/src/infrastructure/platform/full_dump_publication.rs::symlink_in_private_stage_is_rejected_before_publication
  - crates/unica-coder/src/infrastructure/daemon/v13_source_export.rs::an_unconfirmed_run_retains_the_named_stage_of_a_live_writer
  - crates/unica-coder/src/infrastructure/daemon/v13_source_export.rs::an_unwind_retains_the_named_stage_of_a_live_writer
  - crates/unica-coder/src/infrastructure/platform/full_dump_publication.rs::canonical_ready_invalid_owned_xml_cleans_after_test_owned_writer_finishes
  - crates/unica-coder/src/infrastructure/platform/full_dump_publication.rs::canonical_ready_platform_recheck_refusal_cleans_after_test_owned_writer_finishes
  - crates/unica-coder/src/infrastructure/platform/full_dump_publication.rs::canonical_published_repeat_never_adopts_a_new_stage_name
  - crates/unica-coder/src/infrastructure/daemon/v13_source_export.rs::invalid_owned_xml_ready_rejects_and_retains_the_unknown_writer_stage
  - crates/unica-coder/src/infrastructure/platform/process.rs::controlled_stream_held_stderr_keeps_timeout_after_leader_exit
  - crates/unica-coder/src/infrastructure/platform/process.rs::controlled_stream_late_stderr_eof_preserves_tail_after_leader_exit
  - crates/unica-coder/src/infrastructure/platform/process.rs::controlled_stdout_read_error_stays_an_error_after_disconnect
  - crates/unica-coder/src/infrastructure/platform/process.rs::controlled_stdout_reader_panic_is_not_eof
  - crates/unica-coder/src/infrastructure/platform/process.rs::controlled_stderr_reader_faults_do_not_satisfy_eof
  - crates/unica-coder/src/infrastructure/daemon/v13_source_export.rs::a_verified_publication_with_no_terminal_retains_its_recovery
---

# Полная выгрузка сверяет идентичность объектов и сохраняет неопознанные данные

Чтение, проверка и очистка полной выгрузки сверяют удержанные идентичности
каталогов и файлов. Подменённый объект не принимается за проверенный.
Символические ссылки и reparse points не обходятся; многоссылочные файлы
и неподдерживаемые виды записей не принимаются как проверенные исходные данные.

Выгрузку выполняют конечные команды
[проверенной платформы](full-export-platform-trust.md). Публикация требует
завершения команд, записывающих подготовленное дерево. Раннер, ожидающий
подтверждения публикации, может оставаться запущенным.

Служебным каталогом владеет одна попытка до подтверждённого завершения
выполнения. После начавшегося исполнения автоматическая очистка собственных
деревьев требует завершения раннера и записывающих команд, полного конечного
результата и EOF обоих потоков вывода раннера. На macOS и Linux она выполняется
при условии, что сторонние процессы того же пользователя не изменяют
служебные каталоги во время очистки.

При обнаруженной подмене или недоказанном завершении писателя данные
сохраняются для восстановления. Ошибка сверки сообщается как нарушение
целостности. После аварии новая попытка создаёт отдельный служебный каталог.
Если конкурентное перемещение мешает безопасному возврату имени, ответ
содержит сведения о сохранённом объекте по
[правилу публикации](full-export-publication.md).

Удаление ссылки при очистке не затрагивает её цель. Delete-pending имя
в Windows не считается подтверждённым отсутствием.
