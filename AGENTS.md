# Agent Entry Points

## Source Of Truth

When changing Unica, resolve conflicts in this order:

1. code/tests/package metadata > spec > historical plans
2. `plugins/unica/.mcp.json`, `plugins/unica/.codex-plugin/plugin.json`, and `plugins/unica/third-party/tools.lock.json` are package-contract sources, not background notes.
3. `spec/` is the active architecture layer unless it contradicts live code, tests, or package metadata.
4. `docs/superpowers/plans/` is historical execution context. Do not treat old plan text as current requirements without checking code and tests.
5. `docs/superpowers/investigations/` is a non-normative evidence archive. It
   preserves research, rejected alternatives, and handoff material, but never
   overrides code, tests, package metadata, active `spec/`, or accepted ADRs.

## Search Hygiene

Do not scan local ignored corpora as part of normal repo understanding:

- `docs/research`
- `docs/its`
- `target`
- `.build`
- `dist`

Use `rg`/`git ls-files` first. For packaging questions, prefer tracked files plus generated package artifacts over raw filesystem walks.
For archived investigations, open the nearest `README.md` first and then read
only the documents named by its continuation map; do not recursively ingest the
whole archive during normal repository understanding.

## Development Rules

- Fix root causes, not symptoms.
- Surface contradictions in assumptions, docs, tests, and runtime behavior.
- Keep the public MCP boundary as one server named `unica` with `unica.*` tools unless an ADR changes that contract.
- Prompt-visible skills stay MCP-first. Direct packaged-script execution paths must not return once a native `unica.*` tool exists, except for documented utility exceptions.
