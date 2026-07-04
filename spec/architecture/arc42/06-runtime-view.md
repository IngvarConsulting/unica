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
2. Native MCP handler executes the operation. A transitional adapter may execute
   only for not-yet-migrated operations.
3. Successful mutation emits domain events.
4. `WorkspaceStateRepository` marks affected caches stale and records eager
   refreshes.
5. Result returns `{ ok, summary, changes, warnings, errors, artifacts, cache }`.

## Read Operation

Read tools do not emit mutation events by default. They may inspect current cache
state and, in future slices, trigger lazy refresh if a required cache is stale.

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
