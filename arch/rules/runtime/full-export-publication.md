---
id: INV.RUNTIME.FULL-EXPORT-PUBLICATION
check:
  - crates/unica-coder/src/infrastructure/platform/full_dump_publication.rs::concurrent_target_directory_before_first_installed_probe_survives_rollback
  - crates/unica-coder/src/infrastructure/platform/full_dump_publication.rs::concurrent_target_file_before_first_installed_probe_survives_rollback
  - crates/unica-coder/src/infrastructure/platform/full_dump_publication.rs::installed_tree_interior_mutation_is_quarantined_and_rolled_back
  - crates/unica-coder/src/infrastructure/platform/full_dump_publication.rs::installed_tree_capture_error_with_same_root_identity_is_quarantined_and_rolled_back
  - crates/unica-coder/src/infrastructure/platform/full_dump_publication.rs::sealed_source_swap_after_identity_precheck_preserves_data_and_reports_unknown
  - crates/unica-coder/src/infrastructure/platform/full_dump_publication.rs::sealed_source_swap_after_identity_precheck_without_original_reports_unknown
  - crates/unica-coder/src/infrastructure/daemon/v13_source_export.rs::full_export_publishes_configuration_and_extension_from_a_private_stage
  - crates/unica-coder/src/infrastructure/daemon/v13_source_export.rs::full_export_rejects_invalid_results_before_replacing_sources
  - crates/unica-coder/src/infrastructure/platform/full_dump_publication.rs::canonical_shared_publisher_keeps_post_rename_content_checks_and_rolls_back_mutation
  - crates/unica-coder/src/infrastructure/platform/full_dump_publication.rs::publication_failure_after_backup_restores_original_tree
  - crates/unica-coder/src/infrastructure/daemon/v13_source_export.rs::a_verified_publication_with_no_terminal_retains_its_recovery
  - crates/unica-coder/src/infrastructure/daemon/v13_source_export.rs::published_sources_survive_missing_terminal_ack_failure_and_late_cancel
  - crates/unica-coder/src/infrastructure/platform/full_dump_publication.rs::canonical_ready_root_replacement_after_capture_remains_unowned
  - crates/unica-coder/src/infrastructure/platform/full_dump_publication.rs::canonical_ready_same_inode_mutation_after_capture_is_not_recaptured
---

# Полная выгрузка подтверждает проверенный результат и сохраняет данные при отказе

Применение полной выгрузки конфигурации или расширения сначала готовит
и проверяет отдельное дерево. Замена цели не перезаписывает объект,
появившийся под тем же именем. Подменённое или непроверенное дерево
не подтверждается как успешно опубликованный результат.

При ошибке после перемещения прежней цели Unica пытается восстановить её.
Сохранность прежних исходников и посторонних данных имеет приоритет перед
автоматическим возвратом имён. Если конкурентная подмена мешает безопасному
откату или определению результата, ответ сообщает неопределённое состояние
цели и сведения о сохранённых данных для восстановления. Неподтверждённое дерево
может остаться по целевому пути; такой отказ не означает прежнего состояния
исходников. Эта гарантия действует на всех поддерживаемых ОС.
