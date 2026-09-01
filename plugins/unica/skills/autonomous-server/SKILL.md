---
name: autonomous-server
description: "Автономный сервер отладки 1С. Используй когда нужно развернуть или проанализировать локальный автономный контур для отладки HTTP-сервисов и веб-клиента, проверить URL, запуск клиента, изоляцию и диагностические артефакты. Не используй для обычной веб-публикации."
---

# Autonomous Server

## MCP routing

- Preferred path: use MCP `unica` tools `unica.view {}`, `unica.runtime.execute`, `unica.meta.info`, `unica.code.search`, and `unica.code.diagnostics`.
- По INV-MCP-RUNTIME-RECEIPT и ADR-0074: `unica.runtime.execute` с `dryRun: true`
показывает запланированную команду без побочных эффектов, а с `dryRun: false`
исполняет классифицированную операцию и отвечает её терминальным результатом в
том же вызове, приложив названную причину риска (`runtime_risk_*`)
предупреждением; неклассифицированная операция по-прежнему отказывает
`runtime_operation_unbounded` до обнаружения рабочего пространства. Preview
исполнением не является. Работу, которую вызов ждать не должен, запускай через
`unica.runtime.job.start`. Не обходи контракт прямым runner-ом или через
`unica.build.*`.
- Do not call internal runtime, server, analyzer, or package adapters directly. They are hidden behind MCP `unica`.

## Workflow

1. Identify the debug target: HTTP service, web service, web client scenario, client MCP session, or isolated infobase startup.
2. Map project source-sets with `unica.view {}`; inspect HTTP/WebService metadata with `unica.meta.info` and handlers with `unica.code.search`.
3. Preview the intended infobase sequence through `unica.runtime.execute`: `config-init` if needed, `init`, `build`, then `syntax`; this does not prepare or verify the infobase.
4. Preview `operation=launch` with the intended `clientMode=mcp` or `clientMode=mcp-va`, then stop: no isolated client/debug surface is started by the current public contract.
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
    "name": "unica.runtime.execute",
    "arguments": {
      "cwd": "<workspace>",
      "operation": "launch",
      "clientMode": "mcp",
      "mode": "thin",
      "mcpPort": 1550,
      "dryRun": true
    }
  }
}
```
