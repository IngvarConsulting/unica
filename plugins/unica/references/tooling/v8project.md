# v8project.yaml Contract

`v8project.yaml` is the only project configuration format used by Unica skills.
Unica reads it from the workspace root together with `v8project.local.yaml`;
there is no argument that points a call at another file.

Runtime идёт через `unica.run`: вызов без `op` отдаёт словарь операций и
контракт каждой — `argsSchema`, `execution`, `previewRequired`,
`dryRunRequired`. Контракт вызова бери оттуда, а не из этого текста;
при `implemented: true` используй опубликованную `argsSchema`; при
`support.state: limited` разрешено только подмножество `support.supportedArgs`.
При `support.state: unavailable` остановись; не выдумывай аргументов при
`argsSchema: null`. Для плановой операции сначала проверь результат `dryRun: true`,
затем исполняй запрос с `dryRun: false`. Preview не фиксирует входы между
вызовами. Не обходи контракт прямым runner-ом.

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
workPath: 'build'
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
attached to is known to the machine, not to the project. v8-runner 0.14 reads
the infobase map only from the local layer. An `infobases` section (or the
legacy `infobase` key) in `v8project.yaml` is still accepted: the adapter
moves it into a private copy of the local layer, where fields of the local
file override it, and keeps that copy in `.build/unica/runner-project/` of the
working copy (the runner records this directory as the holder of a file infobase). Only
`origin` is supported; multiple named bases, raw connection argv and `infobase`
mixed with `infobases` in one file are refused. `unica.run` accepts optional
top-level `infobase: "origin"`; another target is never silently redirected to
origin. Do not use legacy top-level `connection` in `v8project.yaml`.

`basePath` is not part of the pinned v8-runner contract, and `unica.run`
refuses a config that sets it. Relative `workPath`, infobase file paths, and
source-set paths are resolved from the directory containing the primary config.

`push.partialLoadThreshold` (and `build.partialLoadThreshold`) is not supported:
v8-runner 0.14 has no partial-load threshold and chooses the mode itself; request
a full load with `push` `full:true`. Unica refuses a config with this key; remove it.

A file infobase is held by one working copy. The runner records the holder next
to the infobase directory. A write from another working copy is not refused: it
changes that copy's infobase, and the runner warns about it. Give each working
copy its own infobase (`infobase.create`).

`execution_timeout` is not supported. v8-runner 0.14 has no overall command
deadline: a command runs to its terminal outcome, and limits belong to the
steps that need them (`tools.edt_cli.command_timeout_ms`,
`tests.execution_timeout_seconds`, `tools.client_mcp.wait_ready_timeout_ms`).
Unica refuses to run with `execution_timeout` in either file and adds no
timeout of its own; remove the key.

Server infobase connections use the normal 1C connection string form in
`infobases.origin.connection`, for example `Srvr="srv01";Ref="dev";`. IBCMD server
connections also require the documented `infobases.origin.dbms` block.

`v8project.local.yaml` is loaded automatically next to the primary config. It
may override local-only `workPath`, `infobases` (or legacy `infobase`), `providers`, `tools`, `tests`, and `mcp`
settings. It is not selectable by a call and must not redefine shared
`source-set` or `format`.

## Runner migration

The top-level `builder` key is rejected. Remove it to use the runner's per-operation
provider defaults. If the project requires a particular executor, declare it in
`providers`, for example `providers: {download: designer, extensions: ibcmd}`.
Names are lowercase (`designer`, `agent`, `ibcmd`); the runner validates each
operation/provider pair. Do not mechanically replace `builder` with one global
provider: each operation has its own supported executors. Keep machine-specific
overrides in `v8project.local.yaml`. Unica does not rewrite existing project files.

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

Format discovery remains per source-set, but `unica.epf.init` and
`unica.erf.init` specifically require the global `format` value to be exact
`DESIGNER` or omitted. v8-runner selects the external-project layout from that
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
| Replace one source set from the working configuration | `pull`, `force:true`, optional `sourceSet`, `extension`; no local-work protection |
| Export the configuration or an extension as `.cf`/`.cfe` | `download`, `state=working` or `state=database`, `output`, optional `extension` |
| Load a `.cf`/`.cfe` into the working configuration only | `upload` — unavailable: v8-runner 0.14 has no load without applying the database configuration ([#1246](https://github.com/IngvarConsulting/unica/issues/1246)) |
| Build a `.cf`/`.cfe` from sources | `make`, `output`, optional `sourceSet`, `extension`; `.epf`/`.erf` are not published |
| Export the whole infobase as `.dt` | `infobase.dump`, `output` |
| Load a `.dt` | `infobase.restore`, `input`, `mode=create` or `mode=replace` |
| Launch a 1C client | `launch`, `clientMode`, optional `execute`; `waitForExit` with `waitTimeoutMs` supports `thin` + `.epf`; terminal, no preview required |
| Inspect installed extensions | `extensions.list`, empty args; preview/apply opens a platform session |
| Change installed extension activity | `extensions.set`, `name`, boolean `active`; other properties are unavailable |
| Apply or discard pending configuration changes | `apply` and `reset` — unavailable until the runner supports them ([#1246](https://github.com/IngvarConsulting/unica/issues/1246)) |

Syntax checks are `unica.check`; test runs and Designer/EDT conversion are not
operations of the dictionary. A previewApply operation requires explicit boolean
`dryRun`: `true` returns a non-mutating plan; `false` executes using the current
inputs, without requiring a previous preview. `unica.run` accepts no `ifRev`
and returns no `rev`. In this workflow, inspect the preview before execution.
Preview does not freeze the workspace or the database between calls.

Source `push` checks the infobase generation before loading a set and is refused with `non_fast_forward` when the infobase moved ahead of this working copy's record, or with `no_memory` when this working copy has no memory of the infobase; the refusal offers a `pull` preview and a `push` preview with `force:true`. `force:true` overwrites the infobase: every selected set is loaded in full without that check, and changes made in the infobase are lost. A set the runner skips by its memory is neither loaded nor checked. Full pulling requires explicit `force:true` and provides no local-work protection. Source
`push` also applies the database configuration; `noApply:true` is unavailable.
With `force:true`, `pull` discards uncommitted and untracked files of the
source set without a copy. `upload`, `apply` and `reset` are unavailable until
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
- Do not write `execution_timeout` or `push.partialLoadThreshold`: v8-runner 0.13 has neither an overall command
  deadline nor a partial-load threshold, and Unica refuses a config with either key. Unica exposes no `timeoutMs` argument.
- `upload`, `apply` and `reset` are unavailable: report loading a CF/CFE as a Unica MCP contract gap ([#1246](https://github.com/IngvarConsulting/unica/issues/1246)) and do not call the runner directly.
- Designer/EDT conversion is not on the surface: Unica reads platform XML only.
- Designer `rawKeys` are not on the surface. Source `push` checks the infobase generation unless `force:true` is given; never add `force:true` on your own after a `non_fast_forward` or `no_memory` refusal: choosing between pulling the infobase and overwriting it belongs to the user. `pull` requires explicit `force:true` and does not protect local work.
- When credentials are absent, do not initiate a runtime probe to discover them. Ask the user; classify only authentication evidence already supplied by a verified boundary.
- If a command reports a 1C license problem, stop and ask the user to fix licensing. Do not edit license services, HASP settings, registry, or license files.
- If a runtime flag or debug-server step is missing from the `unica.run`
  dictionary, treat it as a Unica MCP contract gap. `.epf`/`.erf` publication
  is one such gap: `make` builds `.cf` and `.cfe` only.
