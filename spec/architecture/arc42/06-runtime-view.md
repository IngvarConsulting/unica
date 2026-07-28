# 6. Представление времени выполнения

Глава описывает наблюдаемое поведение процессов во время работы. Строительные
блоки, которые здесь упоминаются, разобраны в
[главе 5](05-building-block-view.md); нормативные правила принадлежат
[реестру инвариантов](../invariants.md) и цитируются по ID.

## Initialize

1. Source checkout `.mcp.json` starts
   `cargo run --quiet --manifest-path ../../Cargo.toml --bin unica`. The Cargo
   manifest path is written relative to the plugin directory.
2. Packaged `.mcp.json` invokes a command-scoped Git shell alias. The portable
   selector starts one native `unica-bootstrap` for the current host.
3. Bootstrap validates the pinned release manifest, obtains or reuses an atomic
   runtime cache, then replaces itself with or supervises `unica` while
   preserving stdio.
4. The Rust runtime resolver starts internal bundled tools directly from the
   cached `bin/<target>/<tool>` after SHA-256 verification.
5. The MCP transport is the official Rust SDK (`rmcp`, ADR-0013). It enforces
   the specification handshake: the first request must be a well-formed
   `initialize` (`ping` may precede it), and `protocolVersion` is negotiated
   with the client.
6. MCP `initialize` returns `serverInfo.name = "unica"`.

`unica` не имеет подкоманд: по умолчанию процесс становится stdio MCP-сервером.
Аргумент `--help` или `-h` печатает версию и список поддерживаемых MCP-методов;
`--workspace-service` и `--runtime-job-worker` включают скрытые внутренние
режимы, описанные ниже.

## Tool List

1. MCP `tools/list` calls the application tool registry.
2. The response contains only `unica.*` tools.
3. Internal adapters are not listed.

## Concurrent MCP Dispatch and Cancellation

1. The SDK spawns each request into its own task, so `initialize`,
   `tools/list`, and `ping` never wait for an active `tools/call`. Tool
   execution runs in a blocking worker; only 32 tool calls are admitted at
   once, and saturation returns `-32603` with an `overloaded` message while
   `ping` and cancellation stay usable.
2. The SDK tracks each request ID with a cancellation token.
   `notifications/cancelled` with `params.requestId` cancels that token; the
   MCP interface bridges it to the domain cancellation token propagated
   through the application ports, CLI/index commands, and the
   workspace-service connector.
3. A request cancelled through `notifications/cancelled` gets no response, as
   the MCP specification prescribes; the SDK drops a late handler result.
4. On EOF the SDK drains finishing calls (bounded at 5 seconds), then the
   process cancels all still-running domain operations and waits up to
   2 seconds for them before exiting and closing stdout, so tool
   implementations can terminate their child process trees. Public input line
   length is delegated to the SDK transport; the internal workspace-service
   protocol keeps its own 8 MiB bound.

## Mutating Dry Run

1. Caller invokes a mutating tool without `dryRun: false`.
2. Application resolves `dryRun: true`.
3. Adapter returns planned command or placeholder outcome without changing files.
4. Application emits the relevant domain event for impact calculation.
5. Cache report returns `mode = "dry-run"` and impacted cache names.

## Applied Mutation

1. Caller explicitly passes `dryRun: false`.
2. Native MCP handler executes the operation.
3. Successful mutation emits domain events.
4. `WorkspaceStateRepository` marks affected caches stale and records eager
   refreshes.
5. Result returns `{ ok, summary, changes, warnings, errors, artifacts, cache }`.

## Read Operation

Read tools do not emit mutation events. They may inspect current cache state;
no lazy refresh path exists today, so a stale cache is reported rather than
rebuilt on read.

## Workspace Analyzer Service

Здесь и далее RLM — внутренний индекс BSL, который Unica строит бандлованным
инструментом `rlm-bsl-index` из проекта `rlm-tools-bsl`
(`Dach-Coin/rlm-tools-bsl`, MIT, версия закреплена в
`plugins/unica/third-party/tools.lock.json`). Аббревиатура пришла из
апстрим-проекта и в репозитории не расшифровывается. Индекс — файл SQLite под
кешем рабочего пространства, в каталоге `rlm-tools-bsl/`, рядом с
`bsl_index_status.json` и `bsl_index.lock`. Публичных инструментов `unica.rlm.*`
не существует: индекс скрыт за `unica.code.*` и `unica.meta.profile`
(INV-PRODUCT-03). См. также [глоссарий](12-glossary.md).

1. `unica.code.graph`, MCP-mode `unica.code.diagnostics`, and RLM-backed code
   navigation resolve the workspace and one effective source root. An explicit
   non-empty `sourceDir` is resolved relative to the request working directory.
   Without it, a source set named `main` wins; otherwise the sole
   `CONFIGURATION` source set wins. Missing or ambiguous choices fail with
   `invalid_source_root:` instead of silently using the workspace root.
2. The application asks the internal workspace service manager for a service
   keyed by normalized `workspaceRoot + sourceRoot`. The resolved source must
   remain inside the workspace and is also reported as the effective root by
   `project.status` and `project.map`.
3. If a matching live service exists, `unica` sends an internal localhost JSONL
   request using the token from `service.json`.
4. If the service is missing, stale, unreachable, or has a mismatched version,
   `unica` starts hidden mode `unica --workspace-service ...`.
5. The service keeps one persistent `bsl-analyzer` workspace MCP child and
   restarts it when source generation or explicit invalidation changes.
6. RLM index readiness/build/update is coordinated by the same service, but the
   RLM index remains a persistent file index under the workspace cache root.
   Readiness is classified as ready, missing, stale, building, failed, or
   unavailable; one index command is bounded at 30 seconds, the index lock is
   refreshed every 5 seconds and treated as stale after 10 minutes.
7. Every analyzer or RLM work request carries a UUID `operation_id`. The shared
   runtime registers one cancellation token per operation and rejects duplicate
   IDs or new work after shutdown begins.
8. Accepted connections are handled independently. `ping`, `cancel`, and
   `shutdown` never acquire the analyzer lane and remain responsive while work
   is active. RLM jobs run outside that lane; only mutable access to the single
   warm analyzer session is serialized.
   The runtime caps general handlers at 64 and work workers at 8. Saturated
   general capacity feeds a bounded 64-socket, 500 ms aggregate control
   classifier (64 KiB classification prefix) and up to 8 reserved control
   handlers. It rejects classified work with the stable general-handler
   overload error and closes unclassified overflow; no unbounded thread or
   connection queue is created.
9. MCP cancellation causes the connector to send `cancel { operation_id }` on a
   separate connection. A disconnected work socket also cancels its operation.
   An operation guard removes the ID on every completion path, so the next call
    does not require a service restart.
10. Work and ordinary `Ping`, `Invalidate`, and `Shutdown` requests have one
    120-second overall deadline starting before connect. Control kinds use a
    500 ms connect cap; connect, write, flush, and read use the remaining
    overall budget. Reads poll every 100 ms so cancellation can be observed;
    cancellation takes precedence over timeout, EOF, protocol, and successful
    process-exit races. A best-effort `Cancel` is different: it uses a separate
    500 ms aggregate budget for connect, write, and flush and does not read a response.
    Internal request/response lines are capped at 8 MiB. Request-header parsing
    has one 5-second aggregate deadline from accept and polls in at most 100 ms
    slices; receiving another byte never resets the deadline.
11. Shutdown marks the runtime unavailable, cancels all active operations,
    rejects new work, removes the service record it owns, and drains handlers
    within the configured grace period.
12. Persistent analyzer and RLM subprocesses use `ManagedChild`. Windows starts
    each process suspended, assigns it to a kill-on-close Job Object, then
    resumes it; Unix creates a dedicated process group. Cancellation, timeout,
    and drop terminate the whole tree with bounded waits. Platforms other than
    Windows and Unix provide only immediate-child termination.

`initialize`, `tools/list`, `project.status`, `project.map`, `dryRun`, and
`unica.code.grep` do not start workspace analyzer services.

Bundled executable versions and assets are selected from
`plugins/unica/third-party/tools.lock.json`. CI validates the CLI/MCP surface of
the artifact selected by that lock rather than embedding a second analyzer
version constant.

## Durable Runtime Jobs

Долгие операции рантайма 1С не держат MCP-вызов открытым: они вынесены в
отдельный процесс с состоянием на диске. Публичная поверхность —
`unica.runtime.job.start`, `.status`, `.wait`, `.logs`, `.cancel`, `.list`;
синхронный `unica.runtime.execute` остаётся без изменений.

1. `unica.runtime.job.start` resolves the plugin root, resolves the bundled
   `v8-runner`, and rejects the bounded external-EPF arguments `waitForExit`,
   `waitTimeoutMs`, and `stderrOutput`, which belong to
   `unica.runtime.execute`. Without `dryRun: false` it returns the planned
   command and starts nothing.
2. An applied start accepts one of the operation labels `make`, `syntax`,
   `test`, `tools-download`, `config-init`, `init`, `build`, `dump`, `convert`,
   `load`, `launch`, `extensions`, enqueues a job, and spawns a detached
   worker.
3. The worker is the same executable re-invoked as
   `unica --runtime-job-worker`. The start request is handed over the worker's
   stdin as one JSON document and is never passed through argv, so raw
   arguments never reach the process table. The worker ignores its own
   arguments: the flag only selects the mode.
4. Job state lives under `<cacheRoot>/jobs/`. Each job owns
   `<jobId>/record.json`, `stdout.log`, `stderr.log`, and, once cancellation is
   requested, `cancel.json`. Raw arguments are never persisted verbatim, and
   captured output is redacted before it is written.
5. `jobs/active.lock` admits one active job per workspace cache root. A lock
   whose owner stopped sending heartbeats is recovered under a separate
   recovery lock after five minutes, and the abandoned job is finished as
   `lost`.
6. Phases are `queued`, `running`, `cancelRequested`, `succeeded`, `failed`,
   `cancelled`, `timedOut`, and `lost`. Only the first three are non-terminal,
   and every transition is validated against the allowed successors.
7. The worker polls its job every 25 ms until a terminal phase, writing a
   heartbeat and the bounded 16 KiB output tails on each poll.
8. `unica.runtime.job.cancel` writes a cancel marker rather than killing a
   process directly. An operation classified as safe to interrupt — `make`,
   `syntax`, `test`, `tools-download` — is cancelled through the platform
   process-tree facade with a bounded five-second reap and finishes as
   `cancelled`. Every other operation can leave the workspace inconsistent, so
   the job reports `cancelRequested` with `cancelDeferred` and the unsafe
   phase, and still runs to completion.
9. `unica.runtime.job.status` and `.list` read snapshots and never block.
   `.wait` polls every 25 ms up to a caller-side `timeoutSeconds` that defaults
   to 30 and, on expiry, returns the current snapshot with `waitTimedOut`
   instead of failing. `.logs` returns redacted stdout and stderr tails and
   their paths, defaulting to 4096 characters.
10. On a successful terminal phase the worker rediscovers the workspace from
    the recorded working directory, emits the domain event of the operation,
    reports cache impact through `WorkspaceStateRepository`, and notifies live
    workspace services of the invalidation (INV-CACHE-05). A failure to apply
    those effects is recorded as a job warning, never as a lost mutation.

## Bootstrap Cache Publication

1. The manifest must identify the exact source commit, `v<plugin-version>` tag,
   approved GitHub release origin, and all three supported targets.
2. The runtime cache root is resolved host-neutrally, in a fixed order:
   `UNICA_RUNTIME_CACHE_DIR` — discarded when the value still contains an
   unexpanded `${` token that the host did not substitute — then
   `CLAUDE_PLUGIN_DATA` with a `runtimes` suffix, then `CODEX_HOME`, then
   `HOME` or `USERPROFILE` with a `.codex` suffix; the last two also append
   `unica/runtimes`. When none of them is set, bootstrap fails instead of
   guessing (INV-CACHE-07, ADR-0012).
3. A per-version and per-target lock serializes population of
   `<cacheRoot>/<pluginVersion>/<target>`.
4. Download and extraction occur in a UUID transaction directory on the same
   filesystem as the final cache.
5. Archive membership and SHA-256 hashes must exactly match the manifest.
6. `.ready.json` is written only after verification; the transaction is renamed
   atomically. Invalid prior state is quarantined and removed by the owning
   transaction.
7. `verify` performs MCP `initialize` and `tools/list` and requires the stable
   project/status and standards tools before reporting success.
