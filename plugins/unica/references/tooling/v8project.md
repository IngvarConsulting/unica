# v8project.yaml Contract

`v8project.yaml` is the only project configuration format used by Unica skills.
Use `V8TR_CONFIG` when the config file is not located at `./v8project.yaml`.

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
basePath: '.'
workPath: 'build'
format: DESIGNER
builder: DESIGNER
connection: '/F/Users/me/1c-bases/dev'
source-set:
  - name: main
    type: CONFIGURATION
    path: 'src'
build:
  partialLoadThreshold: 20
```

Server infobase connections use the normal 1C connection string form, for
example `/Sserver/ref`.

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

## Command Mapping

Use the `v8-runner` skill and MCP `unica.runtime.execute` for runtime operations.

| Operation | MCP arguments |
| --- | --- |
| Create project config | `operation=config-init`, `connection=<connection>` |
| Initialize infobase/workspace | `operation=init` |
| Load XML sources and update DB | `operation=build` |
| Force full source load | `operation=build`, `fullRebuild=true` |
| Dump XML sources | `operation=dump`, `mode=full|incremental|partial` |
| Dump selected objects | `operation=dump`, `mode=partial`, `object=TYPE:NAME` |
| Load `.cf` / `.cfe` artifact | `operation=load`, `path=<file>`, `mode=load|merge|update` |
| Export `.cf` / `.cfe` artifact | `operation=make`, `output=<file>` |
| Launch 1C | `operation=launch`, `clientMode=thin|thick|designer|ordinary` |
| Run syntax checks | `operation=syntax`, `mode=designer-config|designer-modules|edt` |
| Run tests | `operation=test`, `testRunner=yaxunit|va` |

## Skill Rules

- Do not create or read any legacy JSON project registry.
- Resolve the active config as `V8TR_CONFIG` first, then `./v8project.yaml`.
- If the config is missing, use `operation=config-init` or ask for the connection string.
- Prefer `source-set` names over ad hoc source directories.
- When credentials are absent, try only empty-password `Администратор`, then empty-password `Admin`; if both fail, ask the user.
- If a command reports a 1C license problem, stop and ask the user to fix licensing. Do not edit license services, HASP settings, registry, or license files.
- Use native skill scripts only for operations v8-runner does not expose directly, such as web Apache publication helpers. EPF/ERF dump/build flows use external source sets through `unica.runtime.execute`.
