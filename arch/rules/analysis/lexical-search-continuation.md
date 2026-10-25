---
id: INV.APP.LEXICAL-SEARCH-CONTINUATION
check:
  - crates/unica-coder/tests/v13_search_integration.rs::lexical_role_search_pages_past_two_hundred_matches_without_loss
  - crates/unica-coder/src/infrastructure/daemon/v13_service.rs::lexical_role_search_continues_past_the_former_window_through_the_public_entry
  - crates/unica-coder/src/infrastructure/daemon/v13_service.rs::lexical_role_search_continues_after_a_byte_stopped_page
  - crates/unica-coder/src/infrastructure/daemon/v13_service.rs::unscoped_lexical_continuation_crosses_source_sets_as_one_traversal
  - crates/unica-coder/tests/platform/code_intelligence_integration.rs::git_grep_continuation_passes_over_the_delivered_prefix_and_proves_it
  - crates/unica-navigation/src/result_store.rs::search_next_answer_rebinds_only_the_answer_and_replays_its_token
---

# Буквальный поиск роли lexical продолжается до последнего совпадения

`unica.search` с ролью `lexical` выдаёт совпадения страницами в порядке
обхода поставщика, и курсор продолжает выдачу до конца обхода. Число
совпадений продолжение не ограничивает: каждое совпадение, до которого обход
доходит в пределах срока поставщика, достижимо; ни одно не повторяется
и не пропускается. Из совпадений страница удерживает только
свои, сколько бы их ни выдали раньше.

Курсор связан с последовательностью уже выданных совпадений. На неизменном
источнике повтор курсора выдаёт ту же страницу и то же продолжение.
Изменение в выданной части, включая вставку или удаление совпадения перед
курсором, даёт `stale_cursor`; изменение после курсора читает следующая
страница. Если после курсора не осталось ни одного совпадения, это тоже
`stale_cursor`, а не пустой поиск. Если срок поставщика истёк раньше, чем
обход дошёл до курсора, ответ — отказ `deadline_exceeded`, а не пустая
страница.

Промежуточная страница сообщает незавершённый поиск и нижнюю оценку числа
совпадений, последняя — завершённый поиск и точный счёт. Истечение срока
и неразобранные записи остаются видимой неполнотой по
[правилу результата](search-result-evidence.md). Размер страницы задаёт
[общий контракт](../mcp/read-page-budget.md), жизнь курсора —
[правило владельца](../mcp/search-cursor-owner.md). Окно ролей `symbol`
и `semantic` этим правилом не меняется.
