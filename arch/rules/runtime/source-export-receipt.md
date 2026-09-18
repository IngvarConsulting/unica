---
id: INV.WIRE.SOURCE-EXPORT-TARGET-STAYS-INSIDE-THE-WORKSPACE
check:
  - crates/unica-coder/src/infrastructure/daemon/server.rs::v5_source_export_prepares_before_source_admission_and_keeps_the_revision_gate
  - crates/unica-coder/src/infrastructure/daemon/v13_source_export.rs::preview_names_the_target_inside_the_workspace_without_writing
  - crates/unica-coder/src/infrastructure/daemon/v13_source_export.rs::preview_refuses_a_target_outside_the_workspace_an_undeclared_set_or_a_write
  - crates/unica-coder/src/infrastructure/daemon/v13_source_export.rs::apply_repeats_the_preview_and_counts_the_exported_files_itself
  - crates/unica-coder/src/infrastructure/daemon/v13_source_export.rs::apply_refuses_a_stale_revision_another_target_and_an_empty_target
---

# Выгрузка исходников подтверждает согласованную цель

Preview `source.export` принимает только объявленный набор и цель внутри
рабочего пространства, показывает относительный путь и не допускает
ответа о выполненной записи. Применение требует `ifRev` preview. Ответ
раннера о другом наборе или другой цели не признаётся успехом.

После выгрузки Unica сама считает файлы в целевом каталоге. Отсутствующая
или пустая цель вызывает отказ. Это квитанция о наличии файлов, а не
проверка полноты или содержимого выгрузки. Состояние информационной базы
между preview и применением не фиксируется.

Проверки используют управляемый раннер и реальные каталоги. Они не
запускают выгрузку платформой 1С и не обещают отмену внешних записей раннера
при отказе.
