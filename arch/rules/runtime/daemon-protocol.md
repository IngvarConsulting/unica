---
id: CTR.WIRE.DAEMON-INVOCATION-PROTOCOL
check:
  - crates/unica-coder/src/infrastructure/daemon/protocol_v5.rs::strict_v5_client_decoder_round_trips_every_closed_request_kind
  - crates/unica-coder/src/infrastructure/daemon/protocol_v5.rs::strict_v5_server_response_round_trips_the_cr0_invocation_algebra
  - crates/unica-coder/src/infrastructure/daemon/protocol_v5.rs::strict_v5_client_decoder_rejects_unknown_missing_cross_variant_and_invalid_values
  - crates/unica-coder/src/infrastructure/daemon/protocol_v5.rs::strict_v5_server_response_rejects_unknown_kinds_extra_fields_and_invalid_enums
  - crates/unica-coder/tests/daemon_receipt_ledger.rs::v5_rejects_v3_v4_and_strictly_round_trips_receipt_messages
---

# Демон принимает строгие сообщения своей версии протокола

Протокол `unica-daemon-jsonl-5` отклоняет handshake версий 3 и 4 с кодом
`protocol_mismatch`. Сообщения разбираются по закрытым вариантам:
неизвестные и лишние поля, отсутствие обязательных полей, несогласованные
поля разных вариантов и недопустимые значения отвергаются. Идентификаторы
имеют каноническую форму; бюджет вызова и ожидание задания находятся
в диапазоне 0–7000 мс. Ошибки протокола передают закрытый код без
произвольного текста внутренних ошибок.

Размер и чтение строки задаёт [граница кадра](daemon-request-framing.md),
а идентичность квитанции — [правило ключа](daemon-receipt-key.md).
Проверки исполняют кодеки; обмен с запущенным демоном проверяет версию
и сообщения, но не каждое конечное состояние предметной операции.
