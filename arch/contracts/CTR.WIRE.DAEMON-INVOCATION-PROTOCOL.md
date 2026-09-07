---
id: CTR.WIRE.DAEMON-INVOCATION-PROTOCOL
status: active
governs: product
decision: DEC.2026-09-07.DAEMON-V5-PRODUCTION-CUTOVER
check:
  - crates/unica-coder/src/infrastructure/daemon/protocol_v5.rs::strict_v5_client_decoder_round_trips_every_closed_request_kind
  - crates/unica-coder/src/infrastructure/daemon/protocol_v5.rs::strict_v5_server_response_round_trips_the_cr0_invocation_algebra
  - crates/unica-coder/tests/daemon_receipt_ledger.rs::v5_rejects_v3_v4_and_strictly_round_trips_receipt_messages
scope: [app, wire]
version: 5
producer: crates/unica-coder/src/infrastructure/daemon/protocol_v5.rs
consumers: [host]
---

# Внутренний daemon protocol canonical Invocation

Protocol identity `unica-daemon-jsonl-5` является частью `CoreIdentity` и
разделяет discovery/state от любой иной wire ABI: процесс и состояние v5
(`daemon-p5-<digest>`) не делятся ни с одной предшествующей identity, и
`Hello` с версией 3 или 4 отклоняется закрытым `protocol_mismatch`.

Версионированный JSONL protocol принимает строгие `Hello`, `Ping`, `Release`,
`SubmitInvocation`, `GetTask`, `WaitTask`, `CancelTask`,
`RecoverInvocationReceipt`, `AcknowledgeInvocationReceipt` и
`CancelInvocation`. Submit несёт `invocationId`, `reservedTaskId`, инструмент,
аргументы, `workspaceHint` и `responseBudgetMs` в 0..=7000; daemon сам выводит
ключ квитанции из этих полей, digest core identity и нормализованного hash
аргументов, и ровно тот же ключ строит frontend. Submit отвечает
`direct` (квитанция с terminal, его digest и epoch), `task` (снимок) либо
`error`; `receipt_pending` возвращается только на recover живой квитанции и не
открывает нового бюджета. Direct-квитанция подтверждается
`AcknowledgeInvocationReceipt` с точными ключом и digest после того, как
frontend построил окончательное значение для хоста; неподтверждённая живёт час.
Terminal закрыт: `completed` с DomainResult, `failed` с одной из девяти
закрытых причин без текста, `cancelled`. Снимок Task несёт `taskId`,
`invocationId`, `receiptKeyDigest`, epoch времена, `ttlMs`, `pollIntervalMs`, номер
версии записи, `cancelRequested` и terminal-поля; reconnect/restart не заменяет их
временем чтения. Неизвестные поля, сообщения, неканонические идентификаторы и
wait больше 7000 мс отклоняются; ошибки используют восемнадцать закрытых кодов,
текст внутренних ошибок в protocol не попадает.

Request JSONL ограничен 16 KiB. Один canonical `DomainResult` ограничен 8 MiB;
Task record и response JSONL ограничены 8 MiB + 64 KiB bounded envelope. Direct
и Task применяют один result limit и закрытую причину `result_too_large`.
Frontend читает response cap независимо от request cap; oversized, malformed,
truncated или пришедший после cutoff response закрывает owner session.
IPC serialization имеет 125 мс сверх переданного operation budget и не
перезапускает этот deadline; внутренний safety cap ответа — 10 секунд.

Для `SubmitInvocation` daemon резервирует квитанцию до валидации, admission и
подготовки и захватывает absolute response deadline; wire `responseBudgetMs`
только сужает его. Если handoff истёк, Invocation уже Task с
`reservedTaskId`. Потерянный ответ восстанавливается по ключу без повторной
отправки: `RecoverInvocationReceipt` читает durable state и никогда не
исполняет вызов снова.
