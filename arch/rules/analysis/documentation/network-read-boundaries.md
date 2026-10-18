---
id: INV.APP.DOCUMENTATION-NETWORK-READ-BOUNDARIES
check:
  - crates/unica-coder/src/infrastructure/kb_1ci.rs::kb_get_denied_by_policy_refuses_without_network
  - crates/unica-coder/src/infrastructure/kb_1ci.rs::kb_denied_by_policy_answers_policy_denied_without_network
  - crates/unica-coder/src/infrastructure/kb_1ci.rs::cancellation_stops_before_the_next_network_request
  - crates/unica-coder/src/infrastructure/kb_1ci.rs::cancellation_during_ranking_yields_a_diagnostic_section_not_ok_with_gaps
  - crates/unica-coder/src/infrastructure/standards_documentation.rs::v8std_get_not_found_and_policy_deny_are_owner_failures
  - crates/unica-coder/src/infrastructure/standards_documentation.rs::v8std_cancellation_stops_before_the_network_for_search_and_get
  - crates/unica-coder/src/infrastructure/standards_documentation.rs::v8std_later_page_failure_or_cancellation_never_publishes_a_partial_stream
gap: https://github.com/IngvarConsulting/unica/issues/953
---

# Сетевое чтение справки соблюдает запрет и отмену

Запрещённый поставщик не начинает сетевых обращений. Уже отменённый вызов
также не начинает сеть и не возвращает успех из кеша. Отмена посреди
дочитывания страниц прекращает следующие обращения; частичная выдача
этого корпуса не публикуется как найденный результат.

Разрешение вычисляется по [политике сети](network-policy.md).
[Запрет поиска v8std после прогрева кеша](denied-provider-cache.md) сохраняет
ту же границу разрешения. Корпус и стоимость чтения базы знаний ограничены
[отдельным правилом](kb-read-scope.md). Проверки отмены не моделируют заранее
отменённый вызов после прогрева кеша; этот сценарий остаётся в `gap`.
