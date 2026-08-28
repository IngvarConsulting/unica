# Unica Plugin

Unica models day-to-day 1C:Enterprise development workflows and exposes one
public stdio MCP server named `unica`. Prompt-visible skills call native
`unica.*` tools; bundled analyzers, runners, indexes, and the standards adapter
remain private implementation details.

One plugin directory serves both Codex and Claude Code. Each host reads its own
manifest, `.codex-plugin/plugin.json` or `.claude-plugin/plugin.json`, and
ignores the other.

## Public installation

Prerequisites are Git and one host: Codex CLI, or Claude Code 2.1.69 or newer.
Node.js, Python, download utilities, and archive utilities are not consumer
dependencies.

```sh
codex plugin marketplace add IngvarConsulting/unica-marketplace --ref main
codex plugin add unica@unica
```

Open a new Codex task after install or update. Update with:

```sh
codex plugin marketplace upgrade unica
codex plugin remove unica@unica
codex plugin add unica@unica
```

On Claude Code the catalog is added without a ref, and skills appear under the
plugin namespace as `/unica:<skill>`:

```sh
claude plugin marketplace add IngvarConsulting/unica-marketplace
claude plugin install unica@unica
```

Claude Code 2.1.68 and earlier reject the catalog's `git-subdir` source type and
cannot load it at all; 2.1.69 is the first release that accepts it.

## Legacy transition boundary

Unica `v0.7.8` is the immutable migration bridge. A local, duplicated, or
otherwise legacy installation must first run the published
[`install-unica.sh`](https://github.com/IngvarConsulting/unica/releases/download/v0.7.8/install-unica.sh)
or
[`install-unica.ps1`](https://github.com/IngvarConsulting/unica/releases/download/v0.7.8/install-unica.ps1).

Unica `v0.8.0` supports ordinary marketplace updates only from canonical
`v0.7.5`, canonical `v0.7.6`, canonical `v0.7.7`, canonical `v0.7.8`, and technical
`0.7.x` installations.
The version string alone does not make a local or duplicated installation
canonical.

Uninstall with:

```sh
codex plugin remove unica@unica
codex plugin marketplace remove unica
```

## DCS naming migration

The release containing [issue #158](https://github.com/IngvarConsulting/unica/issues/158)
atomically replaces the transliterated `skd` domain with the official
**Data Composition System (`dcs`)** term. There is no deprecated alias:

| Removed contract | Canonical contract |
| --- | --- |
| `unica.skd.compile` | `unica.dcs.compile` |
| `unica.skd.edit` | `unica.dcs.edit` |
| `unica.skd.info` | `unica.dcs.info` |
| `unica.skd.validate` | `unica.dcs.validate` |
| `skd-compile/edit/info/validate` | `dcs-compile/edit/info/validate` |

The operation arguments and `DataCompositionSchema` XML format are unchanged.

## Read-only output migration

The release containing [issue #191](https://github.com/IngvarConsulting/unica/issues/191)
removes caller-controlled file sinks from read-only MCP tools. The affected
`info`/`validate` tools no longer accept `OutFile` or `outFile`, and
`unica.mxl.decompile` no longer accepts `OutputPath` or `outputPath`. There is
no compatibility alias: these arguments are rejected as contract errors.

Reports, exact raw DCS queries, and the MXL JSON DSL are returned in the MCP
response. Consumers must read `stdout`/structured response data instead of
reading a file created by Unica. If a durable artifact is needed, the caller
must save the returned value explicitly outside the read-only tool contract.

## Logical source target migration

Tools migrate to one logical target, one merge request at a time. There is no
deprecated alias:

| Tool | Removed selector | Canonical selector |
| --- | --- | --- |
| `unica.code.patch` | `path` + `sourceDir` | `sourceSet` + `metadataPath` |
| `unica.meta.info` | `ObjectPath` / `Path` | `sourceSet` + `metadataPath` |

Calls that still pass a removed field fail with `legacy_target_removed` and
name the canonical replacement. The logical selector addresses existing
Platform XML Configuration and Extension targets; Unica resolves the physical
`*Module.bsl` or descriptor location privately. `unica.meta.info` also stops
accepting `Detailed`, which it never read.

`unica.source.resolve` finds an address by name, and `unica.source.locate`
converts a path discovered by other means into one.

### Readers that accept either selector

Thirteen readers and validators are in the transitional state ADR-0049
defines: they accept the logical selector **and** still accept their existing
path. Nothing is removed here, so no call breaks; removing each path is its own
later merge request.

| Tool | Logical selector | Path kept for now |
| --- | --- | --- |
| `unica.cf.info`, `unica.cf.validate` | `sourceSet` | `ConfigPath` |
| `unica.subsystem.info` | `sourceSet`, optional `metadataPath` | `SubsystemPath` |
| `unica.subsystem.validate` | `sourceSet` + `metadataPath` | `SubsystemPath` |
| `unica.role.info`, `unica.role.validate` | `sourceSet` + `metadataPath` | `RightsPath` |
| `unica.form.info`, `unica.form.validate` | `sourceSet` + `metadataPath` | `FormPath` |
| `unica.dcs.info`, `unica.dcs.validate` | `sourceSet` + `metadataPath` | `TemplatePath` |
| `unica.mxl.info`, `unica.mxl.validate`, `unica.mxl.decompile` | `sourceSet` + `metadataPath` | `TemplatePath` |

Exactly one selector per call. Passing both fails with `selector_conflict`,
because resolving a conflict silently would hide which selector produced the
answer. A configuration root has no address, so `unica.cf.*` takes `sourceSet`
alone and no longer publishes `metadataPath`; `unica.subsystem.info` reads the
whole registered tree when the address is omitted.

An addressed object whose requested body is missing — a template whose
`TemplateType` writes `Template.bin` rather than `Template.xml` — fails with
`resource_absent`, not `target_not_found`: the object exists and is
addressable, that body does not.

## XDTO operations migration

The release containing [issue #374](https://github.com/IngvarConsulting/unica/issues/374)
replaces the flat single-operation form of `unica.xdto.edit` with a typed
ordered `operations` array (ADR-0071). There is no compatibility alias: a call
that still passes any retired top-level field fails with
`legacy_arguments_removed` and names the replacement.

| Removed top-level form | Canonical `operations` element |
| --- | --- |
| `operation: "add-value-type"` + `name`, `base` | `{"op": "addValueType", "name": "Amount", "base": "xs:decimal"}` |
| `operation: "add-object-type"` + `name` | `{"op": "addObjectType", "name": "Order"}` |
| `operation: "add-property"` + `typeName`, `property` [, `propertyPath`] | `{"op": "addProperty", "typeName": "Order", "property": {"name": "Ref", "type": "tns:Document"}}` — optional `propertyPath` targets a nested `typeDef` |
| `operation: "remove-type"` + `name` | `{"op": "removeType", "name": "Order"}` |
| `operation: "remove-property"` + `typeName`, `name` [, `propertyPath`] | `{"op": "removeProperty", "typeName": "Order", "name": "Ref"}` — optional `propertyPath` targets a nested `typeDef` |

Field semantics are unchanged — an element carries exactly the fields the
package writer has always read. Operations in one call apply in order, see
each other's results, and publish once; a failed element leaves no partial
write, and every effect is reported by `operationIndex`.

## Template and help migration

The release containing [issue #375](https://github.com/IngvarConsulting/unica/issues/375)
retires `unica.template.add`, `unica.template.remove` and `unica.help.add`
(ADR-0072). There is no compatibility alias: every call answers
`unknown unica tool`. Template registration and embedded help are operations
of the shared `unica.meta.add`/`unica.meta.edit` union:

| Removed call | Canonical `operations` element |
| --- | --- |
| `unica.template.add` + `ObjectName`, `TemplateName`, `TemplateType` | `{"op": "add", "collection": "templates", "elements": [{"name": "Basic", "templateType": "SpreadsheetDocument"}]}` |
| `unica.template.remove` + `ObjectName`, `TemplateName` | `{"op": "remove", "collection": "templates", "names": ["Basic"]}` |
| `unica.help.add` + `ObjectName`, `Lang` | `{"op": "addHelp", "lang": "ru"}` |

The owner is addressed by `sourceSet + metadataPath`; the retired
`ObjectName` path dialect under `SrcDir` is gone. `templateType` defaults to
`SpreadsheetDocument`; `addHelp` is create-only and flips
`IncludeHelpInContents` on the owner's forms exactly the way the retired tool
did.

## Runtime delivery

The marketplace plugin contains skills, references, assets, `launch.sh`, and
three small native bootstrap binaries. It contains neither the `unica` core nor
engine binaries. Packaged `.mcp.json` invokes a command-scoped Git alias. Git's shell
runs `bootstrap/launch.sh`, which selects exactly one bootstrap:

- `darwin-arm64`;
- `linux-x64`;
- `win-x64` under Git for Windows.

The alias resolves the plugin root from whichever host it runs under. Claude
Code rewrites `${CLAUDE_PLUGIN_ROOT}` before the shell sees it; Codex leaves the
token unset, and the shell falls back to Git's own `$PWD`/`$GIT_PREFIX` pair.
One launcher therefore serves both hosts without a per-host package.

The bootstrap downloads only `unica-runtime-<target>.tar.gz` before MCP startup.
It reads the release-pinned `runtime-manifest.json`, verifies archive and file
SHA-256 values, publishes the core atomically in the host cache, and then execs
the single `unica` MCP process. Runtime stdout stays reserved for JSON-RPC;
bootstrap diagnostics use stderr.

The cache is `$CODEX_HOME/unica/runtimes` under Codex and
`${CLAUDE_PLUGIN_DATA}/runtimes` under Claude Code, which survives plugin
updates. Packaged `.mcp.json` passes the Claude token through
`UNICA_RUNTIME_CACHE_DIR`; a host that does not substitute it forwards the
literal token, and the bootstrap discards any value that still contains `${`
rather than creating a directory named after it.

Each installed artifact lives below
`<artifact>/<version>--<asset-sha256>/<target>`. The SHA-256 component prevents
a rebuilt engine with the same upstream version from reusing or overwriting old
bytes. The generated `third-party/manifest.json` maps tools to those artifact
roots, and internal launches re-check the pinned binary hash.

The core download happens inside the host's MCP startup budget. Packaged
`.mcp.json` therefore declares `startup_timeout_sec`, which bounds this
pre-startup transfer. The host waits for the core to be verified and published
before it starts MCP; a host that does not know the key ignores it.

After startup, engine delivery is non-blocking for concurrent callers. The
first call that needs an absent engine starts one server-owned delivery from the
pinned `unica-toolchain` asset. Concurrent calls
share it. If the owner cannot finish inside the bounded wait window, the call
returns `work.status=working`; retry the same domain call after the suggested
interval. There is no public install tool, and cancelling one call does not
cancel the shared delivery. To populate the core and every engine before
building an offline image, run:

```sh
<plugin-root>/bootstrap/bin/<target>/unica-bootstrap prefetch --plugin-root <plugin-root>
```

## Skills

The `skills/` tree covers configuration and extension metadata, forms, roles,
DCS/MXL, command interfaces, EPF/ERF and BSP registration, database/build
workflows, BSL search and diagnostics, integrations, background jobs,
performance, security, data separation, release support, autonomous runtime,
platform help, and logical source-resource inspection with a guarded BSL
replacement fallback.

It also covers applied-solution design, where the question is what to build
rather than how to write it: choosing the object class and typing its attributes
(`metadata-modeling`), designing registers (`register-design`), what a document
records and under which locks (`document-posting`), which event handler owns a
piece of logic (`object-events`), the managed form module and its client/server
boundary (`form-events`), which module hosts a procedure
(`module-placement`), the transaction, lock and responsible-read rules the
others defer to (`transactions-locks`), and concurrent editing of one object by
several users (`object-locks`).

## Local development

The source tree intentionally contains no generated tool binaries. Source
`.mcp.json` starts `cargo run --manifest-path ../../Cargo.toml --bin unica`.
Build a current-host development package under the distinct `unica-dev`
marketplace with:

```sh
scripts/dev/install-local-unica.sh
```

On native Windows x64, run the script from **Git Bash** included with 64-bit Git
for Windows. The local build requires Python 3.12 or newer, stable Rust with the
native MSVC toolchain, Microsoft C++ Build Tools, and the Windows SDK. A current
Codex CLI is required for the install and fresh-prompt verification steps.

WSL keeps Linux semantics and builds `linux-x64`. MSYS2 and Cygwin are not
supported shells for this installer; use Git Bash.

Useful flags:

```sh
scripts/dev/install-local-unica.sh --skip-build
scripts/dev/install-local-unica.sh --skip-install
scripts/dev/install-local-unica.sh --marketplace-name unica-dev
```

Claude Code loads a plugin directory directly, so the source tree needs no
marketplace and no install step:

```sh
claude --plugin-dir ./plugins/unica
```

To package a current-host Claude debug build instead, pass
`--local-debug-host claude` to `scripts/ci/package-unica-plugin.py`.

## Release pipeline

The source workflow builds the core and `unica-bootstrap` natively on each
runner, creates three deterministic core archives and checksum metadata,
re-downloads published core bytes, checks every pinned engine address, proves a
full `prefetch`, and emits one thin marketplace payload carrying both host
catalogs. Engine bytes remain in immutable `unica-toolchain` releases. A
separate workflow opens a plugin-only
staging PR in `IngvarConsulting/unica-marketplace`. After that commit is tagged
immutably, a catalog-only promotion PR points both stable `git-subdir` entries,
`.agents/plugins/marketplace.json` for Codex and `.claude-plugin/marketplace.json`
for Claude Code, to the tag.

The public catalog is never promoted before the source assets, staging commit,
and immutable marketplace tag exist.

## Verification

```sh
python3.12 -m pip install -r tests/ci/requirements.txt
python3.12 -m unittest discover -s tests/ci
python3.12 -m py_compile scripts/ci/*.py tests/ci/*.py
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace -- --test-threads=1
git diff --check
```

[Авторы, источники и лицензии](ATTRIBUTIONS.md).
License: LGPL-3.0-or-later.
