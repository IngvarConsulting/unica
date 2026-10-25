---
id: INV.APP.LITERAL-SEARCH
check:
  - crates/unica-coder/src/infrastructure/daemon/v13_service.rs::literal_search_engines_agree_on_matches_order_and_addresses
  - crates/unica-coder/src/infrastructure/daemon/v13_service.rs::literal_module_scope_searches_only_the_selected_module_with_either_engine
  - crates/unica-coder/src/infrastructure/daemon/v13_service.rs::literal_search_reads_submodules_and_embedded_repositories
  - crates/unica-coder/src/infrastructure/daemon/v13_service.rs::git_first_page_continues_by_the_walk_to_the_last_match
  - crates/unica-coder/src/infrastructure/daemon/v13_service.rs::walk_literal_continuation_resumes_at_its_file
  - crates/unica-coder/src/infrastructure/daemon/v13_service.rs::walk_pages_end_by_time_and_continue_without_loss
  - crates/unica-coder/src/infrastructure/daemon/v13_service.rs::literal_search_skips_publication_artifacts_and_names_an_unreadable_entry
  - crates/unica-coder/src/infrastructure/daemon/v13_service.rs::literal_search_ignores_the_retired_git_grep_timeout_key
  - crates/unica-coder/src/infrastructure/daemon/v13_service.rs::first_literal_page_counts_the_whole_corpus_by_kind_and_module
  - crates/unica-coder/src/infrastructure/daemon/v13_service.rs::literal_volume_names_twenty_leading_modules_and_counts_the_rest
  - crates/unica-coder/src/infrastructure/daemon/v13_service.rs::literal_volume_counts_uncovered_files_past_the_page_apart_from_its_coverage
  - crates/unica-coder/src/infrastructure/daemon/v13_service.rs::cancelled_literal_search_answers_no_volume
  - crates/unica-coder/tests/v13_search_integration.rs::literal_search_pages_past_two_hundred_matches_without_loss
  - crates/unica-coder/tests/v13_search_integration.rs::canonical_search_is_source_scoped_and_rejects_legacy_call_shape
  - crates/unica-coder/tests/v13_search_integration.rs::text_search_keeps_nested_module_order_across_pages
  - crates/unica-navigation/src/result_store.rs::search_next_answer_rebinds_only_the_answer_and_replays_its_token
---

# Буквальный поиск без роли доходит до последнего совпадения

`unica.search` без роли ищет в тексте модулей BSL строку целиком, с
`regex: true` — регулярное выражение, с `ignoreCase: true` — без учёта
регистра. Область — корень набора, ветвь или владелец метаданных либо один
модуль; адрес модуля не расширяет поиск до соседних модулей. Без `scope`
поиск охватывает все допущенные наборы исходников в порядке допуска.
Совпадения идут в порядке файлов, без ранга и оценки.

Выдача идёт страницами, и курсор продолжает её до конца корпуса. Число
совпадений продолжение не ограничивает: на неизменном источнике каждое
совпадение достижимо, ни одно не повторяется и не пропускается. Повтор
курсора на неизменном источнике выдаёт ту же страницу и то же продолжение,
кроме страницы, закончившейся по времени. Изменение уже выданных совпадений
файла, на котором продолжается выдача, или исчезновение этого файла дают
`stale_cursor`; изменение после курсора читает следующая страница. Курсор,
после которого совпадений не осталось, даёт пустую последнюю страницу с
завершённым поиском.

Первый ответ несёт сводку объёма всей области поиска: точное число
совпадений и файлов с ними, раскладку по видам объектов, ограниченный список
модулей с наибольшим числом совпадений, с их адресами и счётом, и одну строку
счёта остальных модулей. Совпадения без доказанного адреса считаются
отдельно. Если строки части файлов не прочитаны, сводка называет число
таких файлов, а её счёт относится к прочитанному тексту. Продолжения сводку
не повторяют. Сводка выдаётся только после завершённого подсчёта и не
бывает нижней оценкой: долгий подсчёт продолжается заданием, а отменённый
или сбойный поиск не выдаёт ни сводки, ни страницы.

Продолжение может закончиться по времени (`page.stoppedBy: deadline`), в том
числе без совпадений; курсор продолжает выдачу с места остановки без потерь
и повторов. Срока операции у поиска нет: медленная страница продолжается
заданием, остановить её может только отмена, и отменённый поиск страницы не
выдаёт.

Промежуточная страница сообщает незавершённую выдачу, последняя —
завершённую; сводка первого ответа относится ко всему корпусу, а не к
выданной части. Пустой результат выдаётся только при доказанном завершении;
сбой чтения или недоступный каталог пустой выдачей не становятся.
