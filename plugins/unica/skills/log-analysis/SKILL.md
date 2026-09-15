---
name: log-analysis
description: "Анализ журнала регистрации и технологического журнала 1С. Используй когда нужно разобрать ЖР, ТЖ, исключения, блокировки, SQL/DBMSSQL, deadlock, long call, фоновые задания, HTTP-сервис или связать записи журнала с кодом."
---

# Log Analysis

## MCP routing

- Preferred path: use MCP `unica` tools `unica.search`, `unica.view` on the object node, `unica.view {}`, `unica.check`, and `unica.docs`.
- Runtime идёт через `unica.run`: вызов без `op` отдаёт словарь операций и
контракт каждой — `argsSchema`, `execution`, `previewRequired`,
`ifRevRequiredOnApply`. Контракт вызова бери оттуда, а не из этого текста;
выбирай только операцию с `implemented: true` и не выдумывай аргументов
записи с `argsSchema: null`; превью исполнением не является. Не обходи
контракт прямым runner-ом.
- Check syntax with `unica.check` and launch a client through `unica.run` (`client.run`); test runs are outside the v0.13 surface, and neither call is verification or a substitute for log evidence.
- Do not call internal runtime, analyzer, standards, or package adapters directly. They are hidden behind MCP `unica`.

## Inputs

Accept explicit journal registration exports, technological log files, copied log fragments, or paths provided by the user. Preserve timestamps, process/session ids, users, infobase, event kind, module/procedure, transaction id, SQL text, and correlation ids.

## References

- Read `../../references/platform/runtime-diagnostics.md` for ЖР/ТЖ timeline, startup, web-client, HTTP, background job, and process/session evidence.
- Read `../../references/platform/db-performance.md` when log fragments contain SQL, locks, deadlocks, waits, long queries, or DBMS-specific artifacts.

## Workflow

1. Classify the evidence: ЖР event, ТЖ event, platform exception, DBMS/SQL, lock/deadlock, long call, background job, HTTP service, web client request, or auth/session problem.
2. Build a timeline. Keep clock source and timezone explicit when several files are involved.
3. Extract module, procedure, metadata object, HTTP path, query text, user/session, and transaction identifiers.
4. Map log entries back to source with `unica.search` and metadata with `unica.view` on the object node.
5. Use `unica.docs` with `source: "development-standard"` for diagnostic ids and `development-standard` recommendations. The exact meaning of a platform message requires `unica.docs` with `source: "platform-help"`.
6. Separate root cause from consequences: the first exception/lock/timeout usually matters more than later rollback noise.
7. For DBMS evidence, preserve lock holder/waiter, SQL text, transaction boundary, process id, session id, wait event, table/index name, and elapsed time together.

## Output

- Root-cause hypothesis with evidence lines.
- Timeline of key events.
- Affected code and metadata paths.
- Recommended fix or next measurement.
- Missing evidence, if the log fragment cannot support a reliable conclusion.

## MCP example

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.search",
    "arguments": {
      "query": "ВыполнитьОбменСКонтрагентом",
      "scope": "<source-set-from-unica-view>:Configuration",
      "limit": 20
    }
  }
}
```
