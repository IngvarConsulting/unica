---
id: INV.APP.DOCUMENTATION-NETWORK-READ-BOUNDARIES
check:
  - crates/unica-coder/src/infrastructure/kb_1ci.rs::kb_get_refuses_a_locator_with_parent_segments
  - crates/unica-coder/src/infrastructure/kb_1ci.rs::kb_get_denied_by_policy_refuses_without_network
  - crates/unica-coder/src/infrastructure/kb_1ci.rs::kb_denied_by_policy_answers_policy_denied_without_network
  - crates/unica-coder/src/infrastructure/kb_1ci.rs::cancellation_stops_before_the_next_network_request
  - crates/unica-coder/src/infrastructure/kb_1ci.rs::cancellation_during_ranking_yields_a_diagnostic_section_not_ok_with_gaps
  - crates/unica-coder/src/infrastructure/kb_1ci.rs::kb_provider_matches_titles_and_reads_only_the_top_pages
  - crates/unica-coder/src/infrastructure/standards_documentation.rs::v8std_get_not_found_and_policy_deny_are_owner_failures
  - crates/unica-coder/src/infrastructure/standards_documentation.rs::v8std_cancellation_stops_before_the_network_for_search_and_get
gap: https://github.com/IngvarConsulting/unica/issues/953
---

# Чтение сетевой справки соблюдает запрет, отмену и границы руководств

Запрещённый поставщик не начинает сетевых обращений. Уже отменённый вызов
также не начинает сеть и не возвращает успех из кеша. Отмена посреди
дочитывания страниц прекращает следующие обращения; частичная выдача
этого корпуса не публикуется как найденный результат.

База знаний читает объявленные руководства разработчика и администратора.
Адреса за пределами этих руководств и выход через `..` отклоняются.
Поиск берёт кандидатов из дерева руководств, а не из произвольных ссылок.

Число дочитываемых страниц ограничено. Остальные совпадения сохраняют
заголовок и локатор без фрагмента; обращения разнесены по времени.
Тесты пока не закрепляют все эти границы — недостающие сценарии перечислены
в связанной задаче.
