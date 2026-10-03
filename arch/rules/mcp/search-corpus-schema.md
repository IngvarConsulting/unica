---
id: INV.WIRE.SEARCH-CORPUS
check:
  - crates/unica-coder/src/application/v13/tool_catalog.rs::search_publishes_exactly_two_corpora_and_defaults_to_text
  - crates/unica-coder/src/application/v13/find.rs::find_returns_address_facts_and_not_content_hits
  - crates/unica-coder/src/application/v13/find.rs::exact_kind_alias_filter_limit_and_nearest_are_deterministic
  - crates/unica-coder/tests/v13_search_integration.rs::canonical_search_is_source_scoped_and_rejects_legacy_call_shape
  - crates/unica-coder/tests/v13_search_integration.rs::names_search_pages_all_ranked_matches_and_rejects_changed_answers
  - crates/unica-coder/tests/v13_search_integration.rs::unreadable_source_lines_keep_other_hits_and_truthful_coverage_across_pages
  - crates/unica-coder/tests/v13_search_integration.rs::text_file_coverage_counts_all_uncovered_files_when_details_are_capped
  - crates/unica-coder/tests/v13_search_integration.rs::text_search_refuses_unvisited_depth_instead_of_claiming_complete_coverage
---

# Поиск имён и текста показывает разные сведения о совпадении

`corpus: names` ищет имена и синонимы метаданных, `text` — текст модулей BSL.
Допустимы только эти два значения; без аргумента выбирается `text`.

Попадание по имени содержит логический адрес `at`, вид `kind` и название
`title`. Фильтр `kind` оставляет один вид узлов; `approximate` явно отмечает
совпадение по близости имени. Это не признак полноты поиска.

Попадание в тексте содержит `scope`, `line`, `column` и `snippet`.
Физический путь не выдаётся ни в одном своде. Текстовый результат читает доступные исходники и сообщает неизвестную
актуальность и живое продолжение; `rev` чтения не выдаётся. Справочник имён
использует раскладку исходников. Курсор поиска имён связан с
полным упорядоченным публичным ответом и полнотой чтения дескрипторов:
если что-либо из них изменилось, продолжение отвечает `stale_cursor`.

Проверки покрывают схему, внутренний справочник, публичные страницы имён и
области основного текстового поиска через MCP.

Локальный `text`-поиск сообщает `data.fileCoverage` отдельно от страниц
совпадений. Если обход остановлен до конца корпуса ради следующей страницы,
`scanComplete: false`, а `uncovered` — нижняя граница числа файлов, которые
не удалось дочитать. На последней странице обход завершён и число точно;
`complete: true` допустимо только когда `uncovered` равно нулю. Слишком
длинная строка или неверный UTF-8 оставляют файл непокрытым, но найденные
совпадения в других файлах и прочитанной части сохраняются. `summary`
называет такой результат частичным, даже если `page.stoppedBy: complete`
завершает выдачу доступных совпадений. `details` содержат не более 32 файлов;
`detailsTruncated` отличает неполный список от полного. `fileId` — устойчивый
в пределах обхода и области порядковый номер файла в обходе, а не путь и не
логический адрес.
