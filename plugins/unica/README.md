# Unica Codex Plugin

Unica is a Codex plugin for day-to-day 1C:Enterprise development work.

The public skills model developer operations, not infrastructure tools:

- create, inspect, edit, validate, compile, dump, and load 1C metadata;
- build and validate external processings and reports (`EPF`/`ERF`);
- create, run, update, dump, and load infobases;
- work with forms, roles, SKD, MXL, subsystems, command interfaces, help, templates, and web publication;
- search and analyze BSL code inside those workflows.

Bundled tooling, wrappers, MCP server names, checksums, and third-party notices are internal package infrastructure. Project configuration is `v8project.yaml` / `V8TR_CONFIG`; database and build workflows should use v8-runner before native fallback scripts. See `references/tooling.md` when maintaining the plugin itself.

## Skills

The `skills/` directory contains operation skills adapted from `cc-1c-skills` with plugin-local scripts and references. Examples:

- `cf-edit`, `cf-info`, `cf-init`, `cf-validate`
- `cfe-init`, `cfe-borrow`, `cfe-diff`, `cfe-patch-method`, `cfe-validate`
- `db-create`, `db-run`, `db-update`, `db-dump-xml`, `db-load-xml`, `db-dump-cf`, `db-load-cf`, `db-load-git`
- `epf-init`, `epf-build`, `epf-dump`, `epf-validate`
- `erf-init`, `erf-build`, `erf-dump`, `erf-validate`
- `form-add`, `form-edit`, `form-info`, `form-compile`, `form-validate`, `form-remove`
- `meta-compile`, `meta-edit`, `meta-info`, `meta-remove`, `meta-validate`
- `mxl-*`, `role-*`, `skd-*`, `subsystem-*`, `interface-*`, `template-*`, `web-*`, `img-grid`

The previous infrastructure skills (`unica-setup`, `unica-bsl`, `unica-v8-runner`, `unica-rlm-tools-bsl`, `unica-v8std`) are intentionally not public skills.

## Local Codex Install

Register the repo-local marketplace from the repository root:

```sh
codex plugin marketplace add "$PWD"
```

Enable `unica@unica-local` in Codex. The plugin owns its MCP registrations through `.mcp.json`; do not add these servers separately with global `codex mcp add`.

To check what a fresh Codex session sees:

```sh
codex debug prompt-input 'test'
```

## Support Matrix

| Area | Windows | macOS arm64 | Notes |
| --- | --- | --- | --- |
| Operation skills and PowerShell scripts | Primary path | Available when PowerShell is installed | The source skills are Windows-first because 1C Designer automation is Windows-first. |
| Python script ports | Available with Python | Available with `python3` | Used for XML/metadata operations where ports exist. |
| Bundled binaries | Built by GitHub Actions into `bin/win-x64/` | Built by GitHub Actions into `bin/darwin-arm64/` | Linux x64 is built into `bin/linux-x64/`; release artifacts carry the generated multi-target manifest. |
| MCP local tools | Available through PowerShell launcher when package contains Windows binaries | Available through shell wrappers | Remote `unica-v8std` works independently of local binaries. |
| 1C platform operations | Requires local 1C platform | Requires local 1C platform or compatible tooling | Skills resolve project/database context from `v8project.yaml` when present. |

## Bundled Tools

Release packages include pinned binaries for `darwin-arm64`, `linux-x64`, and `win-x64`:

- `bsl-analyzer` `0.1.144`
- `v8-runner` `0.3.0`
- `rlm-tools-bsl` `1.9.4`
- `rlm-bsl-index` `1.9.4`
- remote v8std MCP endpoint: `https://ai.v8std.ru/mcp`

Every bundled binary launch goes through a wrapper:

- `scripts/run-tool.sh` for macOS/Linux shell environments;
- `scripts/run-tool.ps1` for PowerShell environments;
- per-tool shell wrappers used by current stdio MCP entries.

Wrappers read `third-party/manifest.json`, check the host target, verify SHA-256, and then execute the pinned binary. This prevents Codex from accidentally using a global tool of another version.

## Release Pipeline

`.github/workflows/unica-plugin-release.yml` builds the distributable marketplace package without committing generated binaries to the repository:

1. build `darwin-arm64`, `linux-x64`, and `win-x64` tool bundles;
2. download pinned `bsl-analyzer` and `v8-runner` release assets;
3. build `rlm-tools-bsl` and `rlm-bsl-index` with PyInstaller on the native runner for each target;
4. generate a multi-target `third-party/manifest.json` with SHA-256 checksums;
5. publish `unica-codex-marketplace-<version>.tar.gz` and `.zip` as workflow artifacts and, on tags, GitHub Release assets.

Use the generated marketplace archive as the candidate package for the official Codex store. The checked-in repository can keep local development binaries, but official distribution must use the GitHub Actions package artifacts.

## License

Unica is licensed under `LGPL-3.0-or-later`. See `LICENSE`.

## MCP Servers

`.mcp.json` declares internal MCP endpoints used by operation workflows:

- `unica-bsl-reference`
- `unica-bsl-workspace`
- `unica-v8-runner`
- `unica-rlm-tools-bsl`
- `unica-v8std`

Skills should choose these by task: code search and diagnostics use BSL/RLM tools, build and database workflows use v8-runner/platform tooling, and standards/APK questions use v8std plus reference material.

## Verification

From the repository root:

```sh
python3 -m json.tool plugins/unica/.codex-plugin/plugin.json >/dev/null
python3 -m json.tool plugins/unica/.mcp.json >/dev/null
python3 -m json.tool plugins/unica/third-party/manifest.json >/dev/null
bash -n plugins/unica/scripts/*.sh
rg '\.claude/skills' plugins/unica/skills
codex debug prompt-input 'test'
```

For checked-in local development tools on macOS arm64:

```sh
plugins/unica/scripts/run-bsl-analyzer.sh --version
plugins/unica/scripts/run-v8-runner.sh --help
plugins/unica/scripts/run-rlm-tools-bsl.sh --version
plugins/unica/scripts/run-rlm-bsl-index.sh --version
```

## Updating Pinned Tools

Do not replace binaries without bumping the plugin version and updating `third-party/manifest.json`.

For every tool update:

1. update pinned versions, tags, commits, upstream URLs, and licenses in `scripts/ci/build-unica-tools.py`;
2. run the GitHub Actions release workflow;
3. inspect the generated `third-party/manifest.json` inside the marketplace artifact;
4. run JSON validation, script syntax checks, binary version/help checks, MCP smoke tests, and fresh Codex prompt-input verification against the generated artifact.
