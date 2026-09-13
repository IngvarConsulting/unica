# Unica v0.13 Surface-First Tools Implementation Plan

> **Execution model:** one integrator owns shared daemon/MCP seams; independent
> workers take non-overlapping semantic families after the surface cutover.

**Goal:** replace all published v0.12.3 names with the exact 8/11 v0.13
surface immediately, keep every new tool useful in at least one closed mode,
and then complete semantic migration row-by-row from the 74-row transition
matrix without restoring aliases.

**Spec:** `docs/design/2026-08-31-v0-13-surface-first-cutover-design.md`

**Explicitly out of scope:** `plugins/unica/skills/**`. Their routing and prose
are a separate user-owned review gate. Merge to `main` is not release approval.

## Delivered Cutover Slice

- [x] Freeze the exact native profile at eight tools and compatibility profile
  at eleven tools.
- [x] Select V13 in the package path and route production stdio through the
  user daemon without legacy fallback.
- [x] Give every subject tool one useful closed mode:
  `view`, `find`, literal `search`, admission/readability `check`, structural
  `diff`, run dictionary, documentation search, and the initial write-free
  `apply(dryRun=true)` admission mode.
- [x] Return typed `unsupported_operation`, `unsupported_filter`,
  `unsupported_cursor`, `unsupported_scope`, or `unsupported_source` for known
  unfinished variants.
- [x] Make bootstrap require the exact eleven-tool compatibility list and
  reject any mixed legacy name.
- [x] Track the complete 74-row legacy-to-v0.13 parameter matrix and guard it
  against the immutable v0.12.3 release fixture.
- [x] Regenerate the public surface ledger for the eleven-tool compatibility
  profile and record the active surface-first decision/invariants.

## Remaining Semantic Waves

### Wave A — `apply` publication

Owners may work in parallel by family; the integrator alone edits the request
router, operation registry and actor publication seam.

- [ ] Metadata/properties: port `cf.edit`, `meta.add/edit/remove`, roles,
  subsystems, templates, support and XDTO operations to typed `apply.ops`.
- [ ] Forms/resources: port form, help, interface and resource mutations.
- [ ] DCS/MXL/code: port DCS/MXL writers and `code.patch` with byte-preserving
  selectors.
- [x] Replace admission-only dry-run with the same planner used by real
  publication for `props.set` and `attribute.add/set/remove`; prove no-write
  dry-run and atomic Source+WorkspaceCache commit.

Exit tests: one parity fixture per mapped operation, exact operation index in
diagnostics, revision mismatch, cancellation, rollback and postcondition proof.

### Wave B — read/query projections

This wave can run in parallel with Wave A because it owns separate handlers and
fixtures.

- [ ] `view/find`: add sections, outline, children/resources, XDTO and project
  projections plus logical path resolution required by matrix rows.
- [ ] `search/docs`: add regex/symbol search, addressable document retrieval,
  locale/version and multi-source policy.
- [ ] `check/diff`: add the closed validation/diagnostic filter union,
  extension comparison and revision-bound cursor.

Exit tests: legacy oracle fixture and canonical result fixture for every
absorbed or mapped read row; every deferred branch stays typed unsupported.

### Wave C — `run` and Task-backed operations

- [x] Implement bounded terminal operation `syntax.check`; then
  `test.run`, artifact make/load and infobase build in later iterations.
- [ ] Keep `query.execute` absent from v0.13; it has no successor in the
  approved public dictionary.
- [ ] Implement source create/attach/dump/convert and extension sync with typed
  protected runtime configuration.
- [ ] Keep `client.run`, raw sessions and tool download removed until a bounded
  terminal contract exists; never tunnel a command line through `args`.
- [ ] Prove direct/Task projection equivalence and no replay after handoff.

Exit tests: each implemented `run.op` changes `implemented` in the dictionary,
has direct and durable Task fixtures, and preserves one Invocation/one result.

## Aggregate Gates

- [ ] Execute the semantic oracle for all 74 matrix rows and record each as
  mapped, absorbed, transport-replaced, deliberately removed, or typed
  unsupported with an owner.
- [ ] Run exact 8/11 surface, bootstrap, architecture, package and three-host
  daemon suites.
- [ ] User performs the separate skills review; resulting skill changes, if
  any, are not folded silently into this implementation slice.
- [ ] Prepare an RC only after package evidence; publish only after explicit
  release approval and the release runbook.

## Fastest Parallel Ownership

| Slot | Exclusive scope | First remaining output |
| --- | --- | --- |
| Integrator | MCP/daemon routing, operation registry, publication, aggregate gates | real `apply` request pipeline |
| Worker A | metadata/properties apply families and fixtures | `meta.edit`/`cf.edit` vertical slice |
| Worker B | view/find/search/docs/check/diff handlers and fixtures | read/query parity batch |
| Worker C | run operations, runtime adapters and Task parity fixtures | `syntax.check`; `query.execute` removed from v0.13 |

Workers do not edit `plugins/unica/skills/**` or shared integrator hotspots.
Each slice starts with a failing matrix-derived fixture, merges independently,
and immediately updates its matrix disposition; no long-lived stacked PRs.
