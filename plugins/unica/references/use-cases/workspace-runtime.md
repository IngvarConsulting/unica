# Workspace And Runtime Workflows

## When to use

Use this when the user needs a new workspace, `v8project.yaml`, infobase
creation, source import/export, CF/CFE load, export or build, `.dt` export or
load, or a 1C client launch.

Do not use this for point edits inside XML metadata. Use the object-specific
skills for configuration roots, metadata objects, forms, DCS, MXL, roles,
subsystems, interfaces, and templates.

## Primary path

Use the package-selected MCP runtime surface directly. In v0.13, call
`unica.run {}` first and select only an operation whose dictionary entry says
`implemented: true`; do not infer arguments for planned operations whose
`argsSchema` is `null`.

Runtime идёт через `unica.run`: вызов без `op` отдаёт словарь операций и
контракт каждой — `argsSchema`, `execution`, `previewRequired`,
`ifRevRequiredOnApply`. Контракт вызова бери оттуда, а не из этого текста;
выбирай только операцию с `implemented: true` и не выдумывай аргументов
записи с `argsSchema: null`; превью исполнением не является. Не обходи
контракт прямым runner-ом.

After clone or workspace initialization, and before `source.import` or `source.export`, first
call `unica.check {}`. It returns `status`, `ready`, `repositoryReady`,
`checks[]` and `diagnostics[]` — the verdict on the workspace. The facts it
judges live in `unica.view {}`: `sourceSets` (possibly an empty array),
`config`, `infobase` and the recommended `v8project.yaml` content. The split is
the same as on a node, where `view {at}` gives `props` and `check {at}` gives
`status`. A false `ready` blocks the source operation
until its source-set problem is fixed. In particular, `sourceSet.path: .` is an
error: explain how to move the export into a strict child such as `src/` and
update `v8project.yaml` safely.

### Work the call must not wait for

A long operation does not need a separate call: any `unica.run` invocation
that outlives the handoff window becomes a durable Task. Keep the returned
`taskId`, read the state with `unica.task.get`, wait for a bounded interval with
`unica.task.result`, and cancel with `unica.task.cancel`; a client with native
Tasks uses `tasks/get` and `tasks/cancel` instead. The terminal result never
publishes raw stdout, so liveness is judged by the Task state, not by logs.

Each `sourceSets[].sourceFormat` describes working-tree discovery. Repository
checks may additionally become applicable from staged index markers; do not
interpret that as a rewrite of the published working-tree format.

A false `repositoryReady` does not mean Unica is unusable without Git. It means
portable Git policy has not been proved, so do not claim the workspace is ready
for team work or another clone. Follow `diagnostics[].remediation.steps` when
explaining a fix. `diagnostics[].remediation.commands` are advisory evidence,
not authorization to change `.gitignore`, `.gitattributes`, files, or the Git
index: never execute them automatically. After an approved fix, call
`unica.check {}` again.

Use `unica.view {}` when only the source layout or metadata format matters.
It returns discovered `sourceSets[]` with `kind`, `path`, `sourceFormat`, and
`formatEvidence`. Repository health is a verdict and lives in `unica.check {}`;
it can be ignored only when the task does not make portability or
team-readiness claims.

`v8project.yaml` can contain several source-sets. Format is resolved per
source-set, not for the workspace as a whole. One source-set cannot be mixed:
conflicting platform XML and EDT markers inside the same source-set make it
invalid/ambiguous. Different source-sets in the same project may use different
formats, for example an EDT configuration and platform XML external processors.
The top-level `format` value is only the default/effective format when the
source-set path itself has no stronger structural evidence.

| Intent | `unica.run` operation |
| --- | --- |
| Create the infobase named in `infobase.connection` | `infobase.create` (no arguments) |
| Load declared sources into the infobase | `source.import`, optional `sourceSet`, `fullRebuild` |
| Export sources from the infobase into a declared set | `source.export`, `mode=full` or `mode=incremental`, optional `sourceSet`, `extension` |
| Export the configuration or an extension as `.cf`/`.cfe` | `cf.export`, `state=working` or `state=database`, `output`, optional `extension` |
| Load a `.cf`/`.cfe` into the infobase | `cf.import`, `input`, `extension` for `.cfe` |
| Build a `.cf`/`.cfe` from sources | `artifact.build`, `output`, optional `sourceSet`, `extension`; `.epf`/`.erf` are not published |
| Export the whole infobase as `.dt` | `infobase.export`, `output` |
| Load a `.dt` | `infobase.import`, `input`, `mode=create` or `mode=replace` |
| Launch a 1C client | `client.run`, `clientMode`, optional `execute`, `waitForExit`, `waitTimeoutMs`; terminal, no preview required |

A previewApply operation is applied with the `ifRev` its preview returned; a
changed workspace or plan answers `revision_mismatch` or `concurrent_change`
instead of applying. Syntax checks are `unica.check`; test runs, Designer/EDT
conversion, Designer `rawKeys` and extension property sync are not on the v0.13
surface. Keep a
platform-generated CDFI sidecar out of Git; a legitimate metadata descriptor
(including an external EPF/ERF descriptor) for an object named
`ConfigDumpInfo` remains source.

## Related references

- `../tooling/v8project.md`
- `../tooling/runtime-build.md`
- `autonomous-server-debug.md`
