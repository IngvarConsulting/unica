---
name: autonomous-server
description: "Автономный сервер отладки 1С. Используй когда нужно развернуть или проанализировать локальный автономный контур для отладки HTTP-сервисов и веб-клиента, проверить URL, запуск клиента, изоляцию и диагностические артефакты. Не используй для обычной веб-публикации."
---

# Autonomous Server

## MCP routing

- Preferred path: use MCP `unica` tools `unica.view {}`, `unica.run`, `unica.view` on the object node, `unica.search`, and `unica.check`.
- Runtime идёт через `unica.run`: вызов без `op` отдаёт словарь операций и
контракт каждой — `argsSchema`, `execution`, `previewRequired`,
`ifRevRequiredOnApply`. Контракт вызова бери оттуда, а не из этого текста;
при `implemented: true` используй опубликованную `argsSchema`; при
`support.state: limited` разрешено только подмножество `support.supportedArgs`.
При `support.state: unavailable` остановись; не выдумывай аргументов при
`argsSchema: null`. Превью исполнением не является. Не обходи контракт прямым runner-ом.
- Do not call internal runtime, server, analyzer, or package adapters directly. They are hidden behind MCP `unica`.

## Workflow

1. Identify the debug target: HTTP service, web service, web client scenario, client MCP session, or isolated infobase startup.
2. Map project source-sets with `unica.view {}`; inspect HTTP/WebService metadata with `unica.view` on the object node and handlers with `unica.search`.
3. Check the workspace with `unica.check {}`. The target bootstrap sequence is `infobase.create` then `push`, but both creation and source sending are unavailable with runner 0.11. Report this gap; use an existing prepared infobase for supported operations.
4. Launch the client with `launch` (`clientMode=thin`), then stop: an MCP client mode and a web-client URL are not on the v0.13 surface.
5. If the user independently provides a web URL, report it as the hand-off point for an external browser-testing tool; otherwise report that no public MCP `unica` operation currently produces a web-client URL.
6. Analyze server artifacts: startup command/result, URL, source-set, platform mode, handler metadata, diagnostics, event log or technological log files if provided.

## Diagnostics

- Read `../../references/platform/runtime-diagnostics.md` before explaining startup, HTTP-service, web-client, or process-level failures.
- Preserve launch command/result, platform version, infobase path, port, URL, client mode, source-set, process id, session id, and temporary artifact paths.
- For HTTP-service debugging, map URL path to metadata and handler module before interpreting the error.
- For web-client debugging, separate server startup, authentication, UI load, client script failure, and business error.
- If runtime output does not expose a URL, log path, or process id through public MCP `unica`, record a Unica MCP contract gap.

## Boundaries

- This skill is for local autonomous debugging, not for production deployment.
- Do not create a legacy web server deployment skill surface. If a task requires a missing runtime operation, report it as a Unica MCP contract gap.
- Keep credentials out of versioned files and final output.

## MCP example

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.run",
    "arguments": {
      "op": "launch",
      "args": {
        "clientMode": "thin"
      }
    }
  }
}
```
