# v8project.yaml Contract

`v8project.yaml` is the only project configuration format used by Unica skills.
Unica reads it from the workspace root together with `v8project.local.yaml`;
there is no argument that points a call at another file.

Runtime идёт через `unica.run`: вызов без `op` отдаёт словарь операций и
контракт каждой — `argsSchema`, `execution`, `previewRequired`,
`ifRevRequiredOnApply`. Контракт вызова бери оттуда, а не из этого текста;
при `implemented: true` используй опубликованную `argsSchema`; при
`support.state: limited` разрешено только подмножество `support.supportedArgs`.
При `support.state: unavailable` остановись; не выдумывай аргументов при
`argsSchema: null`. Превью исполнением не является. Не обходи контракт прямым runner-ом.

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

```yaml
workPath: 'build'
execution_timeout: 300000
format: DESIGNER
infobases:
  origin:
    connection: 'File=build/ib'
source-set:
  - name: main
    type: CONFIGURATION
    path: 'src'
push:
  partialLoadThreshold: 20
```

`infobases.origin.connection` is the target form. Put machine addresses and
credentials in `v8project.local.yaml`; the primary example is only a minimal
local project. The runner 0.11 adapter privately projects this form into its
legacy config and removes the temporary files after execution. It supports
only `origin`; multiple named bases, raw connection argv and mixed old/new
infobase keys are refused. `unica.run` accepts optional top-level
`infobase: "origin"`; another target is never silently redirected to origin.
Legacy `infobase.connection` remains readable during migration. Do not use
legacy top-level `connection` in `v8project.yaml`.

`basePath` is also removed from the pinned v8-runner contract. Relative
`workPath`, infobase file paths, and source-set paths are resolved from the
directory containing the primary config.

`execution_timeout` is the v8-runner operation budget in milliseconds. The
default is `300000`; v8-runner validates the value in the `1..=86400000` range.
For a `unica.run` operation this project config value is the runner budget;
Unica adds no timeout argument of its own.

Server infobase connections use the normal 1C connection string form in
`infobases.origin.connection`, for example `Srvr="srv01";Ref="dev";`. IBCMD server
connections also require the documented `infobases.origin.dbms` block.

`v8project.local.yaml` is loaded automatically next to the primary config. It
may override local-only `workPath`, `infobases` (or legacy `infobase`), `providers`, `tools`, `tests`, and `mcp`
settings. It is not selectable by a call and must not redefine shared
`source-set`, `format`, or `execution_timeout`.

## Runner 0.11 migration

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
| Create an infobase with its source/sync baseline | `infobase.create` — unavailable with runner 0.11 |
| Send sources / delete an extension | `push` — only `args: {"delete": "InstalledName"}` is supported with runner 0.11; sending sources is unavailable |
| Bring database changes into sources with local-work protection | `pull` — unavailable with runner 0.11 |
| Export the configuration or an extension as `.cf`/`.cfe` | `download`, `state=working` or `state=database`, `output`, optional `extension` |
| Load a `.cf`/`.cfe` into the working configuration only | `upload` — unavailable: runner 0.11 load also applies the database configuration |
| Build a `.cf`/`.cfe` from sources | `make`, `output`, optional `sourceSet`, `extension`; `.epf`/`.erf` are not published |
| Export the whole infobase as `.dt` | `infobase.dump`, `output` |
| Load a `.dt` | `infobase.restore`, `input`, `mode=create` or `mode=replace` |
| Launch a 1C client | `launch`, `clientMode`, optional `execute`, `waitForExit`, `waitTimeoutMs`; terminal, no preview required |
| Inspect installed extensions | `extensions.list`, empty args; preview/apply opens a platform session |
| Change installed extension activity | `extensions.set`, `name`, boolean `active`; other properties are unavailable |
| Apply or discard pending configuration changes | `apply`, `reset` — unavailable with runner 0.11 |

Syntax checks are `unica.check`; test runs and Designer/EDT conversion are not
operations of the dictionary. A previewApply operation is applied with the
`ifRev` its preview returned; a changed workspace or plan answers
`stale_revision` or `concurrent_change` instead of applying. The remaining gap in binding preview to its inputs is tracked in
[issue #950](https://github.com/IngvarConsulting/unica/issues/950).

The runner 0.11 adapter does not expose its old source dump/build paths as
successful `pull`/`push`: local-work protection, generation checks and the
separate apply step must be verified before those modes become available.
`unica.apply` edits source files; `unica.run` with `op: "apply"` means a future
platform database-configuration update and is currently unavailable.

## Skill Rules

- Do not create or read any legacy JSON project registry.
- The active config is `./v8project.yaml` at the workspace root; no `unica` call takes a `config` argument.
- If the config is missing, read the recommended content from `setup` in
  `unica.view {}` and write `v8project.yaml` yourself: no tool creates it.
- Prefer `source-set` names over ad hoc source directories.
- Treat a platform-generated CDFI sidecar `ConfigDumpInfo.xml` whose root is `ConfigDumpInfo` as local per-infobase runtime state: keep it out of Git and never use it as source-format evidence. A legitimate metadata descriptor (including an external EPF/ERF descriptor) for an object actually named `ConfigDumpInfo` remains source and belongs in Git.
- `execution_timeout` in `v8project.yaml` is the runner budget for `unica.run`
  operations; Unica exposes no `timeoutMs` argument.
- `upload` is unavailable: the old runner implicitly applies the database configuration. Do not substitute it with another operation.
- Designer/EDT conversion is not on the surface: Unica reads platform XML only.
- Designer `rawKeys` are not on the surface; source moves are named `push` and `pull`, but are unavailable with the runner 0.11 adapter.
- When credentials are absent, do not initiate a runtime probe to discover them. Ask the user; classify only authentication evidence already supplied by a verified boundary.
- If a command reports a 1C license problem, stop and ask the user to fix licensing. Do not edit license services, HASP settings, registry, or license files.
- If a runtime flag or debug-server step is missing from the `unica.run`
  dictionary, treat it as a Unica MCP contract gap. `.epf`/`.erf` publication
  is one such gap: `make` builds `.cf` and `.cfe` only.
