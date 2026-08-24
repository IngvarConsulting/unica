---
id: CTR.WIRE.DAEMON-INVOCATION-PROTOCOL
status: active
governs: product
decision: DEC.2026-08-24.DAEMON-INVOCATION-ROUTING-SLICE
check: crates/unica-coder/src/infrastructure/daemon/mod.rs::invocation_protocol_round_trips_all_four_strict_requests_and_closed_responses
scope: [app, wire]
version: 2
producer: crates/unica-coder/src/infrastructure/daemon/protocol.rs
consumers: [host]
---

# Внутренний daemon protocol canonical Invocation

Версионированный JSONL protocol принимает строгие `SubmitInvocation`,
`GetTask`, `WaitTask`, `CancelTask`. Submit отвечает `Direct(DomainResult)` или
`Task(DaemonTaskSnapshot)`. Неизвестные поля, сообщения, неканонические TaskId и
wait больше 7000 мс отклоняются; ошибки транспорта используют закрытые коды.
Исчерпание 64 одновременно живых workspace actor capabilities возвращает
закрытый retryable код `workspace_capacity`; poison actor registry возвращает
`workspace_registry_failed`. Неподтверждаемая durable publication переводит
daemon в controlled restart и возвращает `durability_uncertain`, но не staged
DomainResult. Текст внутренних ошибок не классифицируется и не попадает в
protocol.
