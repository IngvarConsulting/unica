# Architecture Change Checklist

Use this checklist when changing public MCP tools, skill routing, adapters,
cache behavior, or packaging metadata.

## MCP Surface

- [ ] `.mcp.json` still declares exactly one public server: `unica`.
- [ ] `initialize` still returns `serverInfo.name = "unica"`.
- [ ] `tools/list` contains intended `unica.*` tools only.
- [ ] Public tool name changes are covered by tests and ADR updates.

## Skill Routing

- [ ] Updated skills mention MCP `unica`.
- [ ] Updated skills do not expose internal adapter server names as user-facing
  routing.
- [ ] Updated skills do not point users to skill-local Python/PowerShell
  operation files.
- [ ] Mutating skills preserve explicit `dryRun: false` guidance.

## Cache And Events

- [ ] Mutating operation emits the right `DomainEventKind`.
- [ ] `CacheImpact` invalidates affected caches.
- [ ] Dry-run reports impact without writing state.
- [ ] Applied operation writes state only after successful mutation or approved
  state transition.
- [ ] Applied mutations notify live workspace services when analyzer or index
  caches are affected.

## Project Discovery And Discovery Receipts

- [ ] Discovery orchestration still uses typed evidence ports and never parses
  human-readable adapter output or reads adapter storage directly.
- [ ] Related artifacts, flow edges, and actionable candidates remain separate;
  incomplete evidence cannot become a false contradiction or actionable hook.
- [ ] Stable evidence preserves canonical identity, provenance, coverage,
  freshness, and content fingerprints; `workspaceEpoch` remains diagnostic.
- [ ] Every discovery receipt contains atomic grants rather than independent
  lists that can expand tool, target, mutation class, change kind, destination,
  parameters, or allowed-artifact scope.
- [ ] The receipt lease remains held through handler execution, typed-effect and
  manifest verification, and atomic advancement or revocation.
- [ ] Dry-run neither acquires nor advances a receipt; partial and out-of-scope
  writes cannot leave it valid.
- [ ] Support guard still runs before discovery guard, and guard mode remains
  server/workspace configuration without a per-call bypass.
- [ ] Version-1 exchange and report/data-processor variants outside the accepted
  typed proof boundaries remain `unknown`, not lexically inferred.

## Shadow Observation And Replay

- [ ] The non-authoritative JSONL journal is written after the operation outcome
  and journal failure cannot change handler, receipt, or rollout decisions.
- [ ] Observation storage is OS-locked, schema-versioned, bounded, and updates
  aggregate counters without becoming authoritative state.
- [ ] Observation and deterministic replay records contain digests and policy
  predicates but must never contain task text or source text, raw mutation
  arguments, absolute paths, or unhashed artifact names.
- [ ] Corrupt and unknown-schema records are reported and excluded.
- [ ] Audit/replay stays a maintainer-only packaged command and is not added to
  public MCP tools or discovery skill routing.

## Adapters

- [ ] Internal adapter errors are summarized in `warnings` or `errors`.
- [ ] Adapter command construction is covered by focused tests when behavior is
  non-trivial.
- [ ] Analyzer/index adapters that need warm workspace state go through the
  workspace service manager.
- [ ] Cheap read-only adapters such as `unica.code.grep` do not start workspace
  services.
- [ ] Operation backends use native Rust handlers, not Python/PowerShell/Bash
  runtime fallbacks.
- [ ] Fixture parity exists when donor script behavior is retained as the
  reference source model.

## Packaging

- [ ] `third-party/tools.lock.json` names the bundled binary `unica`.
- [ ] Generated `third-party/manifest.json` matches the lock.
- [ ] `cargo run --quiet --bin unica -- --help` works from source checkout.
- [ ] Generated package `.mcp.json` starts `./bin/<target>/unica` directly with
      `cwd` set to the plugin root.
- [ ] Fresh Codex visibility is checked from a clean cache when changing plugin
  metadata.

## Verification

Run:

```sh
cargo fmt --all -- --check
cargo clippy --package unica-coder --all-targets -- -D warnings
cargo test --package unica-coder
python3.12 -m unittest discover -s tests/ci
git diff --check
```

BSP parity fixtures are the narrow exception to whitespace normalization: they
preserve harvested bytes under `.gitattributes` `-text -whitespace`, and their
manifest hashes are the integrity check.
