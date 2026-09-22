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
---

# Демон принимает строгие сообщения своей версии протокола

Протокол `unica-daemon-jsonl-5` отклоняет handshake версий 3 и 4
с кодом `protocol_mismatch`. Его сообщения разбираются по закрытым
вариантам: неизвестные и лишние поля, отсутствие обязательных полей,
несогласованные поля разных вариантов и недопустимые значения отвергаются.
Идентификаторы должны иметь каноническую форму; бюджет вызова и ожидание
задания ограничены диапазоном 0–7000 мс.

Вход состоит из непустой завершённой строки JSONL размером не более
16 КиБ вместе с переводом строки. Незавершённая строка и превышение
предела чтения отвергаются. Прямая квитанция принимается
только с хешем, соответствующим её конечному результату. Ошибки протокола
передают закрытый код, без произвольного текста внутренних ошибок.

Frontend и демон одинаково вычисляют ключ квитанции из `invocationId`,
`reservedTaskId`, core identity, инструмента, хеша нормализованных аргументов
и хеша области запроса. Фиксированный независимый пример закрепляет расчёт.
Частичное поступление байтов не продлевает срок чтения. Некорректный ответ
закрывает соединение: следующая операция не пишет в него новый запрос.

Проверки исполняют настоящие кодеки. Интеграционная проверка запускает
демон и проверяет handshake и обмен; часть вариантов ответа подставляется
как контролируемый кадр для проверки декодера. Это не доказательство
каждого конечного состояния реальной предметной операции.
