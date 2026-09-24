---
id: INV.APP.CODE-SEARCH-RESULT-EVIDENCE
check:
  - crates/unica-coder/src/application/code_intelligence.rs::coordinator_preserves_the_replaceable_provider_identity_for_a_role
  - crates/unica-coder/src/application/code_intelligence.rs::coordinator_preserves_provider_local_ranks_and_limit_claim
  - crates/unica-coder/src/domain/code_intelligence.rs::search_section_serializes_role_provenance_completeness_and_logical_location
  - crates/unica-coder/src/domain/code_intelligence.rs::timed_out_section_serializes_a_machine_readable_terminal_reason
  - crates/unica-coder/src/domain/code_intelligence.rs::section_rejects_an_invalid_terminal_reason_contract
  - crates/unica-coder/src/domain/code_intelligence.rs::partial_section_requires_proven_hits_and_diagnostics
  - crates/unica-coder/src/application/mod.rs::code_search_schema_accepts_a_real_partial_provider_section
  - crates/unica-coder/src/application/code_intelligence.rs::malformed_provider_subset_remains_useful_without_claiming_complete_coverage
  - crates/unica-coder/src/infrastructure/daemon/v13_service.rs::public_role_search_surfaces_partial_provider_warning_and_hit
  - crates/unica-coder/src/infrastructure/daemon/v13_service.rs::provider_limit_remains_visible_after_the_last_received_page
  - crates/unica-coder/src/infrastructure/code_intelligence.rs::bsl_analyzer_at_requested_limit_does_not_claim_exhaustion
  - crates/unica-coder/src/infrastructure/code_intelligence.rs::rlm_at_requested_limit_does_not_claim_exhaustion
  - crates/unica-coder/src/infrastructure/code_intelligence.rs::bsl_analyzer_does_not_claim_complete_when_one_header_is_unreadable
  - crates/unica-coder/src/infrastructure/code_intelligence.rs::rlm_parser_keeps_valid_rows_and_reports_malformed_siblings
  - crates/unica-coder/src/infrastructure/code_intelligence.rs::git_grep_keeps_valid_hits_but_reports_malformed_siblings_as_partial
  - crates/unica-coder/src/infrastructure/platform/process.rs::completed_line_drain_keeps_stop_only_for_successful_children
  - crates/unica-coder/src/infrastructure/code_intelligence.rs::cancelled_projection_is_not_reported_as_a_malformed_search_result
gap: https://github.com/IngvarConsulting/unica/issues/871
---

# Результат поиска сохраняет источник и пределы достоверности

Секция результата называет способ поиска и фактического поставщика отдельно.
Она сообщает, завершён ли поиск, точен ли счёт совпадений и как они упорядочены.
Ранг поставщика сохраняется как его собственная оценка.
Если поставщик вернул ровно запрошенное число результатов без признака конца,
секция сообщает о достижении предела и даёт нижнюю оценку числа совпадений.
Если поставщик завершил поиск, но отдельный результат нельзя разобрать,
остальные проверенные совпадения сохраняются, а секция получает статус
`partial` и нижнюю оценку.
Когда при этом достигнут и предел поставщика, `partial` называет потерю
результата, а диагностика отдельно сообщает о пределе.

Причина остановки согласована со статусом и сообщает, можно ли повторить
запрос. Истечение срока обозначается как `deadlineExceeded` с возможностью
повтора, а не как успешный поиск без совпадений.

Проверки подтверждают форму секции и передачу данных координатором.
Выбор поставщика на публичном входе `unica.search` проверен отдельно в #960.

Порядок выдачи зависит от задачи поиска. Смысловой поиск показывает сначала
наиболее подходящие совпадения; поиск имён — точные совпадения перед близкими;
буквальный поиск — места в порядке файлов. Неполнота результата обозначается
явно. Сводка не заменяет первую страницу полезных совпадений.
Общий [предел страницы](../mcp/read-page-budget.md) сохраняется.
Провайдерный поиск выдаёт страницы полученного окна, сохраняя его роль,
поставщика и полноту. Курсор не доказывает, что поставщик нашёл всё: при
достижении его предела последняя страница сохраняет `limitReached` и нижнюю
оценку. Продолжение за пределами окна поставщика остаётся в `gap`.
