---
id: CTR.WIRE.DAEMON-INVOCATION-PROTOCOL
status: active
governs: product
decision: DEC.2026-08-24.NATIVE-TASK-PROJECTION-SLICE
check: crates/unica-coder/src/infrastructure/daemon/mod.rs::invocation_protocol_round_trips_all_four_strict_requests_and_closed_responses
scope: [app, wire]
version: 3
producer: crates/unica-coder/src/infrastructure/daemon/protocol.rs
consumers: [host]
---

# Внутренний daemon protocol canonical Invocation

Protocol identity `unica-daemon-jsonl-3` является частью `CoreIdentity` и
разделяет discovery/state от любой иной wire ABI.

Версионированный JSONL protocol принимает строгие `SubmitInvocation`,
`GetTask`, `WaitTask`, `CancelTask`. Submit отвечает `Direct(DomainResult)` или
`Task(DaemonTaskSnapshot)`. Неизвестные поля, сообщения, неканонические TaskId и
wait больше 7000 мс отклоняются; ошибки транспорта используют закрытые коды.
Task snapshot несёт сохранённые `createdAt`/`updatedAt` epoch milliseconds,
`ttlMs` и `pollIntervalMs`: reconnect/restart не заменяет их временем чтения,
а `updatedAt` не меняется от чтения и не убывает при durable transition.
Исчерпание 64 одновременно живых workspace actor capabilities возвращает
закрытый retryable код `workspace_capacity`; poison actor registry возвращает
`workspace_registry_failed`. Исчерпание bounded Task retention возвращает
закрытый retryable код `task_capacity`. Неподтверждаемая durable publication
переводит daemon в `RestartRequested` и возвращает `durability_uncertain`, но
не staged DomainResult. Старый процесс перестаёт принимать соединения и
оставляет PID-bound endpoint до своей смерти; successor заменяет только stale
record. Текст внутренних ошибок не классифицируется и не попадает в protocol.

Request JSONL ограничен 16 KiB. Один canonical `DomainResult` ограничен 8 MiB;
Task record и response JSONL ограничены 8 MiB + 64 KiB bounded envelope. Direct
и Task применяют один result limit и закрытый код `result_too_large`. Frontend
читает response cap независимо от request cap; oversized, malformed или
truncated response закрывает owner session, повторное использование запрещено.
IPC serialization имеет 125 мс сверх переданного operation budget, но не
перезапускает этот deadline; внутренний safety cap ответа — 10 секунд.

Для `SubmitInvocation` daemon захватывает absolute response deadline executor
clock сразу после получения JSONL, до strict validation, workspace binding и
service preparation. Wire `responseBudgetMs` только сужает этот deadline;
ActorBound/Prepared/executor и writer получают одну capability, а не duration.
Если handoff истёк, Invocation уже Task. После истечения также response margin
writer закрывается без ответа, но durable Invocation не исполняется повторно.
Task 7 не обещает обнаружение TaskId, не доставленного после final deadline:
для этого нужен будущий protocol-level invocation/idempotency token.
