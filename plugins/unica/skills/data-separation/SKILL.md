---
name: data-separation
description: "Разделение данных 1С. Используй когда нужно проверить tenant-boundaries, разделители, безопасные запросы, RLS/права, фоновые задания, обмены или интеграции в разделенной базе."
---

# Data Separation

## MCP routing

- Preferred path: use MCP `unica` tools `unica.project.map`, `unica.code.search`, `unica.meta.info`, `unica.role.info`, `unica.code.diagnostics`, `unica.standards.search`, `unica.standards.explain`, and `unica.runtime.execute`.
- По INV-MCP-RUNTIME-RECEIPT и ADR-0074: `unica.runtime.execute` с `dryRun: true`
показывает запланированную команду без побочных эффектов, а с `dryRun: false`
исполняет классифицированную операцию и отвечает её терминальным результатом в
том же вызове, приложив названную причину риска (`runtime_risk_*`)
предупреждением; неклассифицированная операция по-прежнему отказывает
`runtime_operation_unbounded` до обнаружения рабочего пространства. Preview
исполнением не является. Работу, которую вызов ждать не должен, запускай через
`unica.runtime.job.start`. Не обходи контракт прямым runner-ом или через
`unica.build.*`.
- Use `unica.dcs.info` when reports/DCS queries may bypass tenant filters.
- Do not call internal metadata, analyzer, standards, runtime, or package adapters directly. They are hidden behind MCP `unica`.

## References

- Read `../../references/platform/platform-mechanics.md` for tenant-boundaries, rights, temporary storage, background jobs, and exchange behavior.
- Read `../../references/platform/db-performance.md` when separated data changes query plans, indexes, locks, or virtual table filters.
- Read `../../references/platform/runtime-diagnostics.md` when the issue appears only in ЖР/ТЖ or runtime traces.

## Workflow

1. Identify separation model: separator values, tenant ownership, user/session context, rights/RLS, privileged code, and external ids.
2. Inspect metadata and roles with `unica.meta.info` and `unica.role.info`; find risky code with `unica.code.search`.
3. Trace tenant value through reads, writes, reports, background jobs, exchange messages, file batches, temp storage, and integration calls.
4. Review queries for missing tenant filters, unsafe privileged mode, broad virtual tables, and joins that cross boundaries.
5. Use `unica.runtime.execute` to preview typed syntax/test arguments and, with `dryRun: false`, to run them, treating the result as evidence only for what the check covers; tenant isolation still requires separate evidence covering at least two tenant contexts.

## Red flags

- Code writes objects without setting separator attributes.
- Background job runs under a broad user context and processes all tenants.
- Exchange or integration payload omits tenant/external owner id.
- Report/DCS query uses privileged access without a documented reason.
- Temporary storage or files are shared across tenant/session boundaries.

## Contract gaps

If public MCP `unica` cannot expose separation metadata, role details, or runtime context required for the task, report a Unica MCP contract gap.
