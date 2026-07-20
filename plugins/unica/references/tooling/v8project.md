# v8project.yaml Contract

`v8project.yaml` is the only project configuration format used by Unica skills.
Use MCP `unica.runtime.execute` argument `config` when the config file is not located at `./v8project.yaml`.

For a new repository with no workspace, use the `v8-runner` skill first. It
creates `v8project.yaml` through MCP `unica.runtime.execute`, prepares the
default `src` source-set, checks database access, and stops on license problems
instead of attempting environment repair.

Create or refresh the config through MCP `unica.runtime.execute`:

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.runtime.execute",
    "arguments": {
      "operation": "config-init",
      "config": "./v8project.yaml",
      "connection": "<connection-string>"
    }
  }
}
```

## Minimal Shape

```yaml
workPath: 'build'
execution_timeout: 300000
format: DESIGNER
builder: DESIGNER
infobase:
  connection: 'File=build/ib'
source-set:
  - name: main
    type: CONFIGURATION
    path: 'src'
build:
  partialLoadThreshold: 20
```

`infobase.connection` is the current runner key. Do not use legacy top-level
`connection` in `v8project.yaml`.

`execution_timeout` is the v8-runner operation budget in milliseconds. The
default is `300000`; v8-runner validates the value in the `1..=86400000` range.
For a known long operation, change this project config value instead of adding a
Unica wrapper timeout argument.

Server infobase connections use the normal 1C connection string form in
`infobase.connection`, for example `Srvr="srv01";Ref="dev";`. IBCMD server
connections also require the documented `infobase.dbms` block.

`v8project.local.yaml` is loaded automatically next to the primary config. It
may override local-only `workPath`, `infobase`, `tools`, `tests`, and `mcp`
settings. It cannot be passed as `config` and must not redefine shared
`source-set`, `format`, `builder`, or `execution_timeout`.

## Source-set format discovery

Use MCP `unica.project.map` to inspect configured source-sets before choosing a
metadata operation. It returns `sourceSets[]` where each entry has `kind`,
`path`, `sourceFormat`, and `formatEvidence`.

The top-level `format` field is a default/effective format, not proof that every
source-set under the workspace has the same layout. A project can contain an EDT
configuration source-set and platform XML external processor/report source-sets.
Within one source-set the format cannot be mixed: conflicting platform XML and
EDT markers mean the source-set is invalid/ambiguous and must be fixed or
converted before XML metadata tools are used.

Format discovery remains per source-set, but `unica.epf.init` and
`unica.erf.init` specifically require the global `format` value to be exact
`DESIGNER` or omitted. v8-runner selects the external-project layout from that
global value; use a separate Designer workspace/config when the active config
has global `format: EDT`.

## Command Mapping

Use the `v8-runner` skill and MCP `unica.runtime.execute` for runtime operations.

| Operation | MCP arguments |
| --- | --- |
| Create project config | `operation=config-init`, `connection=<connection>` |
| Initialize infobase/workspace | `operation=init` |
| Load XML sources and update DB | `operation=build` |
| Force full source load | `operation=build`, `fullRebuild=true` |
| Dump XML sources | `operation=dump`, `mode=full` |
| Preview incremental/selected dump | `operation=dump`, `mode=incremental|partial`, `dryRun=true`; partial also requires `object=TYPE:NAME` or `objects=[...]` |
| Load `.cf` / `.cfe` artifact | `operation=load`, `path=<file>`, `mode=load|merge` |
| Export `.cf` / `.cfe` artifact | `operation=make`, `output=<file>` |
| Launch 1C | `operation=launch`, `clientMode=thin|thick|designer|ordinary` |
| Run syntax checks | `operation=syntax`, `mode=designer-config|designer-modules|edt` |
| Run tests | `operation=test`, `testRunner=yaxunit|va` |
| Download configured tools | `operation=tools-download`, `tool=yaxunit|vanessa|client-mcp` |

## Skill Rules

- Do not create or read any legacy JSON project registry.
- Resolve the active config from the explicit MCP `config` argument when present; otherwise use `./v8project.yaml`.
- If the config is missing, use `operation=config-init` or ask for the connection string.
- Prefer `source-set` names over ad hoc source directories.
- Treat a platform-generated CDFI sidecar `ConfigDumpInfo.xml` whose root is `ConfigDumpInfo` as local per-infobase runtime state: keep it out of Git and never use it as source-format evidence. A legitimate metadata descriptor (including an external EPF/ERF descriptor) for an object actually named `ConfigDumpInfo` remains source and belongs in Git.
- Use `execution_timeout` in `v8project.yaml` for long runtime operations; Unica does not expose `timeoutMs` for `unica.runtime.execute`.
- Do not use `mode=update` for `operation=load`; v8-runner rejects it. Use `mode=load` or `mode=merge` with `settings`.
- When credentials are absent, try only empty-password `Администратор`, then empty-password `Admin`; if both fail, ask the user.
- If a command reports a 1C license problem, stop and ask the user to fix licensing. Do not edit license services, HASP settings, registry, or license files.
- If a runtime flag or debug-server step is missing from `unica.runtime.execute`, treat it as a Unica MCP contract gap. EPF/ERF dump/build flows use external source sets through `unica.runtime.execute`.
