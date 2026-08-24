---
id: DEC.2026-08-24.DAEMON-INVOCATION-ROUTING-SLICE
status: active
governs: product
realized: crates/unica-coder/src/infrastructure/daemon/mod.rs::daemon_executes_one_canonical_invocation_and_poll_cancel_never_relaunches_it
supersedes: []
superseded-by: null
establishes: [INV.APP.DAEMON-INVOCATION-OWNERSHIP, INV.APP.DAEMON-INVOCATION-HANDOFF, INV.APP.DAEMON-TASK-PERSISTENCE, INV.APP.DAEMON-TASK-RECOVERY, INV.APP.DAEMON-ACTOR-AUTHORITY, INV.APP.DAEMON-TERMINAL-RECONCILIATION, INV.APP.DAEMON-ACTOR-CAPACITY, INV.APP.DAEMON-STORE-FAIL-STOP, INV.WIRE.SURFACE-RELEASE-ROUTING, CTR.WIRE.DAEMON-INVOCATION-PROTOCOL]
design: docs/design/2026-08-23-v0-13-execution-surface-design.md
---

# Скрытый canonical v0.13 profile исполняется только в daemon

**Решение.** Явно выбранный внутренний `SurfaceRelease::V13` превращает каждый
вызов в одну daemon-owned Invocation. Frontend только отправляет, ждёт или
проецирует durable Task; повторный execution запрещён структурно.

Срок начинается при frontend receipt: до 7000 мс допустим direct, в 7000 мс
незавершённая работа уже durable Task. Нулевой или более ранний host budget
ускоряет handoff с запасом сериализации. `KnownLong` определяется подготовленной
границей после валидации и до дорогого ожидания, а не именем инструмента.

Строгий daemon protocol v2 принимает `SubmitInvocation`, `GetTask`, `WaitTask`,
`CancelTask`. После schema-проверки вызов получает opaque capability точного
`WorkspaceActor`; handler не видит `workspaceHint`, чтение и публикация проходят
через retained root и revision fence. Weak registry допускает 64 живых actor,
не вытесняет активные; capacity retryable, poison — закрытая внутренняя ошибка.

Durable record schema v2 хранит закрытые identity, digest, status и `SafeFailureReason`,
не raw arguments/runtime text/stdout/stderr. До create
выделяются TaskId и live owner; любой успешный create проверяется по точному
TaskId и полной ожидаемой форме `Working`, execution начинается лишь после этой
проверки. Все обращения executor к sole-writer store идут через один bounded
serial store actor. File store использует атомарный create без замены, bounded
record и recovery, фиксированную retention capacity с удалением только
истёкших terminal records. Uncertain commit разрешается identity-bound readback
по bounded monotonic policy; terminal удерживает owner/capability без
re-execution. Исчерпание policy закрывает submit, скрывает staged result и
переводит процесс в `RestartRequested`. Только смерть старого PID освобождает
зависший syscall/non-cooperative execution и разрешает successor заменить
оставленный PID-bound endpoint. Resume owners пока нет: после restart v1/v2
`Working` закрывается как interrupted/unsupported; unknown schema отклоняется.

Входной `SurfaceRelease::V12` с 71 инструментом остаётся на прежнем dispatch и
wire-контракте; v0.12.3 baseline из 74 имён — отдельная приёмка. Здесь нет
legacy mapping/SEP-2663/task-tools; публичное переключение принадлежит Task 22.

**Почему.** Так execution lifecycle становится единым до изменения поверхности,
не сохраняя legacy payload и не выдавая частичную миграцию за v0.13.
**Цена.** До Task 22 сосуществуют публичный V12 dispatch и скрытый V13 router.
