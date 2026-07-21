# Unica Codex Plugin

Unica models day-to-day 1C:Enterprise development workflows and exposes one
public stdio MCP server named `unica`. Prompt-visible skills call native
`unica.*` tools; bundled analyzers, runners, indexes, and the standards adapter
remain private implementation details.

## Public installation

Prerequisites are Codex CLI and Git. Node.js, Python, download utilities, and
archive utilities are not consumer dependencies.

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

## Runtime delivery

The marketplace plugin contains skills, references, assets, `launch.sh`, and
three small native bootstrap binaries. It contains no full `bin/<target>` tool
runtime. Packaged `.mcp.json` invokes a command-scoped Git alias. Git's shell
runs `bootstrap/launch.sh`, which selects exactly one bootstrap:

- `darwin-arm64`;
- `linux-x64`;
- `win-x64` under Git for Windows.

The bootstrap reads the release-pinned `runtime-manifest.json`, downloads
`unica-runtime-<target>.tar.gz`, verifies archive and file SHA-256 values, and
publishes the runtime atomically under `$CODEX_HOME/unica/runtimes`. It then
execs the single `unica` MCP process. Runtime stdout stays reserved for JSON-RPC;
bootstrap diagnostics use stderr.

The runtime archive contains the target's `unica`, `bsl-analyzer`, `v8-runner`,
`rlm-tools-bsl`, and `rlm-bsl-index` binaries plus the generated
`third-party/manifest.json`. Internal launches re-check the pinned binary hash.

## Skills

The `skills/` tree covers configuration and extension metadata, forms, roles,
DCS/MXL, command interfaces, EPF/ERF and BSP registration, database/build
workflows, BSL search and diagnostics, integrations, background jobs,
performance, security, data separation, release support, autonomous runtime,
and platform help.

## Local development

The source tree intentionally contains no generated tool binaries. Source
`.mcp.json` starts `cargo run --manifest-path ../../Cargo.toml --bin unica`.
Build a current-host development package under the distinct `unica-dev`
marketplace with:

```sh
scripts/dev/install-local-unica.sh
```

On native Windows x64, run the script from **Git Bash** included with 64-bit Git
for Windows. The local build requires Python 3.10 or newer, stable Rust with the
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

## Release pipeline

The source workflow builds tools and `unica-bootstrap` natively on each runner,
creates deterministic runtime archives and checksum metadata, re-downloads
published release bytes for verification, and emits one thin marketplace
payload. A separate workflow opens a plugin-only staging PR in
`IngvarConsulting/unica-marketplace`. After that commit is tagged immutably, a
catalog-only promotion PR points the stable `git-subdir` entry to the tag.

The public catalog is never promoted before the source assets, staging commit,
and immutable marketplace tag exist.

## Verification

```sh
python3.12 -m unittest discover -s tests/ci
python3.12 -m py_compile scripts/ci/*.py tests/ci/*.py
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace -- --test-threads=1
git diff --check
```

License: LGPL-3.0-or-later.
