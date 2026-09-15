---
name: query-optimize
description: "Оптимизация запросов 1С и СКД. Используй когда нужно написать, проверить или ускорить запрос, СКД query, временные таблицы, виртуальные таблицы, отборы, соединения или проблемный SQL/DBMS trace."
---

# Query Optimize

## MCP routing

- Preferred path: use MCP `unica` tools `unica.search`, `unica.view`, `unica.check`, `unica.view` on the schema node, `unica.view` on the object node, `unica.docs`, and `unica.run`.
- Runtime идёт через `unica.run`: вызов без `op` отдаёт словарь операций и
контракт каждой — `argsSchema`, `execution`, `previewRequired`,
`ifRevRequiredOnApply`. Контракт вызова бери оттуда, а не из этого текста;
выбирай только операцию с `implemented: true` и не выдумывай аргументов
записи с `argsSchema: null`; превью исполнением не является. Не обходи
контракт прямым runner-ом.
- Use `unica.view {}` if the source-set or format is unclear.
- Do not call internal analyzer, standards, runtime, or package adapters directly. They are hidden behind MCP `unica`.

## Workflow

1. Extract the exact query text with `unica.search` or `unica.view` on the schema node.
2. Inspect the execution context with `unica.view` on the module node (its `Method` branch lists the methods): module, exported entry point, region, temporary table chain, and caller loop.
3. Find callers with `unica.search` by the method name when the query is inside reusable API, background jobs, event handlers, or suspected query-in-loop flow; a call graph is not on the v0.13 surface.
4. Run `unica.check {at}` on the containing module when analyzer diagnostics can reveal unreachable code, unresolved calls, or type issues around the query. Do not pass a DCS `TemplatePath` as a diagnostic target; locate the BSL module that executes the query.
5. Inspect `unica.view` on the object node for both related modules, subscriptions, roles, functional options and the local registers, dimensions, resources, реквизиты, tabular sections, and indexes implied by the platform object type.
6. Inspect DCS with `unica.view` on the schema node when the query lives in a data composition schema.
7. Search `unica.docs` with `source: "development-standard"` only for `development-standard` query rules. Exact platform query semantics require `unica.docs` with `source: "platform-help"` before a platform-dependent rewrite.
8. Read `../../references/platform/db-performance.md` when performance depends on DBMS behavior, locks, indexes, temp storage, WAL, TEMPDB, or large table statistics.
9. Optimize one cause at a time: filters before joins, virtual table parameters, temporary table materialization, repeated queries in loops, dot dereference expansion, unbounded selections, and unnecessary totals.
10. Check syntax with `unica.check`; require real trace/log evidence when performance depends on data volume.

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
    "name": "unica.view",
    "arguments": {
      "cwd": "<workspace>",
      "at": "main:Report.Продажи.Template.ОсновнаяСхемаКомпоновкиДанных.DataSet"
    }
  }
}
```

```jsonc
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.docs",
    "arguments": {
      "query": "оптимизация запросов 1С виртуальные таблицы",
      "source": "development-standard"
    }
  }
}
```
