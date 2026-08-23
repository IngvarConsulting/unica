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
канонический вызов в одну daemon-owned Invocation. Frontend только отправляет
вызов, ждёт прямой ответ или принимает durable Task и проецирует результат;
повторный frontend execution и polling re-execution запрещены структурно.

Срок начинается при получении frontend-запроса. До 7000 мс завершившийся вызов
может ответить напрямую; в 7000 мс незавершённая работа уже записана как Task.
Нулевой переданный бюджет означает немедленную материализацию после дешёвой
валидации. Более ранний известный host deadline сокращает бюджет с запасом на
сериализацию. `KnownLong` выбирается подготовленной границей после валидации и
до дорогого ожидания или запуска, а не списком имён инструментов.

Daemon protocol v2 принимает только строгие `SubmitInvocation`, `GetTask`,
`WaitTask`, `CancelTask`. После дешёвой schema-проверки daemon связывает вызов с
opaque capability точного `WorkspaceActor`; handler больше не получает
`workspaceHint`, а чтение и публикация проходят через retained physical root и
revision fence этого actor. Реестр actor хранит weak entries, очищает умершие и
ограничивает одновременно живые capabilities числом 64, не вытесняя активные.

Durable record schema v2 хранит закрытые identity, digest, status и
`SafeFailureReason`, но не raw arguments, caller/runtime text, stdout или
stderr. Task сразу создаётся атомарно в `Working`. Неопределённость commit
разрешается identity-bound readback; terminal publication удерживает live owner
и actor capability до подтверждённого durable terminal без повторного domain
execution. На этом срезе зарегистрированных resume owners нет: любой v1/v2
working record после restart становится закрытой interrupted или
resume-unsupported failure, а v1 terminal детерминированно мигрирует в v2.
Неизвестная schema отклоняется.

Входной `SurfaceRelease::V12` с 71 инструментом остаётся на прежнем dispatch и
wire-контракте; опубликованный v0.12.3 baseline из 74 имён остаётся отдельной
миграционной приёмкой. Этот срез не отображает legacy registry в восемь
канонических identities, не включает SEP-2663 capability и не добавляет
task-tools; атомарное публичное переключение и удаление legacy path принадлежат
Task 22.

**Почему.** Так внутренний execution lifecycle становится единым до изменения
публичной поверхности, не сохраняя небезопасный legacy payload и не выдавая
частичную миграцию за отгруженную v0.13.

**Цена.** До Task 22 в одном бинаре сосуществуют неизменённый публичный V12
dispatch и непубликуемый canonical V13 router.
