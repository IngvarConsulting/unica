# 11. Риски и технический долг

## Active Risks

- Standards adapter is not yet a full native HTTP MCP proxy.
- Native XML/DSL handlers can drift from donor behavior if parity fixtures are
  not updated together with Rust ports.
- Cache reporting exists before full lazy/eager rebuild implementation.
- The public tool list can grow too broad if every internal capability is
  mirrored one-to-one.
- Fresh Codex visibility can be affected by stale local plugin cache.
- Designer batch documentation does not prove atomic locks/commits, global lock
  ownership discovery, or rollback semantics required by branched development.
- Existing support editing is not layer-aware and cannot preserve arbitrary
  multi-vendor chains for this workflow.
- Designer credentials supplied as argv may be visible to privileged local
  process inspection even when Unica logs and state are fully redacted.
- Capability evidence can drift across platform version, OS, locale, and
  encoding; version detection alone is not a safety proof.

## Mitigations

- Keep gaps in the implementation task list.
- Add parity fixtures and MCP contract tests for donor behavior that must remain
  compatible.
- Keep `.mcp.json` single-server tests.
- Validate generated marketplace packages, not only the source checkout.
- Use clean `CODEX_HOME` for visibility proof.
- Gate repository automation on exact retained real-platform capability rows;
  keep unproven fields nullable and unproven mutations disabled.
- Use a dedicated integration account, durable operation journal, exact
  compensation, and no-force unlock/recovery rules; gate the distinct
  repository-update add/delete confirmation on an approved plan and fixture.
- Require layer-aware support round trips and full packaged acceptance before
  closing issue #137.
