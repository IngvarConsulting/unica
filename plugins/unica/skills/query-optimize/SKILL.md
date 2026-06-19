---
name: query-optimize
description: "Оптимизация запросов 1С и СКД. Используй когда нужно написать, проверить или ускорить запрос, СКД query, временные таблицы, виртуальные таблицы, отборы, соединения или проблемный SQL/DBMS trace."
---

# Query Optimize

## MCP routing

- Preferred path: use MCP `unica` tools `unica.code.search`, `unica.code.outline`, `unica.code.grep`, `unica.skd.info`, `unica.meta.info`, `unica.standards.search`, `unica.standards.explain`, and `unica.runtime.execute`.
- Use `unica.project.map` if the source-set or format is unclear.
- Do not call internal analyzer, standards, runtime, or package adapters directly. They are hidden behind MCP `unica`.

## Workflow

1. Extract the exact query text with `unica.code.grep`, `unica.code.search`, or `unica.skd.info`.
2. Inspect the execution context with `unica.code.outline`: module, exported entry point, region, temporary table chain, and caller loop.
3. Inspect metadata with `unica.meta.info` for registers, dimensions, resources, реквизиты, tabular sections, and indexes implied by the platform object type.
4. Inspect SKD with `unica.skd.info` when the query lives in a data composition schema.
5. Search standards with `unica.standards.search` for platform-specific query rules before making a risky rewrite.
6. Read `references/platform/db-performance.md` when performance depends on DBMS behavior, locks, indexes, temp storage, WAL, TEMPDB, or large table statistics.
7. Optimize one cause at a time: filters before joins, virtual table parameters, temporary table materialization, repeated queries in loops, dot dereference expansion, unbounded selections, and unnecessary totals.
8. Verify syntax with `unica.runtime.execute` and ask for real trace/log evidence when performance depends on data volume.

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
    "name": "unica.skd.info",
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
