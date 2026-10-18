---
id: INV.RUNTIME.FULL-EXPORT-PUBLICATION
check:
  - crates/unica-coder/src/infrastructure/platform/full_dump_publication.rs::concurrent_target_directory_before_first_installed_probe_survives_rollback
  - crates/unica-coder/src/infrastructure/platform/full_dump_publication.rs::concurrent_target_file_before_first_installed_probe_survives_rollback
  - crates/unica-coder/src/infrastructure/platform/full_dump_publication.rs::installed_tree_interior_mutation_is_quarantined_and_rolled_back
  - crates/unica-coder/src/infrastructure/platform/full_dump_publication.rs::installed_tree_capture_error_with_same_root_identity_is_quarantined_and_rolled_back
gap: https://github.com/IngvarConsulting/unica/issues/981
---

# Полная выгрузка заменяет исходники только после проверки результата

Применение полной выгрузки конфигурации или расширения сначала готовит
и проверяет отдельное дерево. Целевое дерево заменяется без перезаписи
чужого объекта, появившегося под тем же именем.

При ошибке после перемещения прежней цели Unica пытается восстановить её.
Если место уже занято другим объектом, этот объект сохраняется, а собственное
дерево остаётся в карантине для восстановления. Ошибка не изображается
успешной публикацией. Это одинаково относится к Windows, macOS и Linux.

Публичный `pull` доступен в ограниченном режиме с `force:true`.
Его текущие проверки не доказывают эту гарантию; подключение защиты
остаётся разрывом из `gap`.
