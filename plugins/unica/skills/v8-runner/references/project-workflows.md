# Project Workflows

- По INV-MCP-RUNTIME-RECEIPT текущий runtime-контракт: `unica.runtime.execute` — preview-only и вызывается только с `dryRun: true`; любой applied-режим возвращает fail-closed до workspace discovery и process spawn. Preview не является runtime verification. Не обходи этот отказ прямым runner-ом, через `unica.build.*` или fallback через `unica.runtime.job.*`.

`v8project.yaml` is the project contract. `v8project.local.yaml` is for local secrets and paths and must not redefine shared source topology or `execution_timeout`.

Typical empty workspace order:

1. Create `src/` if there are no source files.
2. Preview `operation=config-init` with `dryRun=true`; applied config writes are fail-closed, so stop and ask for a project config instead of bypassing MCP.
3. Preview `operation=init` with `dryRun=true` when runtime state must be materialized; applied init is fail-closed before spawn.
4. If the database is the source of truth, preview synchronous `operation=dump` with `mode=full`; applied dump remains fail-closed because its post-run validation/publication has no proved receipt bound.
5. If Git sources are the source of truth, ask before previewing `operation=build` with `dryRun=true`; applied build is not currently admitted.

All dump modes and applied `convert` remain preview-only. Designer `rawKeys` containing `DumpConfigToFiles` or
`LoadConfigFromFiles` are fail-closed until they share the verified publication
boundary.

For a future admitted applied operation, `build` also prepares configured client
MCP tool extensions when the project has `tools.client_mcp.extension`.
Currently only preview `fullRebuild=true` when that generated state may be
stale; the preview does not prepare the extension.

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
