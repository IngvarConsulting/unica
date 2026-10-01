---
id: INV.WIRE.SOURCE-EXPORT-TARGET-STAYS-INSIDE-THE-WORKSPACE
check:
  - crates/unica-coder/src/infrastructure/daemon/v13_source_export.rs::arguments_are_closed_and_each_refusal_names_the_fix
  - crates/unica-coder/src/infrastructure/daemon/v13_source_export.rs::preview_of_an_extension_set_passes_the_extension_and_the_set
  - crates/unica-coder/src/infrastructure/daemon/v13_source_export.rs::preview_names_the_target_inside_the_workspace_without_writing
  - crates/unica-coder/src/infrastructure/daemon/v13_source_export.rs::preview_refuses_a_target_outside_the_workspace_an_undeclared_set_or_a_write
  - crates/unica-coder/src/infrastructure/daemon/v13_source_export.rs::apply_repeats_the_preview_and_counts_the_exported_files_itself
  - crates/unica-coder/src/infrastructure/daemon/v13_source_export.rs::apply_refuses_a_stale_revision_another_target_and_an_empty_target
gap: https://github.com/IngvarConsulting/unica/issues/950
---

# Выгрузка исходников подтверждает согласованную цель

`pull` адаптера 0.11.2 требует `force:true` и полностью заменяет один
набор исходников. Защита локальных изменений и поколений базы отсутствует.
Аргументы — `force`, необязательные `sourceSet` и `extension`.
Выбор отдельных объектов
не поддерживается. Без `sourceSet` раннер выбирает набор конфигурации;
для набора расширения требуется совпадающее имя расширения.

Preview `pull` принимает только объявленный набор и цель внутри
рабочего пространства, показывает относительный путь и не допускает
ответа о выполненной записи. Применение требует `ifRev` preview. Ответ
раннера о другом наборе или другой цели не признаётся успехом.
Режим и имя расширения в ответе должны совпасть с аргументами вызова.

Ревизия preview связывает проектный файл, объявленный состав наборов,
аргументы, версию раннера и цель выгрузки. Изменение этих входов требует
нового preview; прежний `ifRev` отклоняется до исполняющего вызова.

После выгрузки Unica сама считает обычные файлы в целевом каталоге,
не переходя по символическим ссылкам. Отсутствующая
или пустая цель вызывает отказ. Это квитанция о наличии файлов, а не
проверка полноты или содержимого выгрузки. Состояние информационной базы
между preview и применением не фиксируется.

Проверки используют управляемый раннер и реальные каталоги. Они не
запускают выгрузку платформой 1С и не обещают отмену внешних записей раннера
при отказе.

Полная выгрузка должна также соблюдать [безопасную публикацию дерева](full-export-publication.md).
Подключение этой защиты к текущему `pull` ещё не завершено.
