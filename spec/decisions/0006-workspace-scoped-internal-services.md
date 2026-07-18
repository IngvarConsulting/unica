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
8. Analyzer and RLM work requests carry an internal UUID `operation_id`. The
   service registers a cancellation token for each active operation; a
   `cancel` message uses a separate localhost connection and addresses that
   operation ID.
9. The public stdio dispatcher may execute `tools/call` requests concurrently.
   MCP `notifications/cancelled` flips the same request token that is propagated
   through the application and internal service connector. A cancelled public
   request completes once with JSON-RPC error `-32800` (`request cancelled`).
   Public JSONL input is limited to 8 MiB per line and at most 32 tool workers
   are admitted. An oversized line returns parse error `-32700`; excess work
   returns deterministic JSON-RPC error `-32603` containing `overloaded`.
10. The service accepts each connection independently. `ping`, `cancel`, and
    `shutdown` do not wait for analyzer or RLM work. RLM jobs may run
    independently, while access to the single warm analyzer session remains
    serialized. Disconnecting a work client cancels its operation.
    It admits at most 64 general connection handlers and 8 work workers. When
    general handlers are saturated, a bounded 64-connection classifier with a
    500 ms aggregate lifetime and a 64 KiB classification prefix preserves a
    separate capacity of 8 control handlers. Complete work received through
    that path is rejected with `workspace service overloaded: general connection
    handlers are saturated`; unclassified overflow is closed. The classifier
    is processed before each new accept and evicts its oldest pending socket at
    capacity, so idle clients cannot create an unbounded queue or starve a
    complete control line.
11. `shutdown` rejects new work, cancels every registered operation, removes
    the owned `service.json`, and exits after bounded handler cleanup.
12. Analyzer and RLM child processes are owned as process trees. On Windows a
    child is created suspended, assigned to a kill-on-close Job Object, and only
    then resumed. On Unix it runs in a dedicated process group. Cancellation,
    timeout, shutdown, or session failure terminates that tree with bounded
    waits. Other platforms retain the immediate-child fallback and therefore do
    not provide the descendant-cleanup guarantee.
13. Source selection is deterministic. A non-empty `sourceDir` is resolved
    relative to the request working directory; otherwise a source set named
    `main` wins, followed by the sole `CONFIGURATION` source set. Missing or
    ambiguous configuration roots fail with `invalid_source_root:`. Resolved
    paths are normalized, must stay inside the workspace, and the same effective
    root is used for analyzer, RLM, service identity, `project.status`, and
    `project.map`.
14. `plugins/unica/third-party/tools.lock.json` is the sole bundled-tool version
    authority. Package and executable-interface contract tests read the selected
    version from that lock; they do not duplicate a `bsl-analyzer` version
    literal in CI assertions.
15. Work and ordinary `Ping`, `Invalidate`, and `Shutdown` requests have one
    120-second overall deadline starting before connect. Control kinds have a
    500 ms connect cap; connect, write, flush, and read consume the remaining
    overall budget.
    Reads poll every 100 ms, and cancellation takes precedence over timeout, EOF,
    protocol, and successful process-exit races. A best-effort `Cancel` has a
    separate 500 ms aggregate budget for connect, write, and flush and does not
    read a response.
16. Public and internal request lines are limited to 8 MiB. Workspace-service
    request headers have one 5-second aggregate deadline beginning at accept;
    100 ms read slices do not reset it when a peer drips bytes.
17. A BSL MCP client request is retried at most once after a connector-level
    connect, write, flush, read, or premature-disconnect failure. Before the
    retry, the manager re-runs service discovery so the new request uses the
    current port and token. Typed service failures (`ok: false`), cancellation,
    deadline exhaustion, and invalid protocol responses are terminal and are
    never retried.

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
6. Cancellation is cooperative until it reaches a managed process boundary;
   bounded cleanup is guaranteed, but a cancelled operation is not reported as
   a successful partial result.

## Верификация

- [x] ADR preserves the single public MCP server rule.
- [x] ADR defines workspace service ownership and volatile state location.
- [x] ADR distinguishes persistent RLM index coordination from a long-running
      RLM process.
- [x] ADR defines operation-scoped cancellation and an independent control path.
- [x] ADR defines deterministic source-root selection and process-tree ownership.
- [x] ADR names `tools.lock.json` as the bundled-tool version authority.
