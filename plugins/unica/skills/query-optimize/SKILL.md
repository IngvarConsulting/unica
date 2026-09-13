---
name: query-optimize
description: "Оптимизация запросов 1С и СКД. Используй когда нужно написать, проверить или ускорить запрос, СКД query, временные таблицы, виртуальные таблицы, отборы, соединения или проблемный SQL/DBMS trace."
---

# Query Optimize

## MCP routing

- Preferred path: use MCP `unica` tools `unica.code.search`, `unica.view`, `unica.code.graph`, `unica.code.diagnostics`, `unica.view` on the schema node, `unica.meta.info`, `unica.docs`, and `unica.runtime.execute`.
- По INV-MCP-RUNTIME-RECEIPT и ADR-0074: `unica.runtime.execute` с `dryRun: true`
показывает запланированную команду без побочных эффектов, а с `dryRun: false`
исполняет классифицированную операцию и отвечает её терминальным результатом в
том же вызове, приложив названную причину риска (`runtime_risk_*`)
предупреждением; неклассифицированная операция по-прежнему отказывает
`runtime_operation_unbounded` до обнаружения рабочего пространства. Preview
исполнением не является. Работу, которую вызов ждать не должен, запускай через
`unica.runtime.job.start`. Не обходи контракт прямым runner-ом или через
`unica.build.*`.
- Use `unica.view {}` if the source-set or format is unclear.
- Do not call internal analyzer, standards, runtime, or package adapters directly. They are hidden behind MCP `unica`.

## Workflow

1. Extract the exact query text with `unica.code.search` or `unica.view` on the schema node.
2. Inspect the execution context with `unica.view` on the module node (its `Method` branch lists the methods): module, exported entry point, region, temporary table chain, and caller loop.
3. Use `unica.code.graph` for callers/callees when the query is inside reusable API, background jobs, event handlers, or suspected query-in-loop flow.
4. Run `unica.code.diagnostics` with `action=findings`, the exact `sourceSet`, and the containing module's logical `metadataPath` when analyzer diagnostics can reveal unreachable code, unresolved calls, or type issues around the query. Do not pass a DCS `TemplatePath` as a diagnostic target; locate the BSL module that executes the query.
5. Inspect `unica.meta.info` for both related modules, subscriptions, roles, functional options and the local registers, dimensions, resources, реквизиты, tabular sections, and indexes implied by the platform object type.
6. Inspect DCS with `unica.view` on the schema node when the query lives in a data composition schema.
7. Search `unica.docs` with `source: "development-standard"` only for `development-standard` query rules. Exact platform query semantics require `unica.docs` with `source: "platform-help"` before a platform-dependent rewrite.
8. Read `../../references/platform/db-performance.md` when performance depends on DBMS behavior, locks, indexes, temp storage, WAL, TEMPDB, or large table statistics.
9. Optimize one cause at a time: filters before joins, virtual table parameters, temporary table materialization, repeated queries in loops, dot dereference expansion, unbounded selections, and unnecessary totals.
10. Use `unica.runtime.execute` only to preview typed syntax arguments; report actual syntax as unverified and require real trace/log evidence when performance depends on data volume.

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
