# 11. Риски и технический долг

## Active Risks

- Standards adapter is not yet a full native HTTP MCP proxy.
- Native XML/DSL handlers can drift from donor behavior if parity fixtures are
  not updated together with Rust ports.
- Cache reporting exists before full lazy/eager rebuild implementation.
- The public tool list can grow too broad if every internal capability is
  mirrored one-to-one.
- Fresh Codex visibility can be affected by stale local plugin cache.
- Typed evidence providers may be unavailable, bounded, stale, or disagree;
  hiding those states would create false supported or contradicted verdicts.
- Content fingerprints and an exclusive receipt lease add bounded I/O and
  contention to enforceable applied mutations.
- A resolver that emits a broad or ambiguous scope could let one receipt grant
  authorize a neighboring target.
- Shadow observations could leak user or source material if unhashed fields are
  added casually, or could be mistaken for authoritative state.
- Synthetic fixtures can overstate discovery quality; guard promotion without
  audited real observations can create false blocks.
- EDT and several exchange/print mechanism variants are intentionally outside
  the version-1 typed proof boundary.

## Mitigations

- Keep gaps in the implementation task list.
- Add parity fixtures and MCP contract tests for donor behavior that must remain
  compatible.
- Keep `.mcp.json` single-server tests.
- Validate generated marketplace packages, not only the source checkout.
- Use clean `CODEX_HOME` for visibility proof.
- Preserve typed provider coverage/provenance and return `unknown` for material
  degradation or unsupported variants.
- Bind atomic grants to exact resolver output, hold the lease through handler
  and manifest verification, and exercise concurrency in receipt state-machine
  tests.
- Keep the shadow journal non-authoritative, schema-versioned, and bounded.
  Journal and replay records must never contain task text or source text, raw
  arguments, absolute paths, or unhashed artifact names; provide deterministic
  replay and audit outside public MCP.
- Require the 48-case corpus, metamorphic renaming/decoys, zero-tolerance safety
  gates, and live observation thresholds before promotion from `observe`.
