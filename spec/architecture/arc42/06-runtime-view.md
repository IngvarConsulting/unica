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
   navigation resolve the workspace and source root.
2. The application asks the internal workspace service manager for a service
   keyed by `workspaceRoot + sourceRoot`.
3. If a matching live service exists, `unica` sends an internal localhost JSONL
   request using the token from `service.json`.
4. If the service is missing, stale, unreachable, or has a mismatched version,
   `unica` starts hidden mode `unica --workspace-service ...`.
5. The service keeps one persistent `bsl-analyzer` workspace MCP child and
   restarts it when source generation or explicit invalidation changes.
6. RLM index readiness/build/update is coordinated by the same service, but the
   RLM index remains a persistent file index under the workspace cache root.

`initialize`, `tools/list`, `project.status`, `project.map`, `dryRun`, and
`unica.code.grep` do not start workspace analyzer services.
