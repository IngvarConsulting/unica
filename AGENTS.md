# Agent Entry Points

## Source Of Truth

When changing Unica, resolve conflicts in this order:

1. code and tests
2. `plugins/unica/.mcp.json`, `plugins/unica/.codex-plugin/plugin.json`, `plugins/unica/.claude-plugin/plugin.json`, and `plugins/unica/third-party/tools.lock.json` are package-contract sources, not background notes.
3. `spec/` is the active architecture layer unless it contradicts live code, tests, or package metadata.
4. `README.md` and skill prose

## Releasing

Publishing a version to the public marketplace follows
`docs/release-runbook.md`. Read it before acting on any request to cut, ship,
promote, or finish a release; the step order carries the ADR-0008 guarantee that
the catalog never points at bytes that are not final, and improvising it exposes
consumers to an unverified package.

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
- One plugin directory serves Codex and Claude Code. Keep both manifests at the same version, keep `.mcp.json` host-neutral, and do not add optional manifest or catalog keys without checking that the oldest supported client accepts them; an unrecognized key is a load error there, not a warning.

## Pull-request Topology

- Default to one independently reviewable PR per coherent change, targeting `main` or an explicitly named release branch.
- Do not open a PR whose base is the head branch of another open PR. Do not use child PRs as a queue for review fixes: commit and push those fixes to the existing PR's head branch.
- Before opening a PR, inspect the intended base on GitHub. If it belongs to an open PR, stop and either use that PR's head branch or ask the user for direction; branch names alone are not evidence of an independent base.
- A stacked PR is allowed only when the user explicitly requests a named stack and its merge/rebase order. Each member must explain its parent, standalone review boundary, and closure plan in its PR body.
- If an agent cannot push to the existing PR head, it must provide a patch or ask for access; it must not create a child PR as a workaround.
- A distinct bug discovered during review belongs in an independent `main`-targeted PR or an issue, never in an implicit PR stack.
