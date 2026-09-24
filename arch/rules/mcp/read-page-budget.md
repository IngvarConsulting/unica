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
Продолжение за пределами этого окна и страницы `docs` остаются в `gap`.
