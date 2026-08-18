# Workspace And Runtime Workflows

## When to use

Use this when the user needs a new workspace, `v8project.yaml`, infobase init,
source build/dump, Designer/EDT conversion, CF/CFE artifact load/export,
EPF/ERF external source-set build/export, syntax checks, tests, or 1C launch.

Do not use this for point edits inside XML metadata. Use the object-specific
skills for configuration roots, metadata objects, forms, DCS, MXL, roles,
subsystems, interfaces, and templates.

## Primary path

Use the `v8-runner` skill and MCP `unica.runtime.execute` only to preview typed
runtime arguments. For an explicitly requested applied build, select the
separate durable workflow described below before calling `runtime.execute`.

По INV-MCP-RUNTIME-RECEIPT текущий runtime-контракт: `unica.runtime.execute` — preview-only и вызывается
только с `dryRun: true`; любой applied-режим возвращает fail-closed до
workspace discovery и process spawn. Preview не является runtime verification.
Не обходи этот отказ прямым runner-ом, через `unica.build.*` или fallback через `unica.runtime.job.*`.

After clone or workspace initialization, and before `build` or `dump`, first
call `unica.project.status`. It returns `ready`, `repositoryReady`, `checks[]`,
`sourceSets` (an array after completed source discovery, otherwise `null`), and
`diagnostics[]`. A false `ready` blocks the source operation
until its source-set problem is fixed. In particular, `sourceSet.path: .` is an
error: explain how to move the export into a strict child such as `src/` and
update `v8project.yaml` safely.

### Explicitly selected applied build

When the user explicitly asks to build, load, or update the infobase from
sources, treat that applied intent as a separate, explicitly selected durable
workflow. After `unica.project.status` reports `ready: true`, explain that the
operation continues as a background job and call `unica.runtime.job.start`
with `operation=build` and `dryRun: false`. This is a direct workflow choice,
not a fallback, continuation, or retry after `unica.runtime.execute` refusal.

Keep the returned `jobId`. Read progress with `unica.runtime.job.status`, wait
for a bounded interval with `unica.runtime.job.wait`, and fetch diagnostic
tails with `unica.runtime.job.logs`. A normal build can keep both logs empty
until its terminal JSON envelope; use phase and heartbeat to distinguish that
from a stalled job.

Each `sourceSets[].sourceFormat` describes working-tree discovery. Repository
checks may additionally become applicable from staged index markers; do not
interpret that as a rewrite of the published working-tree format.

A false `repositoryReady` does not mean Unica is unusable without Git. It means
portable Git policy has not been proved, so do not claim the workspace is ready
for team work or another clone. Follow `diagnostics[].remediation.steps` when
explaining a fix. `diagnostics[].remediation.commands` are advisory evidence,
not authorization to change `.gitignore`, `.gitattributes`, files, or the Git
index: never execute them automatically. After an approved fix, call
`unica.project.status` again.

Use `unica.project.map` when only the source layout or metadata format matters.
It returns configured `sourceSets[]` with `kind`, `path`, `sourceFormat`, and
`formatEvidence`; it does not inspect repository health.

`v8project.yaml` can contain several source-sets. Format is resolved per
source-set, not for the workspace as a whole. One source-set cannot be mixed:
conflicting platform XML and EDT markers inside the same source-set make it
invalid/ambiguous. Different source-sets in the same project may use different
formats, for example an EDT configuration and platform XML external processors.
The top-level `format` value is only the default/effective format when the
source-set path itself has no stronger structural evidence.

| Intent | MCP arguments |
| --- | --- |
| Preview config creation | `operation=config-init`, optional `connection`, `format`, `builder`, `dryRun=true` |
| Preview binding an external EPF config locally | `operation=config-init`, required `config`, `sourceSet`, `connection`, `dryRun=true`; no local overlay is created |
| Preview runtime state creation | `operation=init`, `dryRun=true` |
| Preview applying sources to the infobase | `operation=build`, optional `sourceSet`, `fullRebuild`, `dryRun=true` |
| Apply sources through an explicitly selected durable job | `unica.runtime.job.start`, `operation=build`, optional `sourceSet`/`fullRebuild`, `dryRun=false` |
| Preview exporting infobase state | `operation=dump`, `mode=full`, optional matching `sourceSet`/`extension`, `dryRun=true` |
| Preview Designer/EDT conversion | `operation=convert`, optional `sourceSet`, `output`, `dryRun=true` |
| Preview CF/CFE/EPF/ERF export | `operation=make`, required `output`, optional `sourceSet`, `extension`, `dryRun=true` |
| Preview CF/CFE load | `operation=load`, required `path`, optional `mode`, `settings`, `extension`, `dryRun=true` |
| Preview syntax arguments | `operation=syntax`, required `mode`, `dryRun=true` |
| Preview test arguments | `operation=test`, required `testRunner`, `dryRun=true` |
| Preview client or Designer launch | `operation=launch`, required `clientMode`, `dryRun=true` |
| Preview external EPF wait arguments | `operation=launch`, `clientMode=thin`, `execute`, distinct `output`/`stderrOutput`, `waitForExit=true`, bounded `waitTimeoutMs`, `dryRun=true` |
| Preview extension property sync | `operation=extensions`, `dryRun=true` |

Every current applied `unica.runtime.execute` operation is fail-closed before
discovery or spawn. The
operation-specific risks include non-interruptible phases, persistent writes
without bounded recovery, and unproved ownership of separately grouped 1C
processes. Designer `rawKeys` may not contain `DumpConfigToFiles` or
`LoadConfigFromFiles`. Keep a
platform-generated CDFI sidecar out of Git; a legitimate metadata descriptor
(including an external EPF/ERF descriptor) for an object named
`ConfigDumpInfo` remains source.

## Related references

- `../tooling/v8project.md`
- `../tooling/runtime-build.md`
- `autonomous-server-debug.md`
