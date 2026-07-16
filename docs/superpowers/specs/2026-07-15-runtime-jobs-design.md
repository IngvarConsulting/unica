# Durable runtime jobs — design

## Goal

Добавить durable lifecycle для долгих typed-операций `v8-runner`, не меняя
синхронный публичный контракт `unica.runtime.execute`.

## Context and decision

`unica.runtime.execute` удерживает дочерний процесс в MCP server process. При
закрытии stdin этот server завершается, поэтому thread внутри него не сохраняет
наблюдение за процессом. Выбран отдельный worker-процесс самого `unica`:
`unica --runtime-job-worker`. Он продолжает работу независимо от жизненного
цикла одного `tools/call` и обновляет durable record.

Отклонённые варианты:

1. Поток в MCP server — проще, но пропадает при закрытии stdio.
2. Изменить `runtime.execute` на async — ломает его публичную синхронную
   семантику и существующих клиентов.

## Public contract

Новые typed MCP tools:

- `unica.runtime.job.start` — принимает те же typed runtime arguments, что
  `unica.runtime.execute`, быстро возвращает `job` с `jobId`, operation,
  `startedAt` и safe target.
- `unica.runtime.job.status` — принимает `jobId`, возвращает snapshot.
- `unica.runtime.job.wait` — принимает `jobId` и caller-side
  `timeoutSeconds` (1..60); истечение ожидания не изменяет job.
- `unica.runtime.job.logs` — принимает `jobId` и optional `tailChars`;
  возвращает redacted stdout/stderr tails и пути логов.
- `unica.runtime.job.cancel` — принимает `jobId`; безопасно отменяет только
  safe operations, иначе возвращает `cancelDeferred` и `unsafePhase`.
- `unica.runtime.job.list` — возвращает snapshots workspace jobs.

Ответ `OperationResult` получает optional typed `job` JSON. Job tools не
принимают raw argv. `runtime.execute` сохраняет нынешний schema и результат.

## Persistence and state machine

Данные расположены только в `<cacheRoot>/jobs/<jobId>/`:

- `record.json` — atomically-replaced schema-versioned snapshot;
- `stdout.log` и `stderr.log` — bounded, redacted output;
- `cancel.json` — запрос отмены без секрета;
- `<cacheRoot>/jobs/active.lock` — atomic one-active-job claim per workspace.

`record.json` никогда не содержит actual argv или connection string. Родитель
посылает actual program/argv/cwd worker-у через его stdin после spawn; record
содержит только redacted argv and safe target.

Phase is an exhaustive enum: `queued`, `running`, `cancelRequested`,
`succeeded`, `failed`, `cancelled`, `timedOut`, `lost`. Terminal phases cannot
transition again. Worker refreshes `updatedAt` heartbeat. A stale active
record is atomically transitioned to `lost`; it is never automatically
restarted.

## Execution, cancellation and recovery

The worker starts `v8-runner` without a new wrapper deadline; runner
`execution_timeout` remains authoritative. It drains both output pipes in
parallel to avoid a full-pipe deadlock, retains bounded output, redacts before
writing logs, and publishes exactly one terminal exit result.

Cancellation policy is a typed enum, not a boolean:

- safe: `make`, `syntax`, `test`, `tools-download`; worker kills the direct
  child and records `cancelled`.
- critical: `config-init`, `init`, `build`, `dump`, `convert`, `load`;
  record becomes `cancelRequested` with `cancelDeferred=true` and
  `unsafePhase`, while worker keeps observing. `launch` is also deferred
  because Unica cannot safely own its process tree.

Second start sees a fresh active lock/record and returns a conflict with the
first `jobId`. A new MCP server reads the same record. If the worker continues,
status stays current; if heartbeat becomes stale, recovery records `lost`.

## Diagnostics, cache and errors

Terminal snapshot carries exit code, timeout/cancel reason, redacted argv,
stdout/stderr tails, log paths and known `output` artifact path. Corrupt or
unknown-schema record returns a controlled error without deleting a fresh
lock. No `unwrap` or non-exhaustive phase match is allowed in production.

Starting a job does not invalidate caches. After a successful terminal runtime
operation worker applies the existing runtime event mapping and workspace
cache/service invalidation. Cache update failure becomes a redacted warning,
not a false job failure.

## Tests

TDD uses an injected fake worker runner for long success, long failure,
reconnect via a new service instance, caller wait timeout, safe cancellation,
critical deferred cancellation, active-job conflict, stale-worker `lost`,
terminal diagnostics and secret redaction across output chunks. Contract tests
assert typed schemas, unknown-argument rejection and unchanged
`runtime.execute`. A disposable File IB public-MCP smoke test validates start,
poll/reconnect and no user infobase mutation.
