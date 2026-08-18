# Workspace And Runtime Workflows

## When to use

Use this when the user needs a new workspace, `v8project.yaml`, infobase init,
source build/dump, Designer/EDT conversion, CF/CFE artifact load/export,
EPF/ERF external source-set build/export, syntax checks, tests, or 1C launch.

Do not use this for point edits inside XML metadata. Use the object-specific
skills for configuration roots, metadata objects, forms, DCS, MXL, roles,
subsystems, interfaces, and templates.

## Primary path

Use the `v8-runner` skill and MCP `unica.runtime.execute` both to preview typed
runtime arguments and to run them: `dryRun: false` executes the operation and
returns its terminal result in the same call.

По INV-MCP-RUNTIME-RECEIPT и ADR-0074: `unica.runtime.execute` с `dryRun: true`
показывает запланированную команду без побочных эффектов, а с `dryRun: false`
исполняет классифицированную операцию и отвечает её терминальным результатом в
том же вызове, приложив названную причину риска (`runtime_risk_*`)
предупреждением; неклассифицированная операция по-прежнему отказывает
`runtime_operation_unbounded` до обнаружения рабочего пространства. Preview
исполнением не является. Долговременное задание запускай через
`unica.runtime.job.start` для явно выбранной длинной работы; не используй
`unica.runtime.job.start` как запасной путь. Не обходи контракт прямым
runner-ом или через `unica.build.*`.

After clone or workspace initialization, and before `build` or `dump`, first
call `unica.project.status`. It returns `ready`, `repositoryReady`, `checks[]`,
`sourceSets` (an array after completed source discovery, otherwise `null`), and
`diagnostics[]`. A false `ready` blocks the source operation
until its source-set problem is fixed. In particular, `sourceSet.path: .` is an
error: explain how to move the export into a strict child such as `src/` and
update `v8project.yaml` safely.

### Work the call must not wait for

A long applied operation belongs in a durable job: `unica.runtime.job.start` runs
it in a detached process that outlives the call, so a host deadline cannot lose
the result. Keep the returned `jobId`, read progress with
`unica.runtime.job.status`, wait for a bounded interval with
`unica.runtime.job.wait`, and fetch diagnostic tails with
`unica.runtime.job.logs`. A normal build can keep both logs empty until its
terminal envelope; phase and heartbeat are what distinguish that from a stalled
job.

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
| Preview exporting infobase state | `operation=dump`, `mode=full`, optional matching `sourceSet`/`extension`, `dryRun=true` |
| Preview Designer/EDT conversion | `operation=convert`, optional `sourceSet`, `output`, `dryRun=true` |
| Preview CF/CFE/EPF/ERF export | `operation=make`, required `output`, optional `sourceSet`, `extension`, `dryRun=true` |
| Preview CF/CFE load | `operation=load`, required `path`, optional `mode`, `settings`, `extension`, `dryRun=true` |
| Preview syntax arguments | `operation=syntax`, required `mode`, `dryRun=true` |
| Preview test arguments | `operation=test`, required `testRunner`, `dryRun=true` |
| Preview client or Designer launch | `operation=launch`, required `clientMode`, `dryRun=true` |
| Preview external EPF wait arguments | `operation=launch`, `clientMode=thin`, `execute`, distinct `output`/`stderrOutput`, `waitForExit=true`, bounded `waitTimeoutMs`, `dryRun=true` |
| Preview extension property sync | `operation=extensions`, `dryRun=true` |

Every applied operation carries its own named risk into the result instead of a
refusal: non-interruptible phases, persistent writes without bounded recovery,
unproved ownership of separately grouped 1C processes, or a detached child. An
operation the completion map does not classify still fails closed before
discovery or spawn. Designer `rawKeys` may not contain `DumpConfigToFiles` or
`LoadConfigFromFiles`. Keep a
platform-generated CDFI sidecar out of Git; a legitimate metadata descriptor
(including an external EPF/ERF descriptor) for an object named
`ConfigDumpInfo` remains source.

## Related references

- `../tooling/v8project.md`
- `../tooling/runtime-build.md`
- `autonomous-server-debug.md`
