---
name: autonomous-server
description: "Автономный сервер отладки 1С. Используй когда нужно развернуть или проанализировать локальный автономный контур для отладки HTTP-сервисов и веб-клиента, проверить URL, запуск клиента, изоляцию и диагностические артефакты. Не используй для обычной веб-публикации."
---

# Autonomous Server

## MCP routing

- Preferred path: use MCP `unica` tools `unica.project.map`, `unica.runtime.execute`, `unica.meta.info`, `unica.code.search`, and `unica.code.diagnostics`.
- Do not call internal runtime, server, analyzer, or package adapters directly. They are hidden behind MCP `unica`.

## Workflow

1. Identify the debug target: HTTP service, web service, web client scenario, client MCP session, or isolated infobase startup.
2. Map project source-sets with `unica.project.map`; inspect HTTP/WebService metadata with `unica.meta.info` and handlers with `unica.code.search`.
3. Prepare the infobase through `unica.runtime.execute` operations in order: `config-init` if needed, `init`, `build`, then `syntax`.
4. Launch the isolated client/debug surface with `unica.runtime.execute` and `operation=launch`. Use `clientMode=mcp` or `clientMode=mcp-va` when browser/client automation is the goal.
5. If the user provides or the runtime returns a web URL, report it as the hand-off point for an external browser-testing tool; otherwise report that no public MCP `unica` operation currently produced a web-client URL.
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
      "dryRun": false
    }
  }
}
```
