# Unica Internal Tooling Notes

This document is internal reference material for the Unica plugin package. Public
skills should describe 1C developer operations, not the bundled tools themselves.

## Public Skill Boundary

Public skills live in `skills/` and model tasks a 1C developer performs:

- create, edit, validate, compile, dump, and load metadata;
- build and validate EPF/ERF artifacts;
- create and update infobases;
- inspect and edit forms, roles, SKD, MXL, subsystems, interfaces, and autonomous web-client debug;
- search, analyze, and validate BSL code as part of those workflows.

Tool-specific behavior is an implementation detail of those workflows.

## Bundled Tools

The pinned bundled tools are declared in `third-party/tools.lock.json`.
Release packages are platform-specific: each GitHub Actions package contains
one `bin/<target>/` directory and a matching `third-party/manifest.json` for
that target. The checked-in manifest is only a source-tree placeholder.

- `bsl-analyzer`: BSL diagnostics, metadata/code inspection, and local/reference MCP profiles.
- `v8-runner`: 1C build, syntax, test, and platform-oriented automation.
- `rlm-tools-bsl`: token-efficient exploration of large 1C BSL repositories.
- `rlm-bsl-index`: repository indexing for `rlm-tools-bsl`.
- `unica`: Rust stdio MCP orchestrator and the only public MCP server.
- remote v8std endpoint: standards, APK codes, and v8-code-style context through an internal adapter.

Never replace a binary manually in the repository. Update
`third-party/tools.lock.json`, bump the plugin version, and let the release
workflow generate binaries, SHA-256 entries, and marketplace archives.

## Launchers

Internal MCP runtime launches bundled tools through the Rust manifest resolver,
not through shell or PowerShell wrappers. The resolver prevents accidental use
of a globally installed tool with a different version by selecting the pinned
package binary and verifying SHA-256 before execution.

- The Rust resolver reads `third-party/manifest.json`, selects the current
  target (`darwin-arm64`, `linux-x64`, or `win-x64`), verifies the binary, and
  launches `bin/<target>/<tool>` directly.
- Source checkout `.mcp.json` starts the Rust orchestrator with
  `cargo run --manifest-path ../../Cargo.toml --bin unica` from the plugin root.
- Generated marketplace packages rewrite `.mcp.json` to start
  `./bin/<target>/unica` directly with `cwd` set to the plugin root.

Runtime resolver responsibilities:

- locate the plugin root;
- read `third-party/manifest.json`;
- reject unsupported host triples;
- verify the tool binary exists;
- verify SHA-256 before every execution;
- forward all remaining arguments unchanged.

Runtime shell/PowerShell wrapper files are intentionally absent from
`plugins/unica/scripts/`. Dependency versions must stay in
`third-party/tools.lock.json` and the generated manifest, not in executable
launcher scripts.

## Release Packaging

`.github/workflows/unica-plugin-release.yml` builds official marketplace
artifacts:

- each target job prepares `bin/<target>/` and a target-local `tools.json`;
- target jobs read all dependency pins and target asset names from
  `third-party/tools.lock.json`;
- Python-packaged tools are built in a target-local venv; `build-unica-tools.py`
  requires Python 3.10 or newer and CI runs it on Python 3.12;
- each package job writes one target-specific generated
  `third-party/manifest.json`;
- the package job writes official marketplace metadata where the marketplace
  name is `unica`, the plugin id is `unica`, and the visible display name is
  `Unica`;
- the final artifacts are platform-specific, for example
  `unica-codex-marketplace-darwin-arm64.tar.gz`,
  `unica-codex-marketplace-linux-x64.tar.gz`, and
  `unica-codex-marketplace-win-x64.zip`;
- tag builds upload the same archives plus `install-unica.sh` for macOS/Linux
  and `install-unica.ps1` for Windows to the GitHub Release.

## MCP Contract

The plugin declares exactly one public MCP server in `.mcp.json`:

- `unica`

Operation skills should route through `unica`. Build/runtime tools, code
analysis, standards lookup, and XML/JSON DSL scripts are internal adapters
owned by the orchestrator, so cache refresh and source-set invalidation happen
inside one process instead of through LLM-visible coordination. The source
checkout `.mcp.json` uses
`cargo run --manifest-path ../../Cargo.toml --bin unica` because binaries are
not committed; release packaging rewrites `.mcp.json` to launch the
platform-specific `./bin/<target>/unica` binary directly with `cwd` set to the
plugin root.

Raw Git/source marketplace installs are development-only. They can expose
skills and plugin metadata, but they do not provide target-specific binaries or
a generated tool manifest. Users who need working MCP tools should install a
release archive or a locally generated marketplace package.

## Reference Material

Reference material is organized by 1C development scenario rather than by
upstream source:

- `references/README.md`: main scenario index.
- `references/use-cases/`: task-oriented guidance for 1C specialists.
- `references/specs/`: stable XML and JSON DSL contracts.
- `references/platform/`: 1C development standards and platform pitfalls.
- `references/tooling/`: Unica packaging, runtime, MCP, and `v8project.yaml` notes.

The former upstream-shaped folders were intentionally removed. Provenance is
kept in git history; the packaged reference tree should stay task-oriented.

## Verification

From the repository root:

```sh
python3 -m json.tool plugins/unica/.codex-plugin/plugin.json >/dev/null
python3 -m json.tool plugins/unica/.mcp.json >/dev/null
python3 -m json.tool plugins/unica/third-party/tools.lock.json >/dev/null
python3 -m json.tool plugins/unica/third-party/manifest.json >/dev/null
python3 -m py_compile scripts/ci/*.py
rg 'unica-(bsl|v8-runner|v8std|rlm-tools-bsl|coder)' plugins/unica/skills
codex debug prompt-input 'test'
```

Run binary version/help smoke tests from an extracted generated marketplace
archive, not from the source tree.
