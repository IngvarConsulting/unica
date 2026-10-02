---
id: INV.PKG.RESUMABLE-DOWNLOAD
check:
  - crates/unica-bootstrap/tests/runtime_install.rs::an_interrupted_download_resumes_in_the_next_session
  - crates/unica-bootstrap/src/download.rs::a_resumed_download_asks_only_for_the_missing_tail
  - crates/unica-bootstrap/src/download.rs::a_server_that_forgets_the_range_still_leaves_the_whole_file
  - crates/unica-bootstrap/tests/runtime_install.rs::a_partial_that_hashes_wrong_is_dropped_instead_of_resumed_forever
---

# Оборванная загрузка продолжается с сохранённых байтов

После обрыва загрузки установщик сохраняет полученную часть файла.
Следующая попытка той же поставки и платформы в том же кеше использует
сохранившуюся часть: запрашивает недостающие байты через HTTP Range.

При ответе сервера `206` недостающая часть дописывается. Если сервер
возвращает весь файл ответом `200`, он заменяет неполную копию.

Полученный файл проходит [проверку SHA-256](archive-checksum-before-unpack.md).
При несовпадении суммы загруженная копия удаляется, чтобы следующая попытка
начала загрузку заново.
