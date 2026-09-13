---
name: db-performance
description: "Производительность БД и запросов 1С. Используй когда нужно диагностировать slow query, SQL/DBMS trace, индексы, блокировки, deadlock, TEMPDB/WAL, размеры таблиц или СКД на больших данных."
---

# DB Performance

## MCP routing

- Preferred path: use MCP `unica` tools `unica.view {}`, `unica.code.search`, `unica.code.graph`, `unica.meta.info`, `unica.view` on the schema node, `unica.code.diagnostics`, `unica.docs`, and `unica.runtime.execute`.
- По INV-MCP-RUNTIME-RECEIPT и ADR-0074: `unica.runtime.execute` с `dryRun: true`
показывает запланированную команду без побочных эффектов, а с `dryRun: false`
исполняет классифицированную операцию и отвечает её терминальным результатом в
том же вызове, приложив названную причину риска (`runtime_risk_*`)
предупреждением; неклассифицированная операция по-прежнему отказывает
`runtime_operation_unbounded` до обнаружения рабочего пространства. Preview
исполнением не является. Работу, которую вызов ждать не должен, запускай через
`unica.runtime.job.start`. Не обходи контракт прямым runner-ом или через
`unica.build.*`.
- Use `unica.view` on the role node when performance behavior depends on rights filters, RLS, or tenant boundaries.
- Do not call internal analyzer, runtime, standards, or package adapters directly. They are hidden behind MCP `unica`.

## References

- Read `../../references/platform/db-performance.md` for DB-aware workflow, indexes, virtual tables, locks, and DBMS evidence.
- Read `../../references/platform/runtime-diagnostics.md` when performance evidence comes from ЖР/ТЖ, process ids, sessions, or runtime timeline.
- Read `../../references/platform/transactions-locks.md` once evidence points at contention; this skill keeps the evidence side, that document owns the lock and transaction rules.

## Workflow

1. Name the slow scenario first: user action, API call, report, background job, exchange step, or posting.
2. Extract exact query/DCS text with `unica.code.search` or `unica.view` on the schema node; inspect large candidate modules with `unica.view` on the module node (its `Method` branch lists the methods) before reading full bodies.
3. Use `unica.code.graph` for callers/callees when the performance issue depends on execution path, query-in-loop risk, or impact of moving logic.
4. Inspect `unica.meta.info` for both the local object structure and related modules, roles, subscriptions, functional options, or predefined items that can change the performance path.
5. Gather evidence: row counts, generated SQL, query plan, managed locks, lock order, lock/deadlock participants, long transaction boundaries, temp storage, TEMPDB or WAL pressure, and table/index names.
6. Separate causes: inefficient platform query, missing or harmful index, broad virtual table read, query-in-loop, lock contention, DBMS maintenance, or data growth.
7. Propose one measurable change at a time; use `unica.runtime.execute` to preview typed syntax/test arguments and, with `dryRun: false`, to run them, and require separate runtime plus timing/DBMS evidence before calling the change verified.

## Stop rules

- Do not recommend indexes without a concrete predicate, join, sort, grouping, and write-cost tradeoff.
- Do not remove rights filters, tenant filters, or `РАЗРЕШЕННЫЕ` for performance without a named security decision.
- Do not claim a DBMS root cause without DBMS evidence. State missing evidence or Unica MCP contract gap.

## Output

- Scenario and evidence summary.
- Root cause ranked by likelihood.
- Minimal query/metadata/code change.
- Verification command or measurement.
- Residual risk for data volume, locks, or DBMS-specific behavior.
