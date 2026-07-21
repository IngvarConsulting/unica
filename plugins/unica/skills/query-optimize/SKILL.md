---
name: query-optimize
description: "Оптимизация запросов 1С и СКД. Используй когда нужно написать, проверить или ускорить запрос, СКД query, временные таблицы, виртуальные таблицы, отборы, соединения или проблемный SQL/DBMS trace."
---

# Query Optimize

## MCP routing

- Preferred path: use MCP `unica` tools `unica.code.search`, `unica.code.outline`, `unica.code.grep`, `unica.code.graph`, `unica.code.diagnostics`, `unica.dcs.info`, `unica.meta.info`, `unica.meta.profile`, `unica.standards.search`, `unica.standards.explain`, and `unica.runtime.execute`.
- Use `unica.project.map` if the source-set or format is unclear.
- Do not call internal analyzer, standards, runtime, or package adapters directly. They are hidden behind MCP `unica`.

## Workflow

1. Extract the exact query text with `unica.code.grep`, `unica.code.search`, or `unica.dcs.info`.
2. Inspect the execution context with `unica.code.outline`: module, exported entry point, region, temporary table chain, and caller loop.
3. Use `unica.code.graph` for callers/callees when the query is inside reusable API, background jobs, event handlers, or suspected query-in-loop flow.
4. Run `unica.code.diagnostics` with `mode=file` when analyzer diagnostics can reveal unreachable code, unresolved calls, or type issues around the query.
5. Inspect metadata with `unica.meta.profile` when the query is tied to an exact object and you need object modules, subscriptions, roles, functional options, or predefined items. Use `unica.meta.info` for XML-level registers, dimensions, resources, реквизиты, tabular sections, and indexes implied by the platform object type.
6. Inspect DCS with `unica.dcs.info` when the query lives in a data composition schema.
7. Search standards with `unica.standards.search` for platform-specific query rules before making a risky rewrite.
8. Read `references/platform/db-performance.md` when performance depends on DBMS behavior, locks, indexes, temp storage, WAL, TEMPDB, or large table statistics.
9. Optimize one cause at a time: filters before joins, virtual table parameters, temporary table materialization, repeated queries in loops, dot dereference expansion, unbounded selections, and unnecessary totals.
10. Verify syntax with `unica.runtime.execute` and ask for real trace/log evidence when performance depends on data volume.

## DB-aware diagnostics

- Keep platform query text, generated SQL/DBMS evidence, table sizes, index usage, locks, deadlocks, and transaction boundaries together.
- Treat PostgreSQL, MS SQL Server, and file mode as different evidence models. Do not generalize a СУБД-specific conclusion without naming it.
- Do not recommend a new index without tying it to a predicate, join, sort, grouping, and write-cost tradeoff.
- For virtual tables, prefer precise parameters over broad reads followed by post-filtering.
- For блокировки, connect lock holder, waiter, transaction, module path, and user/API scenario before proposing a rewrite.

## Review checklist

- Virtual tables receive parameters instead of broad post-filtering.
- Temporary tables have the minimal fields needed by later stages.
- Repeated subqueries and query-in-loop patterns are removed or justified.
- Joins do not multiply rows silently; totals and grouping match business meaning.
- Date and organization filters are applied as early as the platform query allows.
- Query changes preserve rights semantics and do not replace `РАЗРЕШЕННЫЕ` blindly.

## MCP examples

```jsonc
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.dcs.info",
    "arguments": {
      "cwd": "<workspace>",
      "TemplatePath": "Reports/Продажи/Ext/Report/DataCompositionSchema.xml",
      "Mode": "query"
    }
  }
}
```

```jsonc
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.standards.search",
    "arguments": {
      "query": "оптимизация запросов 1С виртуальные таблицы",
      "limit": 5
    }
  }
}
```
