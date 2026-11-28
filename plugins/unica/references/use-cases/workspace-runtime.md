# Workspace And Runtime Workflows

## When to use

Use this when the user needs a new workspace, `v8project.yaml`, infobase
creation, source import/export, CF/CFE load or export, artifact build, `.dt` export or
load, or a 1C client launch.

Do not use this for point edits inside XML metadata. Use the object-specific
skills for configuration roots, metadata objects, forms, DCS, MXL, roles,
subsystems, interfaces, and templates.

## Primary path

Use the package-selected MCP runtime surface directly. In v0.13, call
`unica.run {}` first and select only an operation the dictionary lists; for
`support.state: limited` use exactly the subset in `support.supportedArgs`.
An unavailable operation is not listed and its call is refused.

Runtime идёт через `unica.run`: вызов без `op` отдаёт словарь операций и
контракт каждой — `argsSchema`, `execution`, `previewRequired`,
`dryRunRequired`. Контракт вызова бери оттуда, а не из этого текста;
используй опубликованную `argsSchema`; при `support.state: limited`
разрешено только подмножество `support.supportedArgs`. Недоступной операции
в словаре нет. Для плановой операции сначала проверь результат
`dryRun: true`, затем исполняй запрос с `dryRun: false`. Preview
не фиксирует входы между вызовами. Не обходи контракт прямым runner-ом.

Source `push` checks the infobase generation before loading a set and is refused with `non_fast_forward` when the infobase moved ahead of this working copy's record, or with `untracked_base` when this working copy has no memory of the infobase. The runner does not carry over its memory across an upgrade, so the first `push` of every working copy after one gets `untracked_base`; the next step is one full `pull` of the set, then `push` as usual. The refusal offers a `pull` preview and a `push` preview with `force:true`; overwrite the infobase only if the sources, not the infobase, hold the right state. `force:true` overwrites the infobase: every selected set is loaded in full without that check, and changes made in the infobase are lost. A set the runner skips by its memory is neither loaded nor checked. Full pulling requires explicit `force:true` and provides no local-work protection; inspect the preview before execution.
For source readiness independently of runtime availability, first
call `unica.check {}`. It returns `status`, `ready`, `repositoryReady`,
`checks[]` and `diagnostics[]` — the verdict on the workspace. Every page
repeats the whole verdict, `checks[]` and `diagnosticCount`; diagnostic groups
arrive in pages of up to 20 (`limit` up to 50, about 64 KiB). When the answer
has a `cursor`, call the `unica.check` continuation from `next` until
`page.stoppedBy` is `complete`: it reads the saved verdict and does not inspect
again. The facts it
judges live in `unica.view {}`: `sourceSets` (possibly an empty array),
`config`, `infobase` and the recommended `v8project.yaml` content. The split is
the same as on a node, where `view {at}` gives `props` and `check {at}` gives
`status`. A false `ready` blocks the source operation
until its source-set problem is fixed. In particular, `sourceSet.path: .` is an
error: explain how to move the export into a strict child such as `src/` and
update `v8project.yaml` safely.

### Headless pull

For Designer XML export without the Designer agent, set
`providers: {pull: ibcmd}` in `v8project.local.yaml`; the local platform must
provide a verifiable `ibcmd` from 1C 8.3.27. The same provider can export the
main configuration or an installed extension.

Read the `unica.run {}` dictionary first. For an extension source set named
`Sales` containing the installed extension `Sales`, preview:

```json
{"op":"pull","args":{"sourceSet":"Sales","extension":"Sales","force":true},"dryRun":true}
```

The source-set name must match `extension`; its directory may have a different
name. Check the returned target and replacement plan, then repeat with
`dryRun:false`. For the main configuration, use its source-set name and omit
`extension`. `force:true` replaces the entire target, including local and
untracked files; this interface does not provide incremental `--sync` export.
A preview does not write sources or read the current extension contents.

### Work the call must not wait for

A quick call returns its result directly. When the daemon hands any operation
to a Task, a call without Tasks receives a receipt with `data.task.taskId`.
Save that ID. Use `unica.task.get` for current state, `unica.task.result` to
wait up to 7000 ms for the result, and `unica.task.cancel` to request
cancellation. A new connection in the same workspace can read the same Task.
Closing the connection or cancelling a wait after handoff leaves the operation
running. A client that declares native Tasks on its call uses the MCP Task
methods. The terminal result never publishes raw stdout. Read the Task state
for the outcome. A `working` state alone does not prove that the runner is
making progress.

If a call's answer is lost, Unica answers with the ordinary Task receipt of
the Task the call may continue as (`data.task.taskId`, status `queued`,
`next` to `unica.task.result`); a client with native Tasks receives that
Task directly. Wait for it instead of calling again. `task_not_found` for
that id means the daemon never took the call or no longer knows it, for
example after a restart: inspect the actual state before repeating `apply`
or `run`. `submit_not_delivered` means
the call never reached the daemon and will not run; repeat it as is.
`provider_limit_exceeded` whose message names the daemon request limit means
the call is larger than the daemon accepts: it was not sent and will not run,
and the message names both sizes. Reduce the call; several smaller calls each
apply separately.
A refused connection (`task_backend_failed` with `needsHuman`) will not pass
on a repeat. An identical read repeated shortly after the first one joins that
work under its own `taskId` only while no source changed since it began;
`apply` and `run` never do, so never repeat them blindly.

### If a mutating runner remains `working`

Keep the `taskId` and the operation's infobase and workspace identity. Read
`tasks/get` (or `unica.task.get`) and use `unica.task.result` only for a bounded
wait without native Tasks. A late `tasks/cancel` is a request, not a
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
Task. A read-only call (`view`, `resolve`, `search`, `check`, `diff`, `docs`)
changed nothing: its direct `outcome_uncertain` refusal says so and carries the
same call in `next`, which is safe to send as a new call. A read-only call that
had become a Task reports `outcome_uncertain` without `next`; it changed
nothing either, so send the original call again.

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
| Check access to the selected infobase | `infobase.check` with empty args. Set `dryRun: false` for a read-only login to `origin`. Success returns the exact version of the Designer client in `data.platformVersion`. Use `dryRun: true` to preview without logging in. |
| Create an absent infobase | `infobase.create`, empty args; a file infobase is created with the main configuration of the `CONFIGURATION` source set (`initializesSources: true`), and the first `push` loads that set only if it changed and the other sets in full; a cluster infobase is created empty, and the first `push` loads every set in full |
| Send sources / delete an extension | `push`, optional `sourceSet`, `full` and `force`; applies the database configuration after the generation check, `force:true` overwrites the infobase. Deletion uses only `delete: "InstalledName"` |
| Export one source set from the working configuration | `pull`, optional `sourceSet`, `extension` and `force`; without `force` the runner chooses a partial or full export, `force:true` replaces the whole set; no copy of the previous sources is kept |
| Re-export one source set in a writable export format | `source.reexport`, `sourceSet` and `format` (`2.20`, `2.21` or `2.22`); the set is loaded into a temporary file infobase with the platform line of the format (8.3.27, 8.5.1, 8.5.4) and exported back in full, replacing the files of the set; the project infobase and files stay unchanged. An edit refused for an older or mixed export format asks the person to re-export |
| Fetch a separate infobase snapshot for comparison | `fetch`, optional `sourceSet` and `extension`; the runner exports the set in full into `.build/unica/fetch/<snapshotId>`. The answer names `snapshotId`, `snapshot`, `sourceFingerprint` and any additional paths in `sourceFingerprintExcludes`, the provider-confirmed `generation` or `null`, and `baseSnapshot` when the previous raw snapshot still matches the runner's exchange baseline. |
| Check or accept a snapshot during a local merge | `fetch.accept` with `snapshotId`; `dryRun:true` checks the current generation. After the client writes the selected local changes, `dryRun:false` records the raw infobase snapshot as the runner's exchange baseline. A compatible runner and a provider-confirmed generation are required. |
| Close a comparison snapshot | `fetch.drop` with `snapshotId` deletes a pending snapshot. The current accepted comparison snapshot for a set is retained. |
| Export the configuration or an extension as `.cf`/`.cfe` | `download`, `state=working` or `state=database`, `output`, optional `extension` |
| Load a `.cf`/`.cfe` into the working configuration only | `upload` — unavailable: the pinned v8-runner upload still applies the database configuration, and the adapter does not run `apply` or `reset` yet ([#1246](https://github.com/IngvarConsulting/unica/issues/1246)) |
| Build a `.cf`/`.cfe`/`.epf`/`.erf` from sources | `make`, `output`; for CF/CFE optional `sourceSet` and `extension` (CFE); EPF/ERF require `sourceSet` naming a singleton matching Designer external source set and a declared main configuration |
| Export the whole infobase as `.dt` | `infobase.dump`, `output` |
| Load a `.dt` | `infobase.restore`, `input`, `mode=create` or `mode=replace` |
| Launch a 1C client | `launch`, `clientMode`, optional `execute` and `applicationParameter`; bounded `waitForExit` with `waitTimeoutMs` supports `thin` + `.epf` or an explicit application parameter; terminal, no preview required |
| Inspect installed extensions | `extensions.list`, empty args; preview/apply opens a platform session |
| Change installed extension activity | `extensions.set`, `name`, boolean `active`; other properties are unavailable |
| Apply or discard pending configuration changes | `apply` and `reset` — unavailable until the runner supports them ([#1246](https://github.com/IngvarConsulting/unica/issues/1246)) |

To check a connection before creating a project, use a temporary workspace
whose private profile declares the candidate as `infobases.origin`. Use the
same call as for a saved project. Keep connection strings and credentials in
the private profile. The MCP arguments are empty.

```json
{"op":"infobase.check","args":{},"dryRun":false}
```

The runner selects Designer from the current platform settings, including an
explicit `tools.platform.version: sources`. The successful login reports the
exact build of the client that connected. A missing version setting stays
absent from the project files. The operation does not export sources. For a classified refusal, read `data.runnerCode`:
`credentials_rejected` identifies a rejected login, `infobase_not_found`
identifies a confirmed missing base, and `infobase_designer_busy` identifies
a conflicting Designer session. A generic failure is insufficient to diagnose
a bad password or a missing base.

For `launch`, `applicationParameter` passes one exact `/C` string to an enterprise
client. Empty is distinct from omitted; NUL and `designer` are rejected. It may
accompany `execute`; when waiting, that file must be `.epf`. Inspect `wait.exitCode`
and `wait.timedOut`: completion alone does not prove an application update succeeded.
The parameter and raw runner command are not echoed in the response. The pinned
runner encodes the value for the 1C command-line parser on every OS, so quotes,
spaces, tabs, line breaks and `/Execute` inside it reach `ПараметрЗапуска` as
exactly one value and never become another launch key; an external processor
opens only through `execute`. Delivery of 23 values, empty and quote-boundary ones
included, is live-verified on macOS 8.3.27.2074; Windows and Linux are not measured live.

A previewApply operation requires explicit boolean `dryRun`: `true` shows the
plan without execution, and `false` executes using the current inputs. The API
does not require a previous preview and accepts no `ifRev`; results contain no
`rev`. In this workflow, inspect a preview before executing an external operation. Syntax checks are `unica.check`; test runs, Designer/EDT
conversion, Designer `rawKeys` and extension property sync are not on the v0.13
surface. Keep a
platform-generated CDFI sidecar out of Git; a legitimate metadata descriptor
(including an external EPF/ERF descriptor) for an object named
`ConfigDumpInfo` remains source.

## Related references

- `../tooling/v8project.md`
- `../tooling/runtime-build.md`
- `autonomous-server-debug.md`
