---
id: INV.APP.CODE-SEARCH-RESULT-EVIDENCE
check:
  - crates/unica-coder/src/application/code_intelligence.rs::coordinator_preserves_the_replaceable_provider_identity_for_a_role
  - crates/unica-coder/src/application/code_intelligence.rs::coordinator_preserves_provider_local_ranks_and_limit_claim
  - crates/unica-coder/src/domain/code_intelligence.rs::search_section_serializes_role_provenance_completeness_and_logical_location
  - crates/unica-coder/src/domain/code_intelligence.rs::timed_out_section_serializes_a_machine_readable_terminal_reason
  - crates/unica-coder/src/domain/code_intelligence.rs::section_rejects_an_invalid_terminal_reason_contract
gap: https://github.com/IngvarConsulting/unica/issues/960
---

# Результат поиска сохраняет источник и пределы достоверности

Секция результата называет способ поиска и фактического поставщика отдельно.
Она сообщает, завершён ли поиск, точен ли счёт совпадений и как они упорядочены.
Ранг поставщика сохраняется как его собственная оценка.

Причина остановки согласована со статусом и сообщает, можно ли повторить
запрос. Истечение срока обозначается как `deadlineExceeded` с возможностью
повтора, а не как успешный поиск без совпадений.

Проверки подтверждают форму секции и передачу данных координатором.
Они не подтверждают выбор поставщика на публичном входе `unica.search`.

Порядок выдачи зависит от задачи поиска. Смысловой поиск показывает сначала
наиболее подходящие совпадения; поиск имён — точные совпадения перед близкими;
буквальный поиск — места в порядке файлов. Неполнота результата обозначается
явно. Сводка не заменяет первую страницу полезных совпадений.
Общий [предел страницы](../mcp/read-page-budget.md) сохраняется.
Проверка этой выдачи на публичном входе остаётся в `gap`.
