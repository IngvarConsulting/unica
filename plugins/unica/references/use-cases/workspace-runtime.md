# Workspace And Runtime Workflows

## When to use

Use this when the user needs a new workspace, `v8project.yaml`, infobase init,
source build/dump, Designer/EDT conversion, CF/CFE artifact load/export,
EPF/ERF external source-set build/export, syntax checks, tests, or 1C launch.

Do not use this for point edits inside XML metadata. Use the object-specific
skills for configuration roots, metadata objects, forms, DCS, MXL, roles,
subsystems, interfaces, and templates.

## Primary path

Use the `v8-runner` skill and MCP `unica.runtime.execute`.

When the source layout or metadata format matters, first call
`unica.project.map`. It returns configured `sourceSets[]` with `kind`, `path`,
`sourceFormat`, and `formatEvidence`.

`v8project.yaml` can contain several source-sets. Format is resolved per
source-set, not for the workspace as a whole. One source-set cannot be mixed:
conflicting platform XML and EDT markers inside the same source-set make it
invalid/ambiguous. Different source-sets in the same project may use different
formats, for example an EDT configuration and platform XML external processors.
The top-level `format` value is only the default/effective format when the
source-set path itself has no stronger structural evidence.

| Intent | MCP arguments |
| --- | --- |
| Create config | `operation=config-init`, optional `connection`, `format`, `builder` |
| Bind external EPF config locally | `operation=config-init`, required `config`, `sourceSet`, `connection`; creates only adjacent `v8project.local.yaml` |
| Prepare runtime state | `operation=init` |
| Apply sources to the infobase | `operation=build`, optional `sourceSet`, `fullRebuild` |
| Export infobase state to configuration/extension files | synchronous `operation=dump`, `mode=full`, optional matching `sourceSet`/`extension`; verified platform 8.3.27.x and staged raw 2.20 |
| Preview Designer/EDT conversion | `operation=convert`, optional `sourceSet`, `output`, `dryRun=true`; applied conversion is fail-closed |
| Export CF/CFE/EPF/ERF artifacts | `operation=make`, required `output`, optional `sourceSet`, `extension` |
| Load CF/CFE artifacts | `operation=load`, required `path`, optional `mode`, `settings`, `extension` |
| Run syntax checks | `operation=syntax`, required `mode` |
| Run tests | `operation=test`, required `testRunner` |
| Launch client or Designer | `operation=launch`, required `clientMode` |
| Run external EPF and wait | `operation=launch`, `clientMode=thin`, `execute`, distinct `output`/`stderrOutput`, `waitForExit=true`, bounded `waitTimeoutMs` |
| Sync extension properties | `operation=extensions` |

Applied `mode=incremental|partial` is temporarily fail-closed because the
pinned runner has no exact path/hash receipt or divergence-safe merge. DESIGNER
writes those modes directly to the source root; EDT stages final publication
but still cannot prove a safe merge. Async full dump and external source-set
dump are also preview-only until they use the verified private-stage boundary.
Applied `convert` is blocked for the same reason. Designer `rawKeys` may not
contain `DumpConfigToFiles` or `LoadConfigFromFiles`. Use `dryRun=true` to
preview blocked routes. Keep a
platform-generated CDFI sidecar out of Git; a legitimate metadata descriptor
(including an external EPF/ERF descriptor) for an object named
`ConfigDumpInfo` remains source.

## Related references

- `references/tooling/v8project.md`
- `references/tooling/runtime-build.md`
- `references/use-cases/autonomous-server-debug.md`
