# 2. Ограничения

## Product Constraints

- Unica поставляется как Codex plugin under `plugins/unica`.
- Public AI entrypoint объявляется через `plugins/unica/.mcp.json`.
- Skills должны описывать developer operations, а не набор внутренних tools.
- `v8project.yaml` остается project/workspace boundary для 1C-проектов.
- Repository profiles live in local-only `unica.local.yaml`; unsupported keys
  are not added to the pinned v8-runner configuration schema.

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
- Branched-development recovery state is not cache. It lives under the OS
  per-user state root or `UNICA_STATE_DIR` and uses durable atomic records.
- Target locators and start-attempt replay records live under a non-overridable
  per-user coordination root, so changing `UNICA_STATE_DIR` cannot hide an
  unresolved task.
- That per-user root and its mutexes are local guards, not cross-host exclusion.
  A platform row declares each endpoint `hostConfined` or `multiHost`; any
  multi-host endpoint requires a reachable linearizable shared coordinator that
  atomically reserves target and repository-account keys before task creation.
  Unproven network-mounted file endpoints and missing coordinator evidence fail
  closed.
- Designer/repository behavior is an external platform capability. A version is
  supported only when an exact OS/platform/locale row has retained real-fixture
  evidence.

## Process Constraints

- Changes to public MCP tool names require tests and ADR update.
- Changes to skill routing must preserve the rule: route through MCP `unica`
  only.
- Generated binaries are not committed.
- Real repository acceptance uses only marker-owned disposable repositories and
  File IBs under an explicit test root; it never targets a user's repository.
