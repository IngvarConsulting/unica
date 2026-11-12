# v8project.yaml Contract

`v8project.yaml` is the only project configuration format used by Unica skills.
Unica reads it from the workspace root together with `v8project.local.yaml`;
there is no argument that points a call at another file.

Runtime идёт через `unica.run`: вызов без `op` отдаёт словарь операций и
контракт каждой — `argsSchema`, `execution`, `previewRequired`,
`dryRunRequired`. Контракт вызова бери оттуда, а не из этого текста;
используй опубликованную `argsSchema`; при `support.state: limited`
разрешено только подмножество `support.supportedArgs`. Недоступной операции
в словаре нет. Для плановой операции сначала проверь результат
`dryRun: true`, затем исполняй запрос с `dryRun: false`. Preview
не фиксирует входы между вызовами. Не обходи контракт прямым runner-ом.

For a new repository with no workspace, call `unica.view {}` first. Оно
работает и без проектного файла: отвечает `config.state: "autodetected"`,
перечисляет найденные наборы и несёт в `setup` рекомендуемое содержимое
`v8project.yaml`.

**Файл заводит человек или модель своими файловыми средствами.** Инструмента
записи `v8project.yaml` в продукте нет: операция `workspace.initialize` снята,
и в словаре `run` её больше не числится. Возьми содержимое из `setup`, запиши
файл и спроси `unica.check {}` о готовности.

Остальной рантайм-контракт открывается через `unica.run {}`. Не выдумывай
аргументы операции, у которой `argsSchema` равен `null`.

## Minimal Shape

`v8project.yaml`:

```yaml
format: DESIGNER
source-set:
  - name: main
    type: CONFIGURATION
    path: 'src'
```

`v8project.local.yaml`:

```yaml
infobases:
  origin:
    connection: 'File=build/ib'
```

The infobase belongs in `v8project.local.yaml`: which infobase a checkout is
attached to is known to the machine, not to the project. The runner reads both
files in place and refuses an `infobases` section in `v8project.yaml`; Unica
passes its refusal on as is. `unica.run` accepts optional top-level
`infobase: "origin"`; another target is never silently redirected to origin.
Do not use legacy top-level `connection` in `v8project.yaml`.

Relative `workPath`, infobase file paths, and source-set paths are resolved
from the directory containing the primary config. Without `workPath` the
runner works in `.build/v8-runner`. A full `pull` stages the export in
`workPath` and publishes it by a rename, so `workPath` and the source sets must
be on one volume; otherwise the runner refuses with
`work_path_on_another_volume`. A runner command that finds the working copy
held by another one refuses with `workspace_busy`; repeat the call when that
command ends. Unica passes both refusals with the runner's text.

The runner refuses keys it does not support, such as `basePath`,
`execution_timeout` and `push.partialLoadThreshold`, and its refusal names the
key. A command has no overall deadline: limits belong to the steps that need
them (`tools.edt_cli.command_timeout_ms`, `tests.execution_timeout_seconds`,
`tools.client_mcp.wait_ready_timeout_ms`), and Unica adds no timeout of its
own. A full load is requested with `push` `full:true`.

Server infobase connections use the normal 1C connection string form in
`infobases.origin.connection`, for example `Srvr="srv01";Ref="dev";`. IBCMD server
connections also require the documented `infobases.origin.dbms` block.

`v8project.local.yaml` is loaded automatically next to the primary config. It
may override local-only `workPath`, `infobases` (or legacy `infobase`), `providers`, `tools`, `tests`, and `mcp`
settings. It is not selectable by a call and must not redefine shared
`source-set` or `format`.

## File infobase holder

A file infobase is held by one working copy. The runner records the holder next
to the infobase directory. A write from another working copy is not refused: it
changes that copy's infobase, and the answer warns
`infobase_of_another_copy`. Give each working copy its own infobase
(`infobase.create`).

The holder is the working copy of the config the runner read. Unica starts the
runner with `--config` on the workspace `v8project.yaml`, so Unica and a person
running the runner by hand in the same working copy are one holder.

## Live Designer agent

The runner keeps the Designer agent of a working copy alive between its
commands, so a command does not start the Designer anew or take another license
session. The answer of a runner operation whose command used that agent carries
`data.agent`:

- `reused`: the command took an agent already running;
- `kept`: the agent stays after the command;
- `reason`: why it did not stay, in the runner's word (`limit`,
  `live_agent_off`, `job` and others);
- `holders`: how many processes hold it;
- `heldByUnica`: Unica holds it.

Unica holds the agent after a command it stayed after. Unica lets it go after
10 minutes without runner commands and without editor sessions of the working
copy, when the last editor session of the working copy closes, or to make room
for a third working copy: Unica holds the agents of two working copies at most,
the runner's default `tools.designer_agent.live-max`. An editor session does not
start an agent. When the limit left a command without an agent, the answer warns
`live_agent_not_held`. An agent switched off (`tools.designer_agent.live:
false`) or impossible in the Windows Job Unica was started in is named once in
`data.agent.note`.

A live agent stays connected to its infobase between commands, as the runner
keeps it, and so locks a file infobase for configuring: while Unica holds the
agent, a batch update or restore of the infobase in the Designer is refused with
a lock error. To free the infobase, run `v8-runner agent release` in the working
copy: it disconnects the agent, and the next runner command connects it again.
Otherwise Unica lets the agent go after 10 minutes without runner commands and
without editor sessions of the working copy.

## Runner migration

The top-level `builder` key is rejected. Remove it to use the runner's per-operation
provider defaults. If the project requires a particular executor, declare it in
`providers`, for example `providers: {download: designer, extensions: ibcmd}`.
Names are lowercase (`designer`, `agent`, `ibcmd`); the runner validates each
operation/provider pair. Do not mechanically replace `builder` with one global
provider: each operation has its own supported executors. Keep machine-specific
overrides in `v8project.local.yaml`. Unica does not rewrite existing project files;
the only key it adds is `tools.platform.version` (see
[strict platform resolution](#strict-platform-resolution)).

The pinned runner names keys as follows: the infobase is `infobases.origin` in
`v8project.local.yaml`; the load settings are `push:`; executors are flat
`providers` keys (`push`, `pull`, `apply`, `reset`, `upload`, `download`,
`diff`, `make`, `convert`, `extensions`, `infobase.create`, `infobase.dump`,
`infobase.restore`); source-set types are `CONFIGURATION`, `EXTENSION`,
`EXTERNAL_DATA_PROCESSORS` and `EXTERNAL_REPORTS`. The former spellings
(`infobase:`, `build:`, `providers.init`/`build`/`load`/`dump`/
`infobase.configuration.export`) are accepted for one release cycle; where the
runner warns about a former key, Unica passes its warning on as is. Rename the
keys.

## Headless source export

For Designer XML, select `ibcmd` for `pull` in `v8project.local.yaml`:

```yaml
providers:
  pull: ibcmd
```

This selects `ibcmd config export` for the main configuration and for an
installed extension. It does not start the Designer agent, so no GTK/X display
or wrapper forcing `GDK_BACKEND` is needed for that export. A local 1C 8.3.27
installation with `ibcmd` must be available and verifiable; configure
`tools.platform` as described below when discovery is ambiguous.

Use `unica.run` to preview and execute `pull`; see
[workspace runtime](../use-cases/workspace-runtime.md#headless-pull).
The Unica adapter requires `force:true` and replaces the whole source set,
including untracked files. It does not expose incremental `--sync` export.
The `pull` provider does not choose the executor of `push`.

## Strict platform resolution

Use `tools.platform.strict` when one machine must fail closed on an exact 1C
installation instead of accepting another discovered platform version. A
machine-local path normally belongs in `v8project.local.yaml`:

```yaml
tools:
  platform:
    version: "8.3.27.1859"
    path: "C:\\Program Files\\1cv8\\8.3.27.1859\\bin"
    strict: true
```

`path` is always an explicit-only search boundary. With both `path` and
`strict: true`, the configured `version` is enforced fail-closed: a missing
utility, unknown version, or incompatible version is an error. The first
resolved platform utility fixes one canonical installation root, and sibling
`1cv8`, `1cv8c`, and `ibcmd` are selected only from that root.

With `path` and omitted/false `strict`, the runner still stays inside `path`,
but it ignores `version` for that boundary. With no `path`, omitted/false
`strict` preserves legacy discovery through the normal roots and `PATH`;
`strict: true` alone creates no boundary. This project config field is not an
argument of `unica.run`.

When neither layer sets `tools.platform.version`, a `DESIGNER` project with a
`CONFIGURATION` source-set runs on the platform line of its source format: the
newest installation of the line that writes the format of `Configuration.xml`.
Before each runner operation, previews included, Unica writes
`tools.platform.version: sources` into `v8project.local.yaml`, creating the
file when it is absent and keeping the rest of it and its permissions; the
answer of that call says so with the warning `platform_line_written`. When the key cannot be
placed into the text, the file is written anew with the same keys and values
but without comments, and the warning says so. A read-only file is not
written: the warning `platform_line_not_written` says that the runner chooses
the platform without the line of the source format until the key is set. It
also removes `.build/unica/runner-project/` (with `UNICA_CACHE_DIR`,
`runner-project/<key>` there), the private config copy of Unica versions
before runner 0.17: the runner recorded it as the holder of a file
infobase, and once it is gone the next writing command records the
working-copy root instead. A failed removal is a warning of the answer. A version set in either file is left as it is. Without
an installation of that line the command is refused with
`platformLineNotInstalled`, which names the line, the source format and the
versions of the installations found. Install that line, or set
`tools.platform.version` to choose a platform explicitly. Before the first
`pull` there is no `Configuration.xml` and no restriction.

## Source-set format discovery

Use MCP `unica.view {}` to inspect configured source-sets before choosing a
metadata operation. It returns `sourceSets[]` where each entry has `kind`,
`path`, `sourceFormat`, and `formatEvidence`.

The top-level `format` field is a default/effective format, not proof that every
source-set under the workspace has the same layout. A project can contain an EDT
configuration source-set and platform XML external processor/report source-sets.
Within one source-set the format cannot be mixed: conflicting platform XML and
EDT markers mean the source-set is invalid/ambiguous and must be fixed or
converted before XML metadata tools are used.

Format discovery remains per source-set, but external processor and report
scaffolds written by the `epf-init` and `erf-init` skills specifically require
the global `format` value to be exact `DESIGNER` or omitted. v8-runner selects the external-project layout from that
global value; use a separate Designer workspace/config when the active config
has global `format: EDT`.

## Autodetected source-sets

A workspace without `v8project.yaml` still gets a source map. Autodetection
looks only in a closed catalog of layouts and never competes with the file: one declared
source-set replaces autodetection entirely.

| Layout | Source-set |
| --- | --- |
| `.`, `src` or `src/cf` carrying a configuration marker | `main`, kind `configuration` — first match wins |
| `src/cfe` carrying a marker itself | `cfe`, kind `extension` — its children are that extension's objects, not siblings |
| `src/cfe/<name>` | `<name>`, kind `extension` |
| `src/extensions/<Name>` | `<Name>`, kind `extension` |

A marker is `Configuration.xml`, `Configuration/Configuration.mdo` or
`src/Configuration/Configuration.mdo`, in every layout alike.

An autodetected source-set is named after the directory holding it, verbatim. A
container may hold other things — `.gitkeep`, `README.md`, a symlink — and those
are skipped, not reported and not treated as an error. The same holds for the
container path itself: absent, a plain file or a symlink all mean "no extensions
in this layout", while a container that could not be read at all (permissions) is
reported rather than silently reported as empty. `main` stays with the base
configuration while it exists; when nothing else claims the name, an extension
directory named `main` keeps it.

## Command Mapping

Именами операций, их состоянием и схемами аргументов на проводе v0.13 отвечает
только `unica.run {}`. Таблица ниже — карта прежних намерений на операции
словаря; значения из неё в вызов не передаются, контракт бери из словаря.
Создания проектного файла в ней нет — наследника у него нет ни в одном
инструменте.

| Intent | `unica.run` operation |
| --- | --- |
| Create an absent infobase | `infobase.create`, empty args; a file infobase is created with the main configuration of the `CONFIGURATION` source set (`initializesSources: true`), and the first `push` loads that set only if it changed and the other sets in full; a cluster infobase is created empty, and the first `push` loads every set in full |
| Send sources / delete an extension | `push`, optional `sourceSet`, `full` and `force`; applies the database configuration after the generation check, `force:true` overwrites the infobase. Deletion uses only `delete: "InstalledName"` |
| Export one source set from the working configuration | `pull`, optional `sourceSet`, `extension` and `force`; without `force` the runner chooses a partial or full export, `force:true` replaces the whole set; no copy of the previous sources is kept |
| Re-export one source set in a writable export format | `source.reexport`, `sourceSet` and `format` (`2.20`, `2.21` or `2.22`); the set is loaded into a temporary file infobase with the platform line of the format (8.3.27, 8.5.1, 8.5.4) and exported back in full, replacing the files of the set; the project infobase and files stay unchanged. An edit refused for an older or mixed export format asks the person to re-export |
| Export the configuration or an extension as `.cf`/`.cfe` | `download`, `state=working` or `state=database`, `output`, optional `extension` |
| Load a `.cf`/`.cfe` into the working configuration only | `upload` — unavailable: the pinned v8-runner upload still applies the database configuration, and the adapter does not run `apply` or `reset` yet ([#1246](https://github.com/IngvarConsulting/unica/issues/1246)) |
| Build a `.cf`/`.cfe`/`.epf`/`.erf` from sources | `make`, `output`; for CF/CFE optional `sourceSet` and `extension` (CFE); EPF/ERF require `sourceSet` naming a singleton matching Designer external source set and a declared main configuration |
| Export the whole infobase as `.dt` | `infobase.dump`, `output` |
| Load a `.dt` | `infobase.restore`, `input`, `mode=create` or `mode=replace` |
| Launch a 1C client | `launch`, `clientMode`, optional `execute` and `applicationParameter`; `waitForExit` with `waitTimeoutMs` supports `thin` + `.epf` or an explicit application parameter; terminal, no preview required |
| Inspect installed extensions | `extensions.list`, empty args; preview/apply opens a platform session |
| Change installed extension activity | `extensions.set`, `name`, boolean `active`; other properties are unavailable |
| Apply or discard pending configuration changes | `apply` and `reset` — unavailable until the runner supports them ([#1246](https://github.com/IngvarConsulting/unica/issues/1246)) |

Syntax checks are `unica.check`; test runs and Designer/EDT conversion are not
operations of the dictionary. A previewApply operation requires explicit boolean
`dryRun`: `true` returns a non-mutating plan; `false` executes using the current
inputs, without requiring a previous preview. `unica.run` accepts no `ifRev`
and returns no `rev`. In this workflow, inspect the preview before execution.
Preview does not freeze the workspace or the database between calls.

Source `push` checks the infobase generation before loading a set and is refused with `non_fast_forward` when the infobase moved ahead of this working copy's record, or with `untracked_base` when this working copy has no memory of the infobase. The runner does not carry over its memory across an upgrade, so the first `push` of every working copy after one gets `untracked_base`; the next step is one full `pull` of the set, then `push` as usual. The refusal offers a `pull` preview and a `push` preview with `force:true`; overwrite the infobase only if the sources, not the infobase, hold the right state. `force:true` overwrites the infobase: every selected set is loaded in full without that check, and changes made in the infobase are lost. A set the runner skips by its memory is neither loaded nor checked. Without `force`, `pull` lets the runner choose a partial or full export by its memory of earlier exchanges; `force:true` exports the whole set. Source
`push` also applies the database configuration; `noApply:true` is unavailable.
With `force:true`, `pull` discards uncommitted and untracked files of the
source set without a copy. Unica keeps no copy of the previous sources: a failed
`pull` may leave a partial export, and the answer offers to repeat it with
`force:true`. `upload`, `apply` and `reset` are unavailable until
the runner can load without applying and apply separately
([#1246](https://github.com/IngvarConsulting/unica/issues/1246)).
`unica.apply` edits source files and is unrelated to the runtime `apply`.

## Skill Rules

- Do not create or read any legacy JSON project registry.
- The active config is `./v8project.yaml` at the workspace root; no `unica` call takes a `config` argument.
- If the config is missing, read the recommended content from `setup` in
  `unica.view {}` and write `v8project.yaml` yourself: no tool creates it.
- Prefer `source-set` names over ad hoc source directories.
- Treat a platform-generated CDFI sidecar `ConfigDumpInfo.xml` whose root is `ConfigDumpInfo` as local per-infobase runtime state: keep it out of Git and never use it as source-format evidence. A legitimate metadata descriptor (including an external EPF/ERF descriptor) for an object actually named `ConfigDumpInfo` remains source and belongs in Git.
- Do not write `execution_timeout` or `push.partialLoadThreshold`: the runner has neither an overall command
  deadline nor a partial-load threshold and refuses a config with either key. Unica exposes no `timeoutMs` argument.
- `upload`, `apply` and `reset` are unavailable: report loading a CF/CFE as a Unica MCP contract gap ([#1246](https://github.com/IngvarConsulting/unica/issues/1246)) and do not call the runner directly.
- Designer/EDT conversion is not on the surface: Unica reads platform XML only.
- Designer `rawKeys` are not on the surface. Source `push` checks the infobase generation unless `force:true` is given; never add `force:true` on your own after a `non_fast_forward` or `untracked_base` refusal: choosing between pulling the infobase and overwriting it belongs to the user. `pull` requires explicit `force:true` and does not protect local work.
- When credentials are absent, do not initiate a runtime probe to discover them. Ask the user; classify only authentication evidence already supplied by a verified boundary.
- If a command reports a 1C license problem, stop and ask the user to fix licensing. Do not edit license services, HASP settings, registry, or license files.
- If a runtime flag or debug-server step is missing from the `unica.run`
  dictionary, treat it as a Unica MCP contract gap.
