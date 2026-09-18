---
id: CTR.WIRE.DAEMON-INVOCATION-PROTOCOL
---

# Остаток временных и размерных границ daemon protocol

`receipt_pending` допустим только для живой квитанции при recover,
не в первом ответе submit, и не открывает нового окна ожидания.

Request JSONL ограничен 16 KiB; один `DomainResult` — 8 MiB, Task record
и response JSONL — 8 MiB + 64 KiB. Direct и Task используют один предел
результата и `result_too_large`. Frontend применяет предел ответа независимо
от предела запроса. Oversized и truncated ответы закрывают owner session.

IPC serialization получает 125 мс сверх operation budget, без повторного
начала срока; внутренний safety cap ответа — 10 секунд. Номер версии
записи Task и признак отмены сохраняют durable значения при reconnect/restart.

Отмена до materialized Task должна идти через отдельную daemon session.
Наличие сообщения `CancelInvocation` само по себе не доказывает этот маршрут.
