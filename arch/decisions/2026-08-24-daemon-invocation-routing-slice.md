---
id: DEC.2026-08-24.DAEMON-INVOCATION-ROUTING-SLICE
status: active
governs: product
realized: crates/unica-coder/src/infrastructure/daemon/mod.rs::daemon_executes_one_canonical_invocation_and_poll_cancel_never_relaunches_it
supersedes: []
superseded-by: null
establishes: [INV.APP.DAEMON-INVOCATION-OWNERSHIP, INV.APP.DAEMON-INVOCATION-HANDOFF, INV.APP.DAEMON-TASK-PERSISTENCE, INV.APP.DAEMON-TASK-RECOVERY, INV.APP.DAEMON-ACTOR-AUTHORITY, INV.APP.DAEMON-TERMINAL-RECONCILIATION, INV.APP.DAEMON-ACTOR-CAPACITY, INV.WIRE.SURFACE-RELEASE-ROUTING, CTR.WIRE.DAEMON-INVOCATION-PROTOCOL]
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

Durable record schema v2 хранит закрытые identity, digest, status и
`SafeFailureReason`, не raw arguments/runtime text/stdout/stderr. До create
выделяются TaskId и live owner; execution начинается лишь после точного
`Working`. Uncertain commit разрешается identity-bound readback по bounded
monotonic policy; terminal удерживает owner/capability без re-execution.
Исчерпание policy закрывает submit, скрывает staged result, освобождает actor и
завершает daemon. Resume owners пока нет: после restart v1/v2 `Working`
закрывается как interrupted/unsupported, v1 terminal мигрирует, unknown schema
отклоняется.

Входной `SurfaceRelease::V12` с 71 инструментом остаётся на прежнем dispatch и
wire-контракте; опубликованный v0.12.3 baseline из 74 имён остаётся отдельной
миграционной приёмкой. Здесь нет legacy-to-canonical mapping, SEP-2663 или
task-tools; публичное переключение и удаление legacy path принадлежат Task 22.

**Почему.** Так внутренний execution lifecycle становится единым до изменения
публичной поверхности, не сохраняя небезопасный legacy payload и не выдавая
частичную миграцию за отгруженную v0.13.

**Цена.** До Task 22 сосуществуют публичный V12 dispatch и скрытый V13 router.
