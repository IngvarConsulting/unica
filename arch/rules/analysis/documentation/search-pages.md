---
id: INV.APP.DOCUMENTATION-SEARCH-PAGES
check:
  - crates/unica-documentation/src/projection.rs::full_provider_window_is_a_lower_bound_and_receives_its_own_limit
  - crates/unica-coder/src/infrastructure/daemon/v13_documentation.rs::docs_pages_mixed_sections_and_rejects_changed_answers
  - crates/unica-coder/src/infrastructure/daemon/v13_documentation.rs::late_oversized_docs_hit_refuses_before_first_cursor
  - crates/unica-coder/src/infrastructure/daemon/v13_documentation.rs::one_docs_hit_over_page_target_is_returned_whole
  - crates/unica-coder/src/infrastructure/daemon/v13_documentation.rs::prepared_docs_search_continues_across_calls_before_source_admission
  - crates/unica-documentation-sources/src/standards_documentation.rs::v8std_search_reads_past_fifty_and_replays_the_complete_ranked_stream
  - crates/unica-documentation-sources/src/standards_documentation.rs::old_v8std_without_cursor_marks_a_full_window_incomplete
---

# Поиск справки различает конец страницы и полноту каждого корпуса

Продолжение поиска пересобирает все секции и сверяет отпечаток ответа;
изменившийся ответ не продолжается как прежний. В пределах
[бюджета страницы](../../mcp/read-page-budget.md) совпадения чередуются между
корпусами. До первого курсора проверяется возможность вернуть каждый
неделимый результат целиком, включая результат в конце потока.

Каждая секция сообщает полноту: заполненное окно без подтверждённого
продолжения не считается всем корпусом. `page.stoppedBy: complete`
означает конец полученного потока и не меняет неполноту поиска. Локальные
поставщики перечисляют все совпадения. Адаптер v8std дочитывает страницы
по курсору поставщика; сервер без такого курсора оставляет полное окно
из 50 результатов явно неполным.

Поиск и его продолжение доступны до допуска исходников рабочего пространства.
Чтение открытого документа имеет отдельный
[контракт точных текстовых страниц](document-text-pages.md).
