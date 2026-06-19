---
name: db-performance
description: "Производительность БД и запросов 1С. Используй когда нужно диагностировать slow query, SQL/DBMS trace, индексы, блокировки, deadlock, TEMPDB/WAL, размеры таблиц или СКД на больших данных."
---

# DB Performance

## MCP routing

- Preferred path: use MCP `unica` tools `unica.project.map`, `unica.code.search`, `unica.code.outline`, `unica.code.grep`, `unica.meta.info`, `unica.skd.info`, `unica.code.diagnostics`, `unica.standards.search`, `unica.standards.explain`, and `unica.runtime.execute`.
- Use `unica.role.info` when performance behavior depends on rights filters, RLS, or tenant boundaries.
- Do not call internal analyzer, runtime, standards, or package adapters directly. They are hidden behind MCP `unica`.

## References

- Read `references/platform/db-performance.md` for DB-aware workflow, indexes, virtual tables, locks, and DBMS evidence.
- Read `references/platform/runtime-diagnostics.md` when performance evidence comes from ЖР/ТЖ, process ids, sessions, or runtime timeline.

## Workflow

1. Name the slow scenario first: user action, API call, report, background job, exchange step, or posting.
2. Extract exact query/SKD text with `unica.code.grep`, `unica.code.search`, or `unica.skd.info`; inspect large candidate modules with `unica.code.outline`.
3. Inspect metadata with `unica.meta.info`.
4. Gather evidence: row counts, generated SQL, query plan, lock/deadlock participants, long transaction boundaries, temp storage, TEMPDB or WAL pressure, and table/index names.
5. Separate causes: inefficient platform query, missing or harmful index, broad virtual table read, query-in-loop, lock contention, DBMS maintenance, or data growth.
6. Propose one measurable change at a time and verify with `unica.runtime.execute` for syntax/tests plus the same timing/DBMS evidence after the change.

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
