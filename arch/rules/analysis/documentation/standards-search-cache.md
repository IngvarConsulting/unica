---
id: INV.APP.DOCUMENTATION-STANDARDS-CACHE
check:
  - crates/unica-coder/src/infrastructure/standards_documentation.rs::a_repeated_search_answers_from_the_process_cache
  - crates/unica-coder/src/infrastructure/standards_documentation.rs::an_expired_search_cache_entry_is_refetched
  - crates/unica-coder/src/infrastructure/standards_documentation.rs::the_search_cache_is_bounded_and_purges_expired_entries
  - crates/unica-coder/src/infrastructure/standards_documentation.rs::the_query_is_normalized_before_the_server_call
  - crates/unica-coder/src/infrastructure/standards_documentation.rs::v8std_search_reads_past_fifty_and_replays_the_complete_ranked_stream
---

# Повторный поиск стандартов использует действующий кеш

Перед отправкой запроса повторяющиеся пробельные символы сводятся к одному
пробелу. Ранжирование стандартов остаётся за сервером.

Повторный запрос использует действующий кеш процесса. По истечении срока
ответ запрашивается заново. Число записей ограничено; истёкшие записи
удаляются при пополнении кеша.

Для нового поиска кешируется весь проверенный ответ поставщика, а не только
первая страница его выдачи. Продолжение уже выданного курсора `unica.docs`
заново читает ранжированный ответ v8std и не пользуется кешем: изменение
результатов должно сделать курсор устаревшим до выдачи следующей страницы.
Успешная перепроверка обновляет кеш новым полным ответом, чтобы после
устаревания курсора новый поиск мог начать с актуальной выдачи. Повтор нового
поиска без курсора продолжает пользоваться действующим кешем.

Кеш не отменяет запрет поставщика или отмену вызова. Проверка запрета кеша
закреплена в `denied-provider-cache.md`; отмена — в `network-read-boundaries.md`.
