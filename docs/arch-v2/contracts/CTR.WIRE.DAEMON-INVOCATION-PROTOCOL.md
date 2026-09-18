---
id: CTR.WIRE.DAEMON-INVOCATION-PROTOCOL
---

# Остаток идентичности и ограничений daemon protocol

Ключ квитанции одинаково вычисляется frontend и демоном из точных
`invocationId`, `reservedTaskId`, инструмента, аргументов, `workspaceHint`
и core identity. Переданный caller digest не заменяет самостоятельный
расчёт демона. `receipt_pending` допустим только для живой квитанции
при recover, не в первом ответе submit; новое окно ожидания он не открывает.

Request JSONL ограничен 16 KiB; один `DomainResult` — 8 MiB, Task record
и response JSONL — 8 MiB + 64 KiB. Direct и Task используют один предел
результата и `result_too_large`. Frontend применяет предел ответа независимо
от предела запроса. Oversized, malformed, truncated и поздний ответ
закрывают owner session.

IPC serialization получает 125 мс сверх operation budget, без повторного
начала срока; внутренний safety cap ответа — 10 секунд. Поля Task,
включая запись версии и признак отмены, сохраняют durable значения при
reconnect/restart. Проверки строгого формата не доказывают все эти условия.
