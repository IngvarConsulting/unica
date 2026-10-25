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
  - crates/unica-coder/tests/v13_search_integration.rs::text_hits_name_their_module_and_view_reads_the_same_line
  - crates/unica-coder/tests/v13_search_integration.rs::broad_text_search_scopes_name_the_module_of_every_hit
  - crates/unica-coder/src/infrastructure/daemon/v13_service.rs::literal_search_hit_without_an_address_carries_no_path
  - crates/unica-coder/tests/platform/event_sources.rs::module_paths_map_to_the_logical_module_view_reads
  - crates/unica-coder/tests/platform/event_sources.rs::files_outside_the_module_layout_have_no_logical_address
---

# Поиск имён и текста показывает разные сведения о совпадении

`corpus: names` ищет имена и синонимы метаданных, `text` — текст модулей BSL.
Допустимы только эти два значения; без аргумента выбирается `text`.

Попадание по имени содержит логический адрес `at`, вид `kind` и название
`title`. Фильтр `kind` оставляет один вид узлов; `approximate` явно отмечает
совпадение по близости имени. Это не признак полноты поиска.

Попадание в тексте содержит `scope`, `line`, `column` и `snippet`.
`scope` — общая область поиска; модуль совпадения называет отдельное поле
`at`. Это логический адрес модуля, раскладка которого помещает найденный
файл, и `view` его `Body` показывает ту же строку и столбец; метка порядка
байтов в начале файла не входит ни в столбец, ни в `snippet`. Выдаётся
только доказанный адрес: владелец модуля зарегистрирован в `Configuration.xml`
того же набора (у расширения — в его собственном), у внешней обработки или
отчёта есть корневой дескриптор. Совпадение без доказанного адреса не
теряется: вместо `at` оно несёт `fileId` того же вида, что в `fileCoverage`,
и причину `unaddressed` — `outside_module_layout` (файл вне раскладки
модулей), `owner_unproven` (владелец не зарегистрирован) или
`unsupported_source_profile` (у набора нет профиля платформы).
Физический путь не выдаётся ни в одном своде. Текстовый результат читает доступные исходники и сообщает неизвестную
актуальность и живое продолжение; `rev` чтения не выдаётся. Справочник имён
использует раскладку исходников. Курсор поиска имён связан с
полным упорядоченным публичным ответом и полнотой чтения дескрипторов:
если что-либо из них изменилось, продолжение отвечает `stale_cursor`.

Проверки покрывают схему, внутренний справочник, публичные страницы имён и
области основного текстового поиска через MCP.

Локальный `text`-поиск сообщает `data.fileCoverage` отдельно от страниц
совпадений. Если поиск остановлен до конца корпуса ради следующей страницы,
`scanComplete: false`, а `uncovered` — нижняя граница числа файлов, которые
не удалось дочитать. На последней странице поиск завершён и число точно;
`complete: true` допустимо только когда `uncovered` равно нулю. Строка,
которая может содержать совпадение, но длиннее предела или не читается как
UTF-8, оставляет файл непокрытым; найденные совпадения в других файлах и
прочитанной части сохраняются. `summary`
называет такой результат частичным, даже если `page.stoppedBy: complete`
завершает выдачу доступных совпадений. `details` содержат не более 32 файлов,
прочитанных этой страницей, а `uncovered` считает и файлы прежних страниц;
`detailsTruncated` отличает неполный список от полного. `fileId` — устойчивый
идентификатор файла в его наборе исходников, а не путь и не логический
адрес.

Текстовые страницы читают текущие исходники с места курсора и сообщают
`dataFreshness: unknown` и `pageConsistency: live`: при изменении корпуса
совпадения могут сдвинуться. Общая ревизия не проверяется. Размер страницы
задаёт [общий контракт](read-page-budget.md).
