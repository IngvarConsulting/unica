# 6. Представление времени выполнения

## Initialize

1. Source checkout `.mcp.json` starts `cargo run --manifest-path ../../Cargo.toml --bin unica` from the plugin root.
2. Packaged `.mcp.json` invokes a command-scoped Git shell alias. The portable
   selector starts one native `unica-bootstrap` for the current host.
3. Bootstrap validates the pinned release manifest, obtains or reuses an atomic
   runtime cache, then replaces itself with or supervises `unica` while
   preserving stdio.
4. The Rust runtime resolver starts internal bundled tools directly from the
   cached `bin/<target>/<tool>` after SHA-256 verification.
5. MCP `initialize` returns `serverInfo.name = "unica"`.

## Tool List

1. MCP `tools/list` calls the application tool registry.
2. The response contains only `unica.*` tools.
3. Internal adapters are not listed.

## Concurrent MCP Dispatch and Cancellation

1. The stdio reader handles `initialize`, `tools/list`, and `ping` without
   waiting for an active `tools/call`; each tool call runs in its own worker.
   Input lines are capped at 8 MiB and only 32 tool workers are admitted.
   Oversized input returns `-32700`; saturation returns `-32603` with an
   `overloaded` message while synchronous `ping` and cancellation stay usable.
2. The dispatcher registers the JSON-RPC request ID with a cancellation token.
   Numeric and string IDs remain distinct.
3. `notifications/cancelled` with `params.requestId` cancels that token. The
   token is propagated through the application ports, CLI/index commands, and
   the workspace-service connector.
4. A cancelled request emits at most one response: JSON-RPC error `-32800` with
   message `request cancelled`. On EOF, already accepted workers get a 250 ms
   publication grace, then are cancelled and get up to 2 seconds to publish a
   terminal response. The publication-admission gate then closes without
   waiting for generic writer I/O: no response not already admitted may begin
   I/O after that linearization point. A publication already inside an arbitrary
   blocking `Write` may complete after `run_stdio_with_handler` returns; the real
   stdio process then exits and closes stdout. Response-writer failure also
   closes admission and cancels all active requests.

## Mutating Dry Run

This flow applies only when the operation has an honest, target-effect-free
preview. Platform mutations such as supported update use the branched
sandbox/prepare/apply flow instead.

1. Caller invokes a preview-capable mutating tool without `dryRun: false`.
2. Application resolves `dryRun: true`.
3. Adapter returns the plan without changing target/workspace/cache/domain
   state. A durable workflow may persist only the bounded operation/idempotency
   and preview evidence needed to replay and authorize apply.
4. Application emits the relevant domain event for impact calculation.
5. Cache report returns `mode = "dry-run"` and impacted cache names.

## Applied Mutation

1. Caller explicitly passes `dryRun: false` for a preview-capable tool, or
   authorizes a previously prepared no-preview transition.
2. Native MCP handler executes the operation.
3. Successful mutation emits domain events.
4. `WorkspaceStateRepository` marks affected caches stale and records eager
   refreshes.
5. Result returns `{ ok, summary, changes, warnings, errors, artifacts, cache }`.

## Read Operation

Read tools do not emit mutation events by default. They may inspect current cache
state and, in future slices, trigger lazy refresh if a required cache is stale.

## Branched Development Cycle

1. `branched.status` reads existing durable state; `repository.recover` is the
   only operation that persists reconciliation after an interrupted effect.
2. `branched.start` validates the named local profile, task/operation IDs,
   capability row, state/work roots, exclusive original-infobase lease, and
   dedicated repository-account reservation before creating a journal and owned
   instance.
3. Repository status and a digest-bound preview/apply update prove the
   still-bound original equals repository content. Exact incoming add/delete
   may trigger only the capability-proven internal structural confirmation.
   Delivery creates and probes distribution D0, deploys a disposable File IB,
   and creates a guarded platform-XML Unica workspace.
4. Compatible existing typed tools receive the original `cwd` plus an opaque
   `branchedTask` context. The application resolves the owned task workspace,
   routes its cache events there, and durably receipts mutations without
   exposing its path. Ordinary mutations atomically return to `developing` and
   invalidate all descendant workflow evidence; session-scoped manual work is
   isolated from task sources. Final local verification freezes the IB/XML
   boundary, reruns configured checks, and records an immutable
   delta/checkpoint.
5. A current repository distribution D1 is applied first in a restored sandbox.
   Typed conflict decisions are replayed from the checkpoint; only a verified
   equivalent/adapted delta is repeated against the authoritative task IB.
6. Main integration is prevalidated in a repository-fresh non-bound sandbox.
   The canonical delta and reference closure produce an explained lock plan.
7. Repository locks are acquired one object at a time with finite per-call
   deadlines, no polling, and intent-before-effect records. A failure compensates
   only observed operation-owned acquisitions; ambiguity enters
   `recoveryRequired`.
8. Relevant anchors are checked after locking. The original configuration is
   merged only with the prepared settings, then maximum validation and exact
   result fingerprints run while locks are held.
9. One exact integration-set commit, including add/delete entries and the frozen
   task-bound comment, is performed without force/keepLocked and accepted only
   after content and released-lock proof. Archive and cleanup follow the
   successful terminal state.
10. Safe abandonment requires original equality, no worker, no owned locks, and
    no unknown effect before archive/quarantine/cleanup.

Each long `operationId`-bound contained or authoritative platform operation runs
in a dedicated worker that survives MCP stdio disconnect. Worker death leaves a
durable observation boundary and is never blindly restarted; only a
proven-contained owned-area outcome resumes from its recorded safe phase.
Read-only platform inspection is bounded, ephemeral, target-effect-free, and
safely rerunnable after its owned process and temporary output are discarded.
Task-workspace mutations report cache events against the disposable workspace
context, not only the original caller context.

## Workspace Analyzer Service

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

## Bootstrap Cache Publication

1. The manifest must identify the exact source commit, `v<plugin-version>` tag,
   approved GitHub release origin, and all three supported targets.
2. A per-version/target lock serializes population under
   `$CODEX_HOME/unica/runtimes`.
3. Download and extraction occur in a UUID transaction directory on the same
   filesystem as the final cache.
4. Archive membership and SHA-256 hashes must exactly match the manifest.
5. `.ready.json` is written only after verification; the transaction is renamed
   atomically. Invalid prior state is quarantined and removed by the owning
   transaction.
6. `verify` performs MCP `initialize` and `tools/list` and requires the stable
   project/status and standards tools before reporting success.
