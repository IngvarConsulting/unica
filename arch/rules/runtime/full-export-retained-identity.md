---
id: INV.RUNTIME.FULL-EXPORT-RETAINED-IDENTITY
check:
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
gap: https://github.com/IngvarConsulting/unica/issues/981
---

# Полная выгрузка сверяет идентичность объектов и сохраняет неопознанные данные

Чтение, проверка и очистка полной выгрузки сверяют удержанные идентичности
каталогов и файлов. Подменённый объект не принимается за проверенный.
Символические ссылки и reparse points не обходятся; многоссылочные файлы
и неподдерживаемые виды записей не принимаются как проверенные исходные данные.

Служебным каталогом владеет одна попытка до завершения всех её процессов.
Автоматическая очистка удаляет собственные деревья после завершения писателей.
На macOS и Linux она выполняется при условии, что сторонние процессы того же
пользователя не изменяют служебные каталоги во время очистки.

При обнаруженной подмене или недоказанном завершении писателя данные
сохраняются для восстановления. Ошибка сверки сообщается как нарушение
целостности. После аварии новая попытка создаёт отдельный служебный каталог.
Если конкурентное перемещение мешает безопасному возврату имени, ответ
содержит сведения о сохранённом объекте по
[правилу публикации](full-export-publication.md).

Удаление ссылки при очистке не затрагивает её цель. Delete-pending имя
в Windows не считается подтверждённым отсутствием.

Проверка завершения писателей и подключение защиты к публичному `pull`
остаются разрывами из `gap`.
