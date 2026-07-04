# ADR-0006: Workspace-scoped internal services

- Статус: `accepted`
- Дата: `2026-06-23`

## Контекст

Some internal engines are workspace-bound and expensive to warm repeatedly.
`bsl-analyzer` workspace MCP keeps an in-memory model, while RLM keeps a
persistent file index with build/update coordination. Starting those engines for
every Codex chat or every tool call loses warm state and can create duplicated
processes for the same workspace.

At the same time, exposing those engines as public MCP servers would violate the
single public MCP rule and return cache coordination back to the LLM.

## Решение

Unica may start hidden internal services scoped by workspace and source root.

1. Public MCP surface remains exactly one server: `unica`.
2. The `unica` orchestrator lazily starts a workspace service for
   `workspaceRoot + sourceRoot` when a non-dry-run analyzer/index operation
   needs it.
3. The workspace service communicates with `unica` through an internal localhost
   JSONL protocol protected by a per-service token stored in volatile cache
   state.
4. The service keeps `bsl-analyzer` workspace MCP warm and coordinates RLM
   index readiness/build/update for the same source root.
5. Service state lives under `<cacheRoot>/services/<serviceKey>/service.json`.
6. The default idle TTL is 7200 seconds and the hard max age is 28800 seconds;
   `UNICA_WORKSPACE_SERVICE_IDLE_SECS` and
   `UNICA_WORKSPACE_SERVICE_MAX_AGE_SECS` may override them.
7. Applied successful mutations notify live workspace services with domain
   events so analyzer state and index readiness can be invalidated without LLM
   coordination.

## Неграницы

1. This does not add public MCP tools or public MCP servers.
2. This does not make RLM a long-running process in v1; RLM remains a
   persistent index plus single-flight background build/update jobs.
3. This does not add a filesystem watcher dependency. Freshness is driven by
   request-time source fingerprints and explicit mutation events.
4. This does not require a user-level daemon that owns all workspaces.

## Последствия

1. Multiple Codex chats for the same workspace/source root can reuse one warm
   analyzer service.
2. Different workspaces or source roots get independent services and indexes.
3. Stale service records must be detected and replaced.
4. Packaged `.mcp.json` must set `cwd` to the plugin root, and packaged
   binaries must still locate the plugin root from their own executable path
   when `UNICA_PLUGIN_ROOT` is absent.
5. Tests must continue proving that `.mcp.json` exposes only `unica` and that
   cheap read-only operations such as `unica.code.grep` do not start services.

## Верификация

- [x] ADR preserves the single public MCP server rule.
- [x] ADR defines workspace service ownership and volatile state location.
- [x] ADR distinguishes persistent RLM index coordination from a long-running
      RLM process.
