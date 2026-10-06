---
id: INV.APP.SEARCH-CURSOR-OWNER
check:
  - crates/unica-coder/src/application/result_store.rs::live_search_keeps_all_300_independent_chains_and_their_replay
  - crates/unica-coder/src/application/result_store.rs::live_search_keeps_the_exact_question_past_eight_mib
  - crates/unica-coder/src/application/result_store.rs::search_cursor_lifetime_is_the_actual_owner_without_retained_entries
  - crates/unica-coder/src/application/result_store.rs::search_authentication_rejects_every_changed_token_byte_and_noncanonical_encoding
  - crates/unica-coder/src/application/result_store.rs::search_every_question_field_precedes_answer_fingerprint_validation
  - crates/unica-coder/src/application/result_store.rs::search_failed_key_entropy_publishes_nothing_and_retry_uses_a_real_key
  - crates/unica-coder/src/application/result_store.rs::search_failed_chain_entropy_preserves_the_existing_owner_and_tokens
  - crates/unica-coder/src/application/result_store.rs::search_concurrent_chains_and_successor_replay_share_one_immutable_owner
  - crates/unica-coder/tests/v13_search_integration.rs::names_search_pages_all_ranked_matches_and_rejects_changed_answers
  - crates/unica-coder/src/infrastructure/daemon/v13_documentation.rs::opened_document_pages_reassemble_exact_utf8_text_and_replay
---

# Поисковый курсор действует, пока жив его владелец

Курсор поиска связан с точным вопросом, порядком выбранных наборов исходников
и размером страницы. Подмена вопроса или токена не позволяет продолжить выдачу.
Если ответ проверяется по отпечатку, изменение ответа делает курсор устаревшим;
сначала проверяется принадлежность вопросу, затем отпечаток ответа.
Повтор того же курсора для того же ответа выдаёт ту же страницу и продолжение.

Владелец хранит один неизменный ключ, а не запросы и записи каждого курсора.
Число выданных курсоров, объём вопроса и прошедшее время не отзывают обещанное
продолжение. Ошибка случайности при создании новой цепочки курсоров не заменяет ключ
и не лишает уже выданные курсоры их владельца. После замены владельца старый
курсор неприменим; сохранение после перезапуска не обещается.

Это правило относится к поиску исходников, имён, провайдерных ролей и справки,
включая страницы открытого документа. Способ повторного чтения и полнота ответа
заданы в [правиле страниц](read-page-budget.md). Жизненный цикл снимка `view`
задан отдельно в [правиле курсора просмотра](view-cursor-binding.md).
