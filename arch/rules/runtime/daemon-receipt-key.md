---
id: INV.WIRE.DAEMON-RECEIPT-KEY
check:
  - crates/unica-coder/tests/daemon_receipt_ledger.rs::receipt_key_is_canonicalized_identically_by_client_and_server
  - crates/unica-coder/src/infrastructure/daemon/protocol_v5.rs::strict_decoded_submit_derives_the_frozen_application_receipt_key
  - crates/unica-coder/src/infrastructure/daemon/protocol_v5.rs::strict_v5_direct_receipt_rejects_a_terminal_digest_mismatch
---

# Frontend и демон одинаково определяют квитанцию вызова

Frontend и демон вычисляют один ключ квитанции из `invocationId`,
`reservedTaskId`, идентичности ядра, инструмента, хеша нормализованных
аргументов и хеша области запроса. Один и тот же строгий запрос не получает
разную идентичность по разные стороны соединения. Фиксированный независимый
пример в `check` закрепляет расчёт.

Прямая квитанция принимается только с хешем, соответствующим её конечному
результату. Несовпадение не выдаётся за подтверждённый результат вызова.
