# Agent Entry Points

## Source Of Truth

When changing Unica, resolve conflicts in this order:

1. code and tests
2. `plugins/unica/.mcp.json`, `plugins/unica/.codex-plugin/plugin.json`, and `plugins/unica/third-party/tools.lock.json` are package-contract sources, not background notes.
3. `spec/` is the active architecture layer unless it contradicts live code, tests, or package metadata.
4. `README.md` and skill prose

## Search Hygiene

Do not scan local ignored corpora as part of normal repo understanding:

- `target`
- `.build`
- `dist`
- `docs-local` (except when the task needs official 1C platform documentation)

Use `rg`/`git ls-files` first. For packaging questions, prefer tracked files plus generated package artifacts over raw filesystem walks.

## Local 1Ci Platform Documentation

For questions about official 1C platform behavior, search the private local
corpus at `docs-local/1ci/8.3.27/en/` before using the network. If the required
guide is absent or `manifest.json` is missing or not marked `"complete": true`,
run `python3.12 scripts/dev/download-1ci-guides.py` from the repository root and
retry the local search.

The corpus is local research material only. Do not commit it, copy it into
`plugins/unica/`, include it in packages, or publish it. The downloader may
fetch `https://kb.1ci.com/bin/download/*` attachments despite that path being
disallowed by `robots.txt`; this is a narrow, explicitly approved exception and
must not be generalized to other disallowed paths.

## Development Rules

- Fix root causes, not symptoms.
- Surface contradictions in assumptions, docs, tests, and runtime behavior.
- Keep the public MCP boundary as one server named `unica` with `unica.*` tools unless an ADR changes that contract.
- Prompt-visible skills stay MCP-first. Direct packaged-script execution paths must not return once a native `unica.*` tool exists, except for documented utility exceptions.
