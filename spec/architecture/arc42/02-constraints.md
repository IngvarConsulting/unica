# 2. Ограничения

## Product Constraints

- Unica поставляется как Codex plugin under `plugins/unica`.
- Public AI entrypoint объявляется через `plugins/unica/.mcp.json`.
- Skills должны описывать developer operations, а не набор внутренних tools.
- `v8project.yaml` остается project/workspace boundary для 1C-проектов.

## Technical Constraints

- Rust workspace currently exposes package `unica-coder` and binary `unica`.
- The stdio MCP server must support `initialize`, `ping`, `tools/list`, and
  `tools/call`.
- Runtime architecture has no dependency on Python/PowerShell/Bash operation
  files. Donor scripts may exist only as test fixtures for parity with native
  `unica.*` MCP implementations.
- Packaged binary execution goes through checksum-verifying wrappers and
  generated `third-party/manifest.json`.
- `.build/` is volatile and ignored; orchestrator cache state may live under
  `.build/unica` unless `UNICA_CACHE_DIR` overrides it.

## Process Constraints

- Changes to public MCP tool names require tests and ADR update.
- Changes to skill routing must preserve the rule: route through MCP `unica`
  only.
- Generated binaries are not committed.
