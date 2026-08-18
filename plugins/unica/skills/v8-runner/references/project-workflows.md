# Project Workflows

- По INV-MCP-RUNTIME-RECEIPT и ADR-0074: `unica.runtime.execute` с `dryRun: true`
показывает запланированную команду без побочных эффектов, а с `dryRun: false`
исполняет операцию и отвечает её терминальным результатом в том же вызове,
приложив названную причину риска (`runtime_risk_*`) предупреждением. Preview
исполнением не является. Работу, которую вызов ждать не должен, запускай через
`unica.runtime.job.start`. Не обходи контракт прямым runner-ом или через
`unica.build.*`.

`v8project.yaml` is the project contract. `v8project.local.yaml` is for local secrets and paths and must not redefine shared source topology or `execution_timeout`.

Typical empty workspace order:

1. Create `src/` if there are no source files.
2. Preview `operation=config-init` with `dryRun=true`; applied config writes are fail-closed, so stop and ask for a project config instead of bypassing MCP.
3. Preview `operation=init` with `dryRun=true` when runtime state must be materialized; applied init is fail-closed before spawn.
4. If the database is the source of truth, preview synchronous `operation=dump` with `mode=full`; applied dump remains fail-closed because its post-run validation/publication has no proved receipt bound.
5. If Git sources are the source of truth, ask before previewing `operation=build` with `dryRun=true`; applied build is not currently admitted.

All dump modes and applied `convert` write persistent state without a bounded recovery contract, and the result names that risk. Designer `rawKeys` containing `DumpConfigToFiles` or
`LoadConfigFromFiles` are fail-closed until they share the verified publication
boundary.

For a future admitted applied operation, `build` also prepares configured client
MCP tool extensions when the project has `tools.client_mcp.extension`.
Currently only preview `fullRebuild=true` when that generated state may be
stale; the preview does not prepare the extension.

Only a durable build carries the one full retry. The synchronous entry point is
refused before it ever starts a process, so it has no first attempt to repeat,
and its preview still shows the normalized command. In a durable build without
`fullRebuild=true`, Unica runs the normal build first and does not inspect
support state or preselect the full path. External exit code `4` together with a valid structured runner
failure that proves a completed partial load is the only result that starts one
full retry. It classifies the failed stage but does not identify the cause or
claim that vendor support caused it.

That normal build carries `--json-message`, so the runner prints one structured
envelope at process exit instead of streaming text. The durable job therefore
keeps empty logs until it finishes; liveness comes from `phase` and the
heartbeat, not from log growth.

Explicit `fullRebuild=true` runs one full build and is never retried. Malformed
or unstructured output, a non-matching error, process spawn failure,
cancellation, a process timeout observed by Unica, or truncated output does not
start the fallback. The pinned receipt has no deferred internal timeout
metadata: a critical runner step that crosses its internal deadline and then
returns the exact completed partial failure is indistinguishable from the same
failure without that deadline and can still start the retry. A failed full retry
does not start a third attempt. This temporary Unica fallback does not replace
the separate runtime/runner redesign planned for v14.

Preview `extensions` with `dryRun=true` when only extension properties need synchronization; applied synchronization is not currently admitted.

Preview `tools-download` with `dryRun=true` when the project needs
v8-runner-managed YaXUnit, Vanessa, or client MCP payloads. Applied download is
fail-closed until the runner exposes bounded atomic publication; require an
already prepared managed artifact before continuing.

Preview `launch` with `clientMode=mcp` or `clientMode=mcp-va` for client-side MCP workflows; detached applied launch is not admitted, and platform launch strings must not be hand-assembled.

For a local external `.epf`, preview direct `clientMode=thin` with
`waitForExit=true`, bounded `waitTimeoutMs`, `dryRun=true`, and distinct paths:
`output` is the platform `/Out` log, while `stderrOutput` captures stderr from
the 1C client process. Applied launch fails closed before spawn even with this
opt-in. Launch Vanessa Automation only when the default managed
`build/tools/vanessa-automation-single.epf` or the effective
`tools.va.epf_path` override already exists; `tools-download` can currently
preview, but not publish, that artifact.
