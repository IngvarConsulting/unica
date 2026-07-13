# Unica Codex Plugin

Unica is a Codex plugin for day-to-day 1C:Enterprise development work.

The public skills model developer operations, not infrastructure tools:

- create, inspect, edit, validate, compile, dump, and load 1C metadata;
- build, dump, and publish external processing/report source sets (`EPF`/`ERF`) through v8-runner MCP workflows;
- run v8-runner database/build/runtime workflows through MCP `unica`;
- work with forms, roles, SKD, MXL, subsystems, command interfaces, help, templates, and autonomous web-client debug;
- search, review, diagnose, test, and analyze BSL code inside those workflows.
- discover evidence-backed extension points before changing a typical supported configuration.
- bootstrap a new 1C repository workspace with `v8project.yaml`.

Bundled tooling, MCP server names, checksums, and third-party notices are internal package infrastructure. Project configuration is `v8project.yaml`; database and build workflows should use the `v8-runner` skill, which calls MCP `unica.runtime.execute`. See `references/tooling/internal-package.md` when maintaining the plugin itself.

## Skills

The `skills/` directory contains operation skills and scenario references for 1C developer workflows. Examples:

- `cf-edit`, `cf-info`, `cf-init`, `cf-validate`
- `cfe-init`, `cfe-borrow`, `cfe-diff`, `cfe-patch-method`, `cfe-validate`
- `v8-runner`, `db-auth-check`
- `epf-bsp-init`, `epf-bsp-add-command` for BSP registration helpers; EPF/ERF build and dump workflows live in `v8-runner`
- `form-add`, `form-edit`, `form-info`, `form-compile`, `form-validate`, `form-remove`
- `meta-compile`, `meta-edit`, `meta-info`, `meta-remove`, `meta-validate`
- `mxl-*`, `role-*`, `skd-*`, `subsystem-*`, `interface-*`, `template-*`, `web-test`, `img-grid`
- `extension-point-discovery`, `code-search`, `code-diagnostics`, `code-review`, `query-optimize`, `test-authoring`
- `api-design`, `platform-help`, `bsp-patterns`, `integration-implement`, `autonomous-server`, `log-analysis`
- `background-jobs`, `data-exchange`, `db-performance`, `security-auth-crypto`, `data-separation`, `release-support`, `support-edit`

## Local Codex Install

The source tree is for plugin and skill development. It does not commit bundled
tool binaries. Source `.mcp.json` starts the Rust orchestrator with
`cargo run --manifest-path ../../Cargo.toml --bin unica` from the plugin root;
bundled tools such as `bsl-analyzer`, `v8-runner`, and
`rlm-*` work from generated marketplace archives where target-specific binaries
exist.

Register the repo-local marketplace from the repository root when you only need
to inspect skills and metadata. Do not use a raw Git/source marketplace snapshot
as a working runtime install: source checkouts intentionally contain the
placeholder `third-party/manifest.json` and no generated `bin/<target>/`
binaries. Install a GitHub Release archive, run the installer, or build a local
debug package when MCP tools must run.

```sh
codex plugin marketplace add "$PWD"
```

Enable `unica@unica` in Codex. The plugin owns its MCP registrations through `.mcp.json`; do not add these servers separately with global `codex mcp add`.

To check what a fresh Codex session sees:

```sh
codex debug prompt-input 'test'
```

## Local Debug Install

From this repository, one command builds a fully working local Unica package for
the current machine, installs it into Codex as `unica-local`, and verifies a
fresh Codex prompt:

```sh
scripts/dev/install-local-unica.sh
```

The script builds only the current host target, writes the generated marketplace
under `.build/local-codex-unica/package/marketplace`, removes any previous
`unica-local` marketplace, adds the new one, validates the bundled MCP metadata
and native binaries, and checks that fresh Codex sees `Unica`, `v8-runner`, and
`db-auth-check`.

Useful development flags:

```sh
scripts/dev/install-local-unica.sh --skip-build
scripts/dev/install-local-unica.sh --skip-install
scripts/dev/install-local-unica.sh --marketplace-name unica-dev
```

## Support Matrix

| Area | Windows | macOS arm64 | Notes |
| --- | --- | --- | --- |
| Operation skills | Primary path | Available where the underlying 1C workflow is available | Skills route through the single public `unica` MCP server. |
| Native XML/DSL operations | Native Rust runtime | Native Rust runtime | `unica.form.*` and `unica.skd.*` do not call Python, Bash, or PowerShell operation scripts at runtime. Python reference models live only in parity fixtures. |
| Bundled binaries | Built by GitHub Actions into `bin/win-x64/` | Built by GitHub Actions into `bin/darwin-arm64/` | Linux x64 is built into `bin/linux-x64/`; the package manifest maps tools to target-specific binaries. Binaries are ignored in source control. |
| MCP local tools | Rust runtime resolver launches packaged binaries directly | Rust runtime resolver launches packaged binaries directly | Source checkouts use `cargo run --manifest-path ../../Cargo.toml --bin unica` from the plugin root; generated packages use `bin/<target>/` binaries. External standards data is reached through the internal standards adapter. |
| 1C platform operations | Requires local 1C platform | Requires local 1C platform or compatible tooling | Skills resolve project/database context from `v8project.yaml` when present. |

## Bundled Tools

Release packages include pinned binaries for `darwin-arm64`, `linux-x64`, and
`win-x64`. The dependency lock is `third-party/tools.lock.json`; do not duplicate
versions in CI scripts or docs.

- `bsl-analyzer`
- `v8-runner`
- `rlm-tools-bsl`
- `rlm-bsl-index`
- `unica`
- remote v8std MCP endpoint: `https://ai.v8std.ru/mcp`

Internal MCP runtime launches resolve bundled tools directly from Rust:

- read `third-party/manifest.json`;
- select the current host target (`darwin-arm64`, `linux-x64`, or `win-x64`);
- verify SHA-256 before real execution;
- execute the pinned `bin/<target>/<tool>` binary directly.

Runtime shell and PowerShell wrappers are not shipped. Source checkouts use
`cargo run --manifest-path ../../Cargo.toml --bin unica` for the orchestrator,
and generated packages use the native `bin/<target>/unica` entrypoint with
`cwd` pinned to the plugin root plus Rust-side resolver calls for internal
tools. This prevents Codex from
accidentally using a different globally installed version.

## Release Pipeline

`.github/workflows/unica-plugin-release.yml` builds the distributable marketplace package without committing generated binaries to the repository:

1. read `third-party/tools.lock.json`;
2. build `darwin-arm64`, `linux-x64`, and `win-x64` tool bundles;
3. download pinned `bsl-analyzer` and `v8-runner` release assets from the lock;
4. build `rlm-tools-bsl` and `rlm-bsl-index` with PyInstaller from the locked upstream source tag;
5. generate a target-specific `third-party/manifest.json` with SHA-256 checksums;
6. write official marketplace metadata with visible display name `Unica` and plugin id `unica`;
7. publish platform-specific archives such as `unica-codex-marketplace-darwin-arm64.tar.gz`, `unica-codex-marketplace-linux-x64.tar.gz`, and `unica-codex-marketplace-win-x64.zip` as workflow artifacts and, on tags, GitHub Release assets;
8. publish `install-unica.sh` for macOS/Linux and `install-unica.ps1` for
   Windows as release assets for one-command installation.

The tool build script requires Python 3.10 or newer; CI uses Python 3.12 and
creates a local venv under `.build/` for Python-packaged tools.

Use the generated marketplace archive as the candidate package for the official Codex store. Official distribution must use GitHub Actions package artifacts, not checked-in generated binaries.

## License

Unica is licensed under `LGPL-3.0-or-later`. See `LICENSE`.

## MCP Server

`.mcp.json` declares exactly one public MCP server:

- `unica`

`unica` owns workspace discovery, cache coordination, and adapter orchestration.
Build/runtime tooling, code analysis, standards lookup, and native XML/JSON DSL
handlers are private implementation details behind this one MCP contract.
Source checkout metadata uses
`cargo run --manifest-path ../../Cargo.toml --bin unica` because generated
binaries are absent from git; packaged archives rewrite `.mcp.json` to launch
`./bin/<target>/unica` directly with `cwd` set to the plugin root.

## Verification

From the repository root:

```sh
python3.12 -m unittest discover -s tests/ci
python3.12 -m py_compile scripts/ci/*.py tests/ci/*.py
python3.12 -m json.tool plugins/unica/.codex-plugin/plugin.json >/dev/null
python3.12 -m json.tool plugins/unica/.mcp.json >/dev/null
python3.12 -m json.tool plugins/unica/third-party/tools.lock.json >/dev/null
python3.12 -m json.tool plugins/unica/third-party/manifest.json >/dev/null
cargo fmt --all -- --check
cargo clippy --package unica-coder --all-targets --all-features -- -D warnings
cargo test --package unica-coder
rg 'references/(cc-1c-skills|ai-rules-1c)|Claude|[.]claude|Anthropic' plugins/unica/references plugins/unica/skills
git diff --check
codex debug prompt-input 'test'
```

For generated marketplace packages, extract the archive and run native binaries
from the package target directory:

```sh
TARGET=darwin-arm64
plugins/unica/bin/$TARGET/bsl-analyzer --version
plugins/unica/bin/$TARGET/bsl-analyzer search --help
plugins/unica/bin/$TARGET/v8-runner config init --help
plugins/unica/bin/$TARGET/rlm-tools-bsl --version
plugins/unica/bin/$TARGET/rlm-bsl-index --version
plugins/unica/bin/$TARGET/unica --help
```

## Updating Pinned Tools

Do not replace binaries in the repository. They are generated by CI.

For every tool update:

1. update pinned versions, tags, commits, upstream URLs, licenses, and target asset names in `third-party/tools.lock.json`;
2. run the GitHub Actions release workflow;
3. inspect the generated `third-party/manifest.json` inside the marketplace artifact;
4. run JSON validation, Python compile checks, binary version/help checks, MCP smoke tests, and fresh Codex prompt-input verification against the generated artifact.
