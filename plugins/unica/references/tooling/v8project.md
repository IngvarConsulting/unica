# v8project.yaml Contract

`v8project.yaml` is the only project configuration format used by Unica skills.
In a preview, use MCP `unica.runtime.execute` argument `config` when the config
file is not located at `./v8project.yaml`.

По INV-MCP-RUNTIME-RECEIPT и ADR-0074: `unica.runtime.execute` с `dryRun: true`
показывает запланированную команду без побочных эффектов, а с `dryRun: false`
исполняет классифицированную операцию и отвечает её терминальным результатом в
том же вызове, приложив названную причину риска (`runtime_risk_*`)
предупреждением; неклассифицированная операция по-прежнему отказывает
`runtime_operation_unbounded` до обнаружения рабочего пространства. Preview
исполнением не является. Работу, которую вызов ждать не должен, запускай через
`unica.runtime.job.start`. Не обходи контракт прямым runner-ом или через
`unica.build.*`.

For a new repository with no workspace, use the `v8-runner` skill first. It
can preview creation of `v8project.yaml` through MCP `unica.runtime.execute`.
It does not create the file, prepare the default `src` source-set, check
database access, or inspect a live license in the current contract.

Preview the config-init arguments through MCP `unica.runtime.execute`:

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.runtime.execute",
    "arguments": {
      "cwd": "<workspace>",
      "operation": "config-init",
      "config": "./v8project.yaml",
      "connection": "<connection-string>",
      "dryRun": true
    }
  }
}
```

## Minimal Shape

```yaml
workPath: 'build'
execution_timeout: 300000
format: DESIGNER
builder: DESIGNER
infobase:
  connection: 'File=build/ib'
source-set:
  - name: main
    type: CONFIGURATION
    path: 'src'
build:
  partialLoadThreshold: 20
```

`infobase.connection` is the current runner key. Do not use legacy top-level
`connection` in `v8project.yaml`.

`basePath` is also removed from the pinned v8-runner contract. Relative
`workPath`, infobase file paths, and source-set paths are resolved from the
directory containing the primary config.

`execution_timeout` is the v8-runner operation budget in milliseconds. The
default is `300000`; v8-runner validates the value in the `1..=86400000` range.
For a future admitted long operation, this project config value is the runner
budget; it is not a reason to attempt an unclassified applied call or add
a Unica wrapper timeout argument.

Server infobase connections use the normal 1C connection string form in
`infobase.connection`, for example `Srvr="srv01";Ref="dev";`. IBCMD server
connections also require the documented `infobase.dbms` block.

`v8project.local.yaml` is loaded automatically next to the primary config. It
may override local-only `workPath`, `infobase`, `tools`, `tests`, and `mcp`
settings. It cannot be passed as `config` and must not redefine shared
`source-set`, `format`, `builder`, or `execution_timeout`.

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
`strict: true` alone creates no boundary. This project config field is not a
new argument of `unica.runtime.execute`.

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

Format discovery remains per source-set, but `unica.epf.init` and
`unica.erf.init` specifically require the global `format` value to be exact
`DESIGNER` or omitted. v8-runner selects the external-project layout from that
global value; use a separate Designer workspace/config when the active config
has global `format: EDT`.

## Autodetected source-sets

A workspace without `v8project.yaml` still gets a source map. Autodetection
looks only in a closed catalog of layouts (ADR-0075,
`INV-SOURCE-AUTODETECT-CATALOG`) and never competes with the file: one declared
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

Use the `v8-runner` skill and MCP `unica.runtime.execute` only for previews of
runtime operation arguments.

| Operation | MCP arguments |
| --- | --- |
| Preview project config creation | `operation=config-init`, `connection=<connection>`, `dryRun=true` |
| Preview infobase/workspace initialization | `operation=init`, `dryRun=true` |
| Preview loading XML sources | `operation=build`, `dryRun=true` |
| Preview a full source load | `operation=build`, `fullRebuild=true`, `dryRun=true` |
| Preview configuration/extension XML dump | synchronous `operation=dump`, `mode=full`, `dryRun=true`; applied post-run validation/publication has no proved receipt bound |
| Preview external source-set dump | `operation=dump`, `mode=full`, `sourceSet=<external>`, `dryRun=true`; the applied run writes without a bounded recovery contract |
| Preview incremental/selected dump | `operation=dump`, `mode=incremental` or `mode=partial`, `dryRun=true`; partial also requires `object=TYPE:NAME` or `objects=[...]` |
| Preview `.cf` / `.cfe` artifact load | `operation=load`, `path=<file>`, `mode=load` or `mode=merge`, `dryRun=true` |
| Preview `.cf` / `.cfe` artifact export | `operation=make`, `output=<file>`, `dryRun=true` |
| Preview 1C launch arguments | `operation=launch`, one of `clientMode=thin`, `clientMode=thick`, `clientMode=designer`, or `clientMode=ordinary`, `dryRun=true` |
| Preview syntax arguments | `operation=syntax`, one of `mode=designer-config`, `mode=designer-modules`, or `mode=edt`, `dryRun=true` |
| Preview test arguments | `operation=test`, one of `testRunner=yaxunit` or `testRunner=va`, `dryRun=true` |
| Preview configured tool download | `operation=tools-download`, one of `tool=yaxunit`, `tool=vanessa`, or `tool=client-mcp`, `dryRun=true` |

A classified applied mode runs and answers with its named risk; a mode the
completion map does not classify still fails closed before workspace discovery
and process spawn. The named risks are non-interruptible phases,
persistent writes without bounded recovery, and unproved ownership of
separately grouped 1C processes. ADR-0016 continues to own the future full-dump
publication contract; its transaction guarantees do not make the current
applied route executable.

On Windows, macOS, and Linux, synchronous full dump (`mode=full`) for DESIGNER
`CONFIGURATION` and `EXTENSION` source-sets runs applied and answers with a named
risk: verified transactional publication still has post-run work without a proved
terminal receipt bound, so a cancelled or timed-out dump has no bounded recovery.

On Windows, Unica attests a local system installation through no-follow handles:
its trusted owner and DACL must prevent mutation of the install tree by the
invoking non-elevated user, while the ancestry must prevent deletion,
replacement, or retargeting of path components. On macOS and Linux, Unica
validates physical DESIGNER markers, attests the exact installation with sibling
`ibcmd --version`, and requires a root-owned, link-free install tree without
group/world write or ACLs. Effective configuration and credentials are never
retained in recovery. User-owned platform installs are rejected before `ibcmd`
or `v8-runner` would execute; other Unix hosts fail closed as well.

## Skill Rules

- Do not create or read any legacy JSON project registry.
- Resolve the active config from the explicit MCP `config` argument when present; otherwise use `./v8project.yaml`.
- If the config is missing, preview `operation=config-init` with `dryRun=true`,
  then ask the user to provide the config; preview cannot create it.
- Prefer `source-set` names over ad hoc source directories.
- Treat a platform-generated CDFI sidecar `ConfigDumpInfo.xml` whose root is `ConfigDumpInfo` as local per-infobase runtime state: keep it out of Git and never use it as source-format evidence. A legitimate metadata descriptor (including an external EPF/ERF descriptor) for an object actually named `ConfigDumpInfo` remains source and belongs in Git.
- `execution_timeout` in `v8project.yaml` describes a future runner-operation
  budget; Unica does not expose `timeoutMs` for `unica.runtime.execute`, and
  changing this value does not admit a current applied operation.
- Do not use `mode=update` for `operation=load`; v8-runner rejects it. Use `mode=load` or `mode=merge` with `settings`.
- Every applied operation carries a named risk; `convert` additionally lacks a
  verified private-stage publication boundary.
- Do not pass `DumpConfigToFiles` or `LoadConfigFromFiles` through Designer `rawKeys`; Unica rejects these unverified source bypasses.
- When credentials are absent, do not initiate a runtime probe to discover them. Ask the user; classify only authentication evidence already supplied by a verified boundary.
- If a command reports a 1C license problem, stop and ask the user to fix licensing. Do not edit license services, HASP settings, registry, or license files.
- If a runtime flag or debug-server step is missing from
  `unica.runtime.execute`, treat it as a Unica MCP contract gap. EPF/ERF
  external-source-set build/dump flows can currently be previewed only with
  `dryRun=true`; neither flow performs runtime work.
