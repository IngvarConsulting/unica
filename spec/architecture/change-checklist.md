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
- [ ] Skills use explicit `dryRun: false` for an honest preview/apply contract,
  or the documented prepare/apply authorization when preview cannot be honest.

## Cache And Events

- [ ] Mutating operation emits the right `DomainEventKind`.
- [ ] `CacheImpact` invalidates affected caches.
- [ ] Supported dry-run reports impact without target/workspace/cache/domain
  mutation; durable workflows may write only bounded idempotency/preview
  evidence. No-preview mutations use a typed sandbox/prepare gate instead of a
  fabricated preview.
- [ ] Applied operation writes remote-effect intent before mutation; it writes
  success/cache/domain-result state only after observed postconditions or an
  approved local state transition.
- [ ] Applied mutations notify live workspace services when analyzer or index
  caches are affected.

## Branched Development

- [ ] Durable task/operation records use `UNICA_STATE_DIR`/OS state, not the
  volatile workspace cache root.
- [ ] A non-overridable owner-only target locator prevents a state-root override
  from hiding unresolved tasks and preserves failed-start replay.
- [ ] Mutating requests require stable `taskId` and `operationId`; replay input
  hashes and state transitions are covered by tests.
- [ ] Compatible tools accept the original `cwd` plus opaque `branchedTask`,
  resolve the owned disposable workspace internally, and mutations return
  durable receipts/cache events for that context.
- [ ] Ordinary task mutations roll phase back to `developing` and invalidate all
  descendant evidence atomically; merge-resolution receipts are session-bound.
- [ ] Compatible general-tool responses recursively project every structured and
  free-text field; byte/value scans prove that no absolute disposable,
  work-root, state, or coordination path crosses MCP.
- [ ] Designer argv is built from typed public/path/secret arguments and raw
  command material is absent from MCP responses.
- [ ] Repository update/acquisition/rollback/commit/unlock behavior is enabled
  only for a matching real-platform capability row; exact incoming add/delete
  is preview-bound before internal structural confirmation.
- [ ] Compensation touches only operation-owned locks; ambiguous ownership
  yields `recoveryRequired`.
- [ ] Original-target and repository-account reservations independently prevent
  concurrent tasks from confusing same-user lock ownership.
- [ ] Support edits name one vendor layer and preserve unrelated layers byte for
  byte.
- [ ] Cleanup revalidates the owned marker, nonce, containment, and every
  symlink/reparse/Git/root guard immediately before quarantine and deletion.
- [ ] Package and real-platform evidence cover the full acceptance matrix in
  `spec/acceptance/branched-development.md`.

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
