---
id: CTR.WIRE.DAEMON-INVOCATION-PROTOCOL
check:
  - crates/unica-coder/tests/daemon_receipt_ledger.rs::receipt_key_is_canonicalized_identically_by_client_and_server
  - crates/unica-coder/src/infrastructure/daemon/protocol_v5.rs::strict_decoded_submit_derives_the_frozen_application_receipt_key
  - crates/unica-coder/src/infrastructure/daemon/client_v5.rs::partial_v5_response_bytes_cannot_replenish_client_deadline
  - crates/unica-coder/src/infrastructure/daemon/client_v5.rs::malformed_v5_response_permanently_poisons_owner_session
  - crates/unica-coder/src/infrastructure/daemon/protocol_v5.rs::strict_v5_client_decoder_round_trips_every_closed_request_kind
  - crates/unica-coder/src/infrastructure/daemon/protocol_v5.rs::strict_v5_server_response_round_trips_the_cr0_invocation_algebra
  - crates/unica-coder/src/infrastructure/daemon/protocol_v5.rs::strict_v5_client_decoder_rejects_unknown_missing_cross_variant_and_invalid_values
  - crates/unica-coder/src/infrastructure/daemon/protocol_v5.rs::strict_v5_server_response_rejects_unknown_kinds_extra_fields_and_invalid_enums
  - crates/unica-coder/src/infrastructure/daemon/protocol_v5.rs::strict_v5_direct_receipt_rejects_a_terminal_digest_mismatch
  - crates/unica-coder/src/infrastructure/daemon/protocol_v5.rs::bounded_v5_reader_rejects_oversized_empty_and_unterminated_frames
  - crates/unica-coder/tests/daemon_receipt_ledger.rs::v5_rejects_v3_v4_and_strictly_round_trips_receipt_messages
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/tests.rs::handshake_keeps_its_small_bound_while_an_authenticated_frame_may_be_far_larger
  - crates/unica-coder/src/infrastructure/daemon/client_v5/request_write.rs::a_frame_above_the_request_bound_is_refused_with_both_sizes_before_writing
  - crates/unica-coder/src/interfaces/daemon_router.rs::live_daemon_runs_a_large_call_and_refuses_one_above_the_bound_unsent
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/tests.rs::a_frame_straddling_the_idle_session_poll_is_read_whole
  - crates/unica-coder/src/infrastructure/daemon/runtime_v5/tests.rs::a_session_frame_missing_its_deadline_closes_the_session
---

# Демон принимает строгие сообщения своей версии протокола

Протокол `unica-daemon-jsonl-5` отклоняет handshake версий 3 и 4
с кодом `protocol_mismatch`. Его сообщения разбираются по закрытым
вариантам: неизвестные и лишние поля, отсутствие обязательных полей,
несогласованные поля разных вариантов и недопустимые значения отвергаются.
Идентификаторы должны иметь каноническую форму; бюджет вызова и ожидание
задания ограничены диапазоном 0–7000 мс.

Вход состоит из непустых завершённых строк JSONL; предел считается вместе
с переводом строки. Первая строка соединения читается до проверки токена
и ограничена 16 КиБ: законный `hello` занимает сотни байтов, а число
соединений не ограничено. Строка аутентифицированной сессии ограничена
64 МиБ. Незавершённая строка и превышение предела чтения отвергаются.

Предел сессии временный, без подтверждённого основания
([#1309](https://github.com/IngvarConsulting/unica/issues/1309),
[#1312](https://github.com/IngvarConsulting/unica/issues/1312)). Прежние
16 КиБ отвергали обычный `unica.apply` с процедурой BSL в несколько экранов.
Новое число выбрано с большим запасом: крупнейший модуль выгрузки УТ
(7,6 МБ) занимает в JSON 7,9 МБ. Демон держит одновременно кадр, разобранные
аргументы и их хеш, поэтому запрос расходует память в несколько раз больше
своего размера; предел оставляет конечным расход одного запроса. Число
одновременных сессий не ограничено, и суммарный расход этот предел не
ограничивает. Frontend не отправляет вызов, кадр которого больше предела.
Ответ — отказ канонического вызова `provider_limit_exceeded` с исходом
`fixCall` в `diagnostics`; его текст называет предел демона и фактический
размер кадра в байтах, без содержимого аргументов. Квитанции у такого вызова
нет, и он не исполняется.

Прямая квитанция принимается
только с хешем, соответствующим её конечному результату. Ошибки протокола
передают закрытый код, без произвольного текста внутренних ошибок.

Frontend и демон одинаково вычисляют ключ квитанции из `invocationId`,
`reservedTaskId`, core identity, инструмента, хеша нормализованных аргументов
и хеша области запроса. Фиксированный независимый пример закрепляет расчёт.
Частичное поступление байтов не продлевает срок чтения. Срок кадра
сессии — 7 с, бюджет вызова frontend, — отсчитывается от его первого байта,
а не от начала ожидания: кадр, начатый в конце простоя, не теряет прочитанное
начало. Кадр, не дочитанный к своему сроку, закрывает сессию; его хвост
не разбирается как новый запрос. Некорректный ответ
закрывает соединение: следующая операция не пишет в него новый запрос.

Проверки исполняют настоящие кодеки. Интеграционная проверка запускает
демон и проверяет handshake и обмен; часть вариантов ответа подставляется
как контролируемый кадр для проверки декодера. Это не доказательство
каждого конечного состояния реальной предметной операции.
