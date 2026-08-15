---
name: background-jobs
description: "Фоновые и регламентные задания 1С. Используй когда нужно спроектировать, проверить или диагностировать background jobs, расписания, очереди, зависания, retry-логику и журналирование."
---

# Background Jobs

## MCP routing

- Preferred path: use MCP `unica` tools `unica.project.map`, `unica.code.search`, `unica.meta.info`, `unica.code.diagnostics`, `unica.standards.search`, `unica.standards.explain`, and `unica.runtime.execute`.
- По INV-MCP-RUNTIME-RECEIPT текущий runtime-контракт: `unica.runtime.execute` — preview-only и вызывается только с `dryRun: true`; любой applied-режим возвращает fail-closed до workspace discovery и process spawn. Preview не является runtime verification. Не обходи этот отказ прямым runner-ом, через `unica.build.*` или fallback через `unica.runtime.job.*`.
- Use `unica.role.info` when job behavior depends on user context or permissions.
- Do not call internal runtime, analyzer, standards, or package adapters directly. They are hidden behind MCP `unica`.

## References

- Read `../../references/platform/platform-mechanics.md` for background job context, temporary storage, and security boundaries.
- Read `../../references/platform/transactions-locks.md` when the job reads and then writes shared state; this skill keeps only job restartability.
- Read `../../references/platform/runtime-diagnostics.md` when the task includes ЖР/ТЖ, hangs, retries, or process/session evidence.

## Workflow

1. Identify job type: scheduled job, background job launched from code, queue worker, exchange worker, or deferred integration retry.
2. Find entry points with `unica.code.search`; inspect related metadata with `unica.meta.info` and project layout with `unica.project.map`.
3. Define execution contract: parameters, user context, transaction scope, idempotency key, lock strategy, timeout, retry count, and logging fields.
4. Check failure behavior before implementation: duplicate launch, partial write, stale lock, external service failure, session termination, and restart after crash.
5. Run `unica.code.diagnostics`; use `unica.runtime.execute` only to preview typed syntax/test/launch arguments, and record runtime verification as unavailable unless separate evidence is supplied.
6. For diagnosis, build a timeline from ЖР/ТЖ and map the first failure back to module code.

## Review checklist

- Job can be safely restarted or retried.
- Long work is split or checkpointed.
- Logs use structured logging fields: job id, parameters summary, correlation id,
  retry count, result, and sanitized error context, but no secrets.
- Shared state a job reads and then writes is handled per `transactions-locks`,
  and the lock does not block unrelated users or tenants.
- Failure paths distinguish retryable and permanent errors.

## Contract gaps

If public MCP `unica` cannot inspect schedules, active jobs, lock state, or runtime artifacts needed for the task, report a Unica MCP contract gap with the missing operation.
