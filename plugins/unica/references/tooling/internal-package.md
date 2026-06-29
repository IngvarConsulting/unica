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

Bundled tools are launched through checksum-verifying resolution instead of
global tool names. Manual/package smoke paths use wrappers; Rust internal
adapters may resolve and execute the packaged binary directly through
`third-party/manifest.json`. Both paths prevent accidental use of a globally
installed tool with a different version.

- `scripts/run-tool.sh <tool-name> [args...]` is the macOS/Linux launcher.
- `scripts/run-tool.ps1 <tool-name> [args...]` is the PowerShell launcher.
- Per-tool shell wrappers call `run-tool.sh` for smoke tests and manual use.
- Rust internal adapters resolve the target-specific binary from the manifest,
  verify SHA-256, set `UNICA_PLUGIN_ROOT`, and execute the binary directly. This
  avoids invoking `.sh` wrappers through `Command::new(...)` on Windows.
  The packaged MCP runtime is shell-first on macOS/Linux for top-level server
  startup; Windows sidecar tools should use manifest-resolved binaries or the
  PowerShell launcher.

Launcher responsibilities:

- locate the plugin root from the launcher path;
- read `third-party/manifest.json`;
- reject unsupported host triples;
- verify the tool binary exists;
- verify SHA-256 before every execution;
- forward all remaining arguments unchanged.

Runtime script inventory in `plugins/unica/scripts/` is intentionally small:

- `run-tool.sh`: common macOS/Linux manifest and checksum launcher;
- `run-tool.ps1`: common PowerShell manifest and checksum launcher;
- `run-bsl-analyzer.sh`, `run-v8-runner.sh`, `run-rlm-tools-bsl.sh`, `run-rlm-bsl-index.sh`: direct per-tool shell entrypoints for smoke tests and manual use;
- `run-bsl-reference.sh`, `run-bsl-workspace.sh`, `run-v8-runner-mcp.sh`: MCP profile launchers where one binary exposes several task-specific server modes.

That is nine runtime scripts: two common launchers, four direct tool wrappers,
and three MCP profile wrappers. Dependency versions must not be copied into
these scripts; they come from `third-party/tools.lock.json` and the generated
manifest.

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
- tag builds upload the same archives plus `install-unica.sh` to the GitHub Release.

## MCP Contract

The plugin declares exactly one public MCP server in `.mcp.json`:

- `unica`

Operation skills should route through `unica`. Build/runtime tools, code
analysis, standards lookup, and XML/JSON DSL scripts are internal adapters
owned by the orchestrator, so cache refresh and source-set invalidation happen
inside one process instead of through LLM-visible coordination.

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
bash -n plugins/unica/scripts/*.sh
python3 -m py_compile scripts/ci/*.py
rg 'unica-(bsl|v8-runner|v8std|rlm-tools-bsl|coder)' plugins/unica/skills
codex debug prompt-input 'test'
```

Run binary version/help smoke tests from an extracted generated marketplace
archive, not from the source tree.
