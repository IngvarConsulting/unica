---
id: INV.WIRE.SOURCE-EXPORT-TARGET-STAYS-INSIDE-THE-WORKSPACE
check:
  - crates/unica-coder/src/infrastructure/daemon/v13_source_export.rs::arguments_are_closed_and_each_refusal_names_the_fix
  - crates/unica-coder/src/infrastructure/daemon/v13_source_export.rs::preview_of_an_extension_set_passes_the_extension_and_the_set
  - crates/unica-coder/src/infrastructure/daemon/v13_source_export.rs::preview_names_the_target_inside_the_workspace_without_writing
  - crates/unica-coder/src/infrastructure/daemon/v13_source_export.rs::preview_refuses_a_target_outside_the_workspace_an_undeclared_set_or_a_write
  - crates/unica-coder/src/infrastructure/daemon/v13_source_export.rs::apply_without_prior_preview_counts_the_exported_files_itself
  - crates/unica-coder/src/infrastructure/daemon/v13_source_export.rs::apply_refuses_another_target_and_an_empty_target
  - crates/unica-coder/src/infrastructure/daemon/v13_source_export.rs::apply_names_the_export_mode_that_happened_when_it_differs
  - crates/unica-coder/src/infrastructure/daemon/v13_source_export.rs::published_sources_survive_missing_terminal_ack_failure_and_late_cancel
  - crates/unica-coder/src/infrastructure/daemon/v13_source_export.rs::invalid_terminal_after_publication_keeps_changed_and_reports_invalid_result
  - crates/unica-coder/src/infrastructure/daemon/v13_source_export.rs::authenticated_ready_capture_precedes_concurrent_input_refusal
gap: https://github.com/IngvarConsulting/unica/issues/950
---

# Выгрузка исходников подтверждает согласованную цель

`pull` требует `force:true` и полностью заменяет один набор исходников.
`force:true` подтверждает замену всего набора, включая незафиксированные
и неотслеживаемые файлы. Аргументы — `force`, необязательные `sourceSet`
и `extension`. Без `sourceSet` выбирается набор конфигурации;
для набора расширения требуется совпадающее имя расширения.

Preview `pull` принимает только объявленный набор и цель внутри
рабочего пространства, показывает относительный путь и не допускает
ответа о выполненной записи. Используется
[режим previewApply](../mcp/run-operation-names.md). Ответ раннера о другом
наборе или другой цели не признаётся успехом.
Запрошенный режим и имя расширения в ответе должны совпасть с аргументами
вызова. Ответ передаёт фактический режим как `performedMode` с причиной расхождения
`modeReason`; preview называет режим плана раннера `plannedMode`.

Исполнение проверяет объявленный состав наборов и цель выгрузки
по текущему проекту.
Внутри вызова проектный файл, его локальное дополнение и объявленный
состав наборов сверяются до и после внутреннего preview раннера.
Их изменение даёт `concurrent_change` до исполнения.

Успешная квитанция называет относительную цель, число проверенных обычных
файлов и состоявшуюся замену. При отказе квитанция сохраняет фактический исход
публикации, в том числе если подтверждение раннера потеряно после замены.
Состояние информационной базы между preview и применением не фиксируется.

Полная выгрузка соблюдает [безопасную публикацию дерева](full-export-publication.md).
