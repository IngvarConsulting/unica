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

Use `rg`/`git ls-files` first. For packaging questions, prefer tracked files plus generated package artifacts over raw filesystem walks.

## Development Rules

- Fix root causes, not symptoms.
- Use the installed Superpowers workflow for implementation, debugging, planning, and verification work.
- For every Rust implementation or refactoring change, use the `rust-expert-best-practices-code-review` skill before final verification; apply its type-safety, error-handling, and API-design guidance.
- Surface contradictions in assumptions, docs, tests, and runtime behavior.
- Keep the public MCP boundary as one server named `unica` with `unica.*` tools unless an ADR changes that contract.
- Prompt-visible skills stay MCP-first. Direct packaged-script execution paths must not return once a native `unica.*` tool exists, except for documented utility exceptions.
