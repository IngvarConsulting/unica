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
`implemented: true`, or exactly the subset declared by `support.state: limited` and `support.supportedArgs`; refuse unavailable operations. Do not infer arguments for operations whose
`argsSchema` is `null`.

Runtime идёт через `unica.run`: вызов без `op` отдаёт словарь операций и
контракт каждой — `argsSchema`, `execution`, `previewRequired`,
`ifRevRequiredOnApply`. Контракт вызова бери оттуда, а не из этого текста;
при `implemented: true` используй опубликованную `argsSchema`; при
`support.state: limited` разрешено только подмножество `support.supportedArgs`.
При `support.state: unavailable` остановись; не выдумывай аргументов при
`argsSchema: null`. Превью исполнением не является. Не обходи контракт прямым runner-ом.

With runner 0.11.2, source sending and full pulling require explicit `force:true`.
They provide no generation or local-work protection; inspect the preview before execution.
For source readiness independently of runtime availability, first
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
publishes raw stdout. The Task state is the authority for the outcome; a
`working` state alone does not prove that the runner is making progress.

### If a mutating runner remains `working`

Keep the `taskId` and the operation's infobase and workspace identity. Read
`tasks/get` (or `unica.task.get`) and use `unica.task.result` only for a bounded
wait in the compatibility profile. A late `tasks/cancel` is a request, not a
receipt that the mutation stopped: native Tasks report it in the next
`tasks/get.statusMessage`; compatibility tools expose `cancelRequested: true`.
If the cancel request itself times out, read the Task again before deciding
what happened. The request may already have been saved while Windows waits to
attach the runner to its Job Object. `ttlMs` controls retention, not execution.

When `working` outlasts the operation's expected window, have the operator
check the runner process and platform session on the host, the target infobase,
and the available platform diagnostics. Escalate to the person responsible for
that infobase if progress cannot be established or the occupied Task capacity
blocks other work. Do not infer progress from `working`, rerun the mutation,
or kill its process tree merely because cancellation was requested. An
intervention in the runner or daemon needs an explicit operational decision
with the target infobase identified.

After a daemon restart, `outcome_uncertain` means the mutation may have taken
effect although no final receipt was durably saved. Inspect the actual
infobase state and intended change, record the finding alongside the `taskId`,
then decide a new action from that evidence. Unica does not replay this work
automatically; a second `run` call is a new mutation, not a retry of the old
Task.

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
| Create an absent empty infobase | `infobase.create`, empty args; then send sources separately; no sync baseline |
| Send sources / delete an extension | `push`, `force:true`, optional `sourceSet` and `full`; applies the database configuration. Deletion uses only `delete: "InstalledName"` |
| Replace one source set from the working configuration | `pull`, `force:true`, optional `sourceSet`, `extension`; no local-work protection |
| Export the configuration or an extension as `.cf`/`.cfe` | `download`, `state=working` or `state=database`, `output`, optional `extension` |
| Load a `.cf`/`.cfe` into the working configuration only | `upload`, `input`, optional `extension`; loading does not apply the database configuration |
| Build a `.cf`/`.cfe` from sources | `make`, `output`, optional `sourceSet`, `extension`; `.epf`/`.erf` are not published |
| Export the whole infobase as `.dt` | `infobase.dump`, `output` |
| Load a `.dt` | `infobase.restore`, `input`, `mode=create` or `mode=replace` |
| Launch a 1C client | `launch`, `clientMode`, optional `execute`, `waitForExit`, `waitTimeoutMs`; terminal, no preview required |
| Inspect installed extensions | `extensions.list`, empty args; preview/apply opens a platform session |
| Change installed extension activity | `extensions.set`, `name`, boolean `active`; other properties are unavailable |
| Apply or discard pending configuration changes | `apply`, optional `extension`; `reset`, `force:true`, optional `extension`; Designer only |

A previewApply operation is applied with the `ifRev` its preview returned; a
changed workspace or plan answers `stale_revision` or `concurrent_change`
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
