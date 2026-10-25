---
id: INV.APP.LITERAL-SEARCH
check:
  - crates/unica-coder/src/infrastructure/daemon/v13_service.rs::literal_search_engines_agree_on_matches_order_and_addresses
  - crates/unica-coder/src/infrastructure/daemon/v13_service.rs::literal_module_scope_searches_only_the_selected_module_with_either_engine
  - crates/unica-coder/src/infrastructure/daemon/v13_service.rs::git_literal_continuation_proves_the_delivered_part
  - crates/unica-coder/src/infrastructure/daemon/v13_service.rs::walk_literal_continuation_resumes_at_its_file
  - crates/unica-coder/src/infrastructure/daemon/v13_service.rs::walk_pages_end_by_time_and_continue_without_loss
  - crates/unica-coder/src/infrastructure/daemon/v13_service.rs::literal_search_skips_publication_artifacts_and_names_an_unreadable_entry
  - crates/unica-coder/src/infrastructure/daemon/v13_service.rs::literal_search_ignores_the_retired_git_grep_timeout_key
  - crates/unica-coder/tests/v13_search_integration.rs::literal_search_pages_past_two_hundred_matches_without_loss
  - crates/unica-coder/tests/v13_search_integration.rs::canonical_search_is_source_scoped_and_rejects_legacy_call_shape
  - crates/unica-coder/tests/v13_search_integration.rs::text_search_keeps_nested_module_order_across_pages
  - crates/unica-navigation/src/result_store.rs::search_next_answer_rebinds_only_the_answer_and_replays_its_token
---

# Буквальный поиск без роли доходит до последнего совпадения

`unica.search` без роли ищет в тексте модулей BSL строку целиком, с
`regex: true` — регулярное выражение, с `ignoreCase: true` — без учёта
регистра. Область — корень набора, ветвь или владелец метаданных либо один
модуль; адрес модуля не расширяет поиск до соседних модулей. Без `scope` все
допущенные наборы исходников проходятся одним обходом в порядке допуска.
Совпадения идут в порядке файлов, без ранга и оценки. Ответ на один и тот же
корпус не зависит от того, читает ли его Unica через Git или своим обходом:
те же совпадения, порядок, адреса и столбцы. Исключение — файл со строкой,
которую нельзя прочитать: его покрытие задаёт
[правило схемы](../mcp/search-corpus-schema.md).

Выдача идёт страницами, и курсор продолжает её до конца корпуса. Число
совпадений продолжение не ограничивает: на неизменном источнике каждое
совпадение достижимо, ни одно не повторяется и не пропускается. Страница
удерживает только свои совпадения. Повтор курсора на неизменном источнике
выдаёт ту же страницу и то же продолжение, кроме страницы, закончившейся по
времени. Изменение после курсора читает следующая страница. В рабочем дереве
Git изменение в уже выданных
совпадениях, включая вставку или удаление совпадения перед курсором, и
курсор, после которого не осталось совпадений, дают `stale_cursor`. Без Git
обход продолжается с файла, на котором остановился: `stale_cursor` дают
изменение этого файла в выданной части и его исчезновение, а уже пройденные
файлы не перечитываются.

Без Git полный обход крупной конфигурации не умещается в один вызов: страница
может закончиться по времени (`page.stoppedBy: deadline`), в том числе без
совпадений, и курсор продолжает обход с места остановки. Последовательность
страниц доходит до конца корпуса. Граница такой страницы зависит от скорости
чтения, поэтому её повтор может закончиться в другом месте; совпадения и
тогда не теряются и не повторяются. Поиск идёт без срока операции; его
останавливает только отмена, и отменённый поиск страницы не выдаёт.

Промежуточная страница сообщает незавершённый обход, последняя — завершённый.
Доказанный пустой результат — успешный ответ с завершённым обходом; сбой
чтения, недоступный каталог или неполный ответ Git пустой выдачей не
становятся. Покрытие файлов задаёт [правило схемы](../mcp/search-corpus-schema.md),
размер страницы — [общий контракт](../mcp/read-page-budget.md), жизнь
курсора — [правило владельца](../mcp/search-cursor-owner.md).
