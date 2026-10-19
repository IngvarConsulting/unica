---
id: INV.RUNTIME.FULL-EXPORT-PUBLICATION
check:
  - crates/unica-coder/src/infrastructure/platform/full_dump_publication.rs::concurrent_target_directory_before_first_installed_probe_survives_rollback
  - crates/unica-coder/src/infrastructure/platform/full_dump_publication.rs::concurrent_target_file_before_first_installed_probe_survives_rollback
  - crates/unica-coder/src/infrastructure/platform/full_dump_publication.rs::installed_tree_interior_mutation_is_quarantined_and_rolled_back
  - crates/unica-coder/src/infrastructure/platform/full_dump_publication.rs::installed_tree_capture_error_with_same_root_identity_is_quarantined_and_rolled_back
  - crates/unica-coder/src/infrastructure/platform/full_dump_publication.rs::sealed_source_swap_after_identity_precheck_preserves_data_and_reports_unknown
  - crates/unica-coder/src/infrastructure/platform/full_dump_publication.rs::sealed_source_swap_after_identity_precheck_without_original_reports_unknown
gap: https://github.com/IngvarConsulting/unica/issues/981
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

Подключение этой защиты к публичному `pull` остаётся разрывом из `gap`.
