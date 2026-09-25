---
id: INV.WIRE.READ-PAGE-BUDGET
check:
  - crates/unica-coder/src/application/v13/view.rs::addressed_collection_prefers_64_kib_pages_and_replays
  - crates/unica-coder/src/application/v13/view.rs::one_item_over_the_page_target_is_returned_whole
  - crates/unica-coder/src/application/v13/view.rs::snapshot_over_byte_quota_refuses_before_promising_a_continuation
  - crates/unica-coder/src/application/v13/view.rs::a_late_oversized_item_refuses_before_the_first_page
  - crates/unica-coder/src/application/v13/view.rs::supplied_graph_collection_uses_stable_pages_bound_to_its_owner
  - crates/unica-coder/tests/v13_search_integration.rs::canonical_search_is_source_scoped_and_rejects_legacy_call_shape
  - crates/unica-coder/tests/v13_search_integration.rs::names_search_pages_all_ranked_matches_and_rejects_changed_answers
  - crates/unica-coder/tests/v13_search_integration.rs::canonical_search_is_source_scoped_and_rejects_legacy_call_shape
  - crates/unica-coder/src/application/v13/check.rs::check_pages_preserve_verdict_and_every_diagnostic_with_replay
  - crates/unica-coder/src/application/v13/check.rs::check_returns_whole_large_finding_and_refuses_late_oversize_before_page_one
  - crates/unica-coder/src/application/v13/check.rs::check_refuses_before_first_page_when_snapshot_cannot_be_retained
  - crates/unica-coder/src/application/v13/check.rs::late_finding_at_transport_edge_is_refused_before_issuing_a_cursor
  - crates/unica-coder/src/infrastructure/daemon/v13_service.rs::bsl_check_never_calls_a_truncated_analyzer_result_complete
  - crates/unica-coder/src/infrastructure/daemon/v13_service.rs::bsl_check_sees_an_error_after_two_hundred_warnings
  - crates/unica-coder/src/application/documentation.rs::full_provider_window_is_a_lower_bound_and_receives_its_own_limit
  - crates/unica-coder/src/infrastructure/daemon/v13_documentation.rs::docs_pages_mixed_sections_and_rejects_changed_answers
  - crates/unica-coder/src/infrastructure/daemon/v13_documentation.rs::late_oversized_docs_hit_refuses_before_first_cursor
  - crates/unica-coder/src/infrastructure/daemon/v13_documentation.rs::one_docs_hit_over_page_target_is_returned_whole
  - crates/unica-coder/src/infrastructure/daemon/v13_documentation.rs::prepared_docs_search_continues_across_calls_before_source_admission
  - crates/unica-coder/src/infrastructure/daemon/v13_documentation.rs::docs_locator_accepts_pagination_arguments
  - crates/unica-coder/src/infrastructure/daemon/v13_documentation.rs::opened_document_pages_reassemble_exact_utf8_text_and_replay
  - crates/unica-coder/src/infrastructure/daemon/v13_documentation.rs::document_over_transport_size_on_one_line_starts_with_a_useful_page
  - crates/unica-coder/src/infrastructure/daemon/v13_documentation.rs::oversized_document_metadata_refuses_before_issuing_a_cursor
  - crates/unica-coder/src/infrastructure/standards_documentation.rs::v8std_search_reads_past_fifty_and_replays_the_complete_ranked_stream
  - crates/unica-coder/src/infrastructure/standards_documentation.rs::old_v8std_without_cursor_marks_a_full_window_incomplete
  - crates/unica-coder/src/infrastructure/daemon/v13_service.rs::provider_search_pages_all_received_hits_and_rejects_changed_answers
  - crates/unica-coder/src/infrastructure/daemon/v13_service.rs::provider_limit_remains_visible_after_the_last_received_page
  - crates/unica-coder/src/infrastructure/daemon/v13_service.rs::late_oversized_provider_hit_refuses_before_first_cursor
gap: https://github.com/IngvarConsulting/unica/issues/871
---

# Страница чтения сообщает полноту и сохраняет доступ к элементам

Для страничных ответов канонической поверхности `limit` по умолчанию равен
20, максимум — 50. Ответ называет, завершена ли выдача и почему страница
остановилась: по числу элементов или по объёму. Неполная страница даёт
применимое продолжение; содержимое элемента не обрезается молча.

Для обычной адресной коллекции `view` 64 КиБ — целевой размер страницы,
а не причина отказать в неделимом элементе или метаданных узла. Такая
страница может быть больше целевого размера. Верхний предел сохранённого
результата остаётся технической границей: если один ответ в неё не помещается,
инструмент явно отказывает и не обещает курсор, который не сможет выполнить.

Это целевой контракт: текущие инструменты используют разные пределы.
Адресный `check` вычисляет вердикт по всем находкам, затем отдаёт диагностики
страницами с неизменным вердиктом. Если снимок не помещается в хранилище
курсоров или поздняя находка превышает транспортный предел, он отказывает до
первой страницы. `view` заранее читает всю
коллекцию, но создаёт страницы по запросу; при превышении квоты снимка
отказывает до выдачи курсора. Локальный текстовый `search` выдаёт страницы,
повторно читая исходники по курсору и проверяя их ревизии. Поиск по именам
пересобирает ранжированный ответ и проверяет его отпечаток перед продолжением;
он не заявляет ревизию исходников. Провайдерные роли пересобирают ограниченное
окно результатов и сверяют отпечаток всего ответа перед продолжением; достижение
предела поставщика остаётся явным даже на последней странице полученного окна.
Поиск справки также пересобирает все секции и сверяет их отпечаток. В пределах бюджета страницы
совпадения чередуются между корпусами. Каждая секция сообщает полноту:
заполненное окно без подтверждённого продолжения не выдаётся за весь корпус.
`page.stoppedBy: complete` завершает полученный поток, даже если поиск в нём
неполон. Локальные поставщики справки перечисляют все совпадения. Адаптер v8std
дочитывает страницы по курсору поставщика; прежний сервер без курсора оставляет
полное окно из 50 результатов явно неполным. До первого курсора проверяется,
что каждый неделимый результат можно вернуть. Открытый документ
с коротким текстом сохраняет полный ответ; длинный текст делится на фрагменты
по строкам и границам UTF-8, а курсор связан с отпечатком всего документа.
Если владелец вернул текст, даже строка длиннее 8 МиБ начинается полезной
страницей. Сведения о документе
не делятся: если они сами превышают транспортный предел, отказ приходит до
первого курсора. Кодовые роли получают окно не более 200 результатов.
При 200 полученных совпадениях ответ рекомендует уточнить запрос.
Если поставщик достиг предела, ответ сохраняет `limitReached` и не обещает
выдачу за пределами окна.
Меньший внутренний предел поставщика также остаётся видимой неполнотой;
из него не следует, что запрос слишком широк. Курсор v8std ещё должен быть развёрнут
на публичном сервере и проверен через установленный пакет.
