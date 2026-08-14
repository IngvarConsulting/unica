# Project Workflows

`v8project.yaml` is the project contract. `v8project.local.yaml` is for local secrets and paths and must not redefine shared source topology or `execution_timeout`.

Typical empty workspace order:

1. Create `src/` if there are no source files.
2. Call `unica.runtime.execute` with `operation=config-init`.
3. Call `operation=init` only when the runtime state must be materialized.
4. If the database is the source of truth, call synchronous `operation=dump` with `mode=full`; for a DESIGNER configuration/extension Unica verifies platform 8.3.27 and staged exact 2.20 before publishing.
5. If Git sources are the source of truth, ask before calling `operation=build`.

Async full dump and external source-set dump remain preview-only. Applied
`convert` and Designer `rawKeys` containing `DumpConfigToFiles` or
`LoadConfigFromFiles` are fail-closed until they share the verified publication
boundary.

`build` also prepares configured client MCP tool extensions when the project has
`tools.client_mcp.extension`. Use `fullRebuild=true` if that generated state may
be stale. Without that explicit flag, synchronous and durable-job builds run the
normal build first; Unica does not inspect support state or preselect the full
path. External exit code `4` together with a valid structured runner failure
that proves a completed partial load is the only result that starts one full
retry. This classifies the failed stage but does not identify the cause or claim
that vendor support caused it.

That normal build carries `--json-message`, so the runner prints one structured
envelope at process exit instead of streaming text. Synchronous `stdout` is that
envelope, and a durable job keeps empty logs until it finishes; liveness comes
from `phase` and the heartbeat, not from log growth.

Explicit `fullRebuild=true` runs one full build and is never retried. Malformed
or unstructured output, a non-matching error, process spawn failure,
cancellation, a process timeout observed by Unica, or truncated output does not
start the fallback. The pinned receipt has no deferred internal timeout
metadata: a critical runner step that crosses its internal deadline and then
returns the exact completed partial failure is indistinguishable from the same
failure without that deadline and can still start the retry. A failed full retry
does not start a third attempt. This temporary Unica fallback does not replace
the separate runtime/runner redesign planned for v14.

Use `extensions` when only extension properties need synchronization.

Use `tools-download` when the project needs v8-runner-managed YaXUnit, Vanessa, or client MCP tool payloads refreshed.

Use `launch` with `clientMode=mcp` or `clientMode=mcp-va` for client-side MCP workflows; do not hand-assemble platform launch strings.

For a local external `.epf` whose exit status is required, use direct
`clientMode=thin` with `waitForExit=true`, bounded `waitTimeoutMs`, and distinct
paths: `output` is the platform `/Out` log, while `stderrOutput` captures stderr
from the 1C client process. Without this explicit opt-in, launch remains
asynchronous. Before launching Vanessa Automation, prepare it with
`operation=tools-download`, `tool=vanessa`; use the default managed
`build/tools/vanessa-automation-single.epf` or the effective
`tools.va.epf_path` override.
