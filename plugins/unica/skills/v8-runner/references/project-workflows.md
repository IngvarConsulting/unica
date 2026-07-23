# Project Workflows

`v8project.yaml` is the project contract. `v8project.local.yaml` is for local secrets and paths and must not redefine shared source topology or `execution_timeout`.

Typical empty workspace order:

1. Create `src/` if there are no source files.
2. Call `unica.runtime.execute` with `operation=config-init`.
3. Call `operation=init` only when the runtime state must be materialized.
4. If the database is the source of truth, call `operation=dump` with `mode=full`.
5. If Git sources are the source of truth, ask before calling `operation=build`.

`build` also prepares configured client MCP tool extensions when the project has `tools.client_mcp.extension`. Use `fullRebuild=true` if that generated state may be stale.

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
