# 6. Представление времени выполнения

## Initialize

1. Source checkout `.mcp.json` starts `cargo run --manifest-path ../../Cargo.toml --bin unica` from the plugin root.
2. Packaged `.mcp.json` starts `./bin/<target>/unica` directly with `cwd` set
   to the plugin root.
3. The Rust runtime resolver starts internal bundled tools directly from
   `bin/<target>/<tool>`.
4. MCP `initialize` returns `serverInfo.name = "unica"`.

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

1. Caller invokes a mutating tool without `dryRun: false`.
2. Application resolves `dryRun: true`.
3. Adapter returns planned command or placeholder outcome without changing files.
4. Application emits the relevant domain event for impact calculation.
5. Cache report returns `mode = "dry-run"` and impacted cache names.

## Applied Mutation

1. Caller explicitly passes `dryRun: false`.
2. Application validates arguments, workspace/source-set/path containment, and
   support policy.
3. For a discovery-sensitive descriptor, application resolves exact targets and
   evaluates the configured discovery guard mode.
4. An allowed enforceable call with a valid receipt obtains an exclusive receipt
   lease before invoking the native handler. An `observe` or `warn` call allowed
   without a receipt has no receipt transition.
5. Application performs handler invocation. When a lease exists, the native MCP
   handler executes while it remains held.
6. Application captures typed mutation effects.
7. Application captures the post-mutation source snapshot.
8. It uses both to advance or revoke the presented receipt while the same
   exclusive receipt lease remains held.
9. Application must release the current receipt lease before touching any other
   receipt lock.
10. Successful mutation proceeds with domain event emission.
11. `WorkspaceStateRepository` performs cache invalidation and updates the cache
    report.
12. Application performs other-receipt reconciliation in deterministic
    receipt-id order.
13. Affected live analyzers receive workspace-service invalidation.
14. After the outcome is known, a non-authoritative shadow observation is
    appended. Recording failure adds operator diagnostics but never changes the
    handler or receipt decision.
15. The final result construction returns `{ ok, summary, changes, warnings,
    errors, artifacts, cache, discoveryGuard }`.

Steps 8 and 9 have no receipt transition when `observe` or `warn` permits an
applied call without a receipt. Receipt advancement/revocation cannot be moved
after domain events or cache effects, and other-receipt reconciliation cannot be
moved before the current lease is released.

## Read Operation

Read tools do not emit mutation events by default. They may inspect current cache
state and, in future slices, trigger lazy refresh if a required cache is stale.

## Project Discovery Explore

1. MCP validates the strict `unica.project.discover` request in `explore` mode
   and resolves the requested analysis source-set.
2. A bounded source-format-aware snapshot produces content fingerprints;
   `workspaceEpoch` remains diagnostic-only.
3. `DiscoverExtensionPointsUseCase` invokes six typed evidence ports, retaining
   coverage, provenance, freshness, stable failure codes, and conflicts.
4. The use case builds a deterministic evidence graph and separates related
   artifacts, connected flows, and actionable candidates.
5. The response reports `complete`, `partial`, or `insufficient`. Explore never
   creates a receipt.

## Project Discovery Validate

1. Validate repeats discovery against the current content snapshot rather than
   trusting an earlier explore response.
2. Each explicit proposal becomes `supported`, `contradicted`, or `unknown`.
   Missing positive evidence is contradictory only under complete fresh
   coverage.
3. Receipt eligibility is evaluated per selected proposal and its material
   checks; unrelated optional degradation can keep a report useful.
4. Fully supported unambiguous typed mutation intents produce server-owned
   atomic grants and a rolling receipt bound to the content-based composite
   fingerprint.

## Receipt Failure Paths

- Dry-run reports the guard decision but acquires no lease and advances no
  receipt.
- A failed handler with an unchanged manifest leaves the revision unchanged.
- Partial writes or an out-of-scope effect revoke the receipt.
- A competing caller receives `receipt_busy` or `stale_receipt_revision` before
  its handler runs.
- Other receipts linked to changed source-sets are reconciled only after the
  current lease is released.

## Shadow Observation And Replay

1. Once handler outcome and receipt transition are known, guard evaluation
   builds a sanitized observation from stable names, reason codes, counters,
   revisions, policy predicates, and digests.
2. The workspace-keyed store appends it to an OS-locked,
   schema-versioned JSONL journal and updates bounded aggregate counters.
3. The journal is non-authoritative; I/O failure cannot alter the operation,
   receipt, or rollout mode.
4. Maintainer-only audit validates schemas and counters. Deterministic replay
   evaluates stored policy predicates. Journal and replay records must never
   contain task text or source text.

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
