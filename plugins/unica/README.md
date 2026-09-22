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

## Data composition schemas

Use [dcs-compile](skills/dcs-compile/SKILL.md) to create a schema and
[dcs-edit](skills/dcs-edit/SKILL.md) to modify it. Both use `unica.apply`
with preview and the resulting revision for application. The XML format is
described in the [DataCompositionSchema specification](references/specs/1c-dcs-spec.md).

## Reading and changing source objects

Find logical addresses with `unica.search` using `corpus: "names"`.
Use `unica.resolve` when a physical path arrived from outside Unica.
Addresses have the form `<sourceSet>:<Kind>.<Name>...`; Unica resolves
physical source files internally.

Read a node with `unica.view` and validate it with `unica.check`, passing
its address in `at`. Results are returned in the MCP response. To change
an object, preview its operations with `unica.apply`, review the result,
and apply using the returned revision.

See [source-access](skills/source-access/SKILL.md) for navigation,
[code-patch](skills/code-patch/SKILL.md) for BSL changes, and
[xdto](skills/xdto/SKILL.md) for typed XDTO operations.

## Templates, embedded help and validation

Template registration and embedded help are `unica.apply` operations of the
owning object: `template.add`, `template.set`, `template.remove` and
`help.create`. Validation is `unica.check` over a node: the node kind
owns its validators (`cf`, `cfe`, `form`, `dcs`, `mxl`, `role`,
`subsystem`, `interface`, `meta`); the verdict travels in `data.status` and a
root outside the platform XML profile `2.20` is reported as a warning
diagnostic next to it. The retired `unica.template.*`, `unica.help.add` and
`unica.*.validate` names have no alias.

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
