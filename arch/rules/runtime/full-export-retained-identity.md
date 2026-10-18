---
id: INV.RUNTIME.FULL-EXPORT-RETAINED-IDENTITY
check:
  - crates/unica-coder/src/infrastructure/platform/full_dump_publication.rs::concurrent_target_during_capture_error_quarantine_is_preserved
  - crates/unica-coder/src/infrastructure/platform/full_dump_publication.rs::concurrent_file_during_capture_error_quarantine_is_preserved
  - crates/unica-coder/src/infrastructure/platform/full_dump_publication.rs::concurrent_target_during_capture_error_quarantine_survives_recovery_failure
gap: https://github.com/IngvarConsulting/unica/issues/981
---

# Полная выгрузка изменяет только удержанные объекты файловой системы

Чтение, проверка, публикация и очистка полной выгрузки связаны с удержанными
идентичностями каталогов и файлов. Подмена имени не переключает операцию
на новый объект. Символические ссылки и reparse points не обходятся;
многоссылочные файлы и неподдерживаемые виды записей не принимаются
как проверенные исходные данные.

Очистка удаляет только собственные объекты. Подменённый файл или каталог
остаётся нетронутым; ошибка идентичности сообщается как нарушение целостности.
Удаление ссылки при очистке не затрагивает её цель. Delete-pending имя
в Windows не считается подтверждённым отсутствием.

Публичный `pull` доступен в ограниченном режиме с `force:true`.
Его текущие проверки не доказывают эту гарантию; подключение защиты
остаётся разрывом из `gap`.
