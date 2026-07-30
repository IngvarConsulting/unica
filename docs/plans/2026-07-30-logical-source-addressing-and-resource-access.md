# Logical Source Addressing and Resource Access Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> `superpowers:subagent-driven-development` to implement this plan task by task.
> Every production change starts with a failing test, records the observed RED
> reason, and ends with the narrow GREEN command before broader verification.

**Goal:** Deliver ADR-0021 and ADR-0022 in PR #266: logical source addressing,
read-only navigation, bounded snapshot-backed resource access, full replacement
of one existing BSL module, and migration of `unica.code.patch` from physical
paths to `sourceSet + metadataPath`.

**Architecture:** Pure domain values describe logical addresses, resource
manifests, mutation plans, and typed events. Application handlers call
provider-neutral source-navigation and source-resource ports. The Platform XML
provider owns every physical path in non-serializable closed handles and
re-authorizes containment, format, support, owner identity, revision, and exact
preimages before publication. Public results never serialize a provider handle
or use a path as object identity.

**Tech Stack:** Rust 2021, `serde`, `serde_json`, `roxmltree`, Tokio
`CancellationToken`, existing Unica application/ports/native-operation
patterns, Python CI contract tests, Markdown ADR/invariant/acceptance corpus.

## Global Constraints

- Keep one public MCP server named `unica`; add only `unica.source.*` tools.
- Keep `unica.source.resources`, `unica.source.read`, and
  `unica.source.apply`; their removal was explicitly rejected.
- Implement both ADR-0021 and ADR-0022 in this PR, but preserve the dependency
  order: address core, `code.patch`, navigation, snapshot/read, then apply.
- `sourceSet` is mandatory for every exact target. `metadataPath` may be absent
  only for the selected source-set root.
- The first writable provider is Platform XML for Configuration and Extension.
  ExternalProcessor and ExternalReport roots are read-only until their
  multi-artifact fixtures prove exact addressing.
- Public schemas and results contain no workspace `path`, `sourceDir`, absolute
  path, provider revision, connection string, or opaque handle internals.
- Legacy `path` or `sourceDir` passed to `unica.code.patch` fails with stable
  code `legacy_target_removed` and names `sourceSet + metadataPath`.
- A closed handle is not write authority. Every write revalidates containment,
  Platform XML ownership/format, support state, logical owner, exact preimage,
  and BSL syntax after resolution.
- `source.apply` writes exactly one already-existing `bslModule`. Descriptor
  XML, registrations, forms, DCS, MXL, rights, binary content, unknown roles,
  multi-resource writes, creation, deletion, and rename are denied.
- Snapshot and resource identifiers are cryptographically unguessable opaque
  values bound to one application instance, workspace, provider, source set,
  target, scope, manifest revision, and expiry. A resource ID is valid only
  inside the snapshot that issued it.
- Incomplete or unavailable manifests are readable only when an issued resource
  is present, but never writable. `source.apply` requires completeness
  `complete`.
- Full BSL replacement preserves the exact leading UTF-8 BOM byte prefix and
  the observed uniform EOL profile. Mixed-EOL input is readable but replacement
  is denied with `validation_failed`; the implementation must not silently
  normalize it.
- `dryRun` defaults to `true`. Preview and apply use the same mutation-plan
  builder. Preview, rejection, no-op, and cancellation publish no event and
  invalidate no cache.
- Successful non-no-op apply publishes exactly one typed
  `SourceResourcesReplaced` event carrying the source set, logical owner,
  resource roles, pre/post hashes, and affected targets. It invalidates the BSL
  index and diagnostics.
- Cancellation is checked before resolution, between bounded read/plan phases,
  and immediately before atomic publication. Once the transaction begins, its
  all-or-nothing result governs.
- Bound the first contract to: 100 manifest resources per snapshot, 50 entries
  per page, 64 KiB per read response, 1 MiB replacement text, and a five-minute
  in-memory snapshot lifetime. These values become a checked quality
  requirement; tests use deterministic clocks or explicit expiry seams.
- Accept ADR-0021 and ADR-0022, supersede ADR-0015, and activate derived
  invariants only in the final contract-sync task after their executable tests
  exist. Intermediate task commits are implementation slices, not published
  architecture states.
- Do not weaken existing `code.patch` guarantees: preview, exact range/diff,
  idempotence, BOM/EOL handling, BSL parsing, source-map provenance, atomic
  publication, and byte-identical unrelated files remain covered.
- Use `apply_patch` for source edits. Preserve unrelated user changes.

---

## Task 1: Add the pure address profile and Platform XML closed-handle resolver

**Files:**

- Create: `crates/unica-coder/src/domain/source_target.rs`
- Modify: `crates/unica-coder/src/domain/mod.rs`
- Create: `crates/unica-coder/src/infrastructure/platform_xml_source_targets.rs`
- Modify: `crates/unica-coder/src/infrastructure/mod.rs`
- Modify: `crates/unica-coder/src/infrastructure/source_roots.rs`
- Modify: `crates/unica-coder/src/infrastructure/metadata_kinds.rs`
- Refactor: `crates/unica-coder/src/infrastructure/native_operations/code.rs`

**Contract:**

- Define serializable `SourceTarget`, canonical `MetadataAddress`,
  `TargetKind`, `ResolvedTarget`, and structured `SourceTargetError`.
- Define the versioned `platform-xml-8.3.27-format-2.20` profile. It accepts
  exact English and explicitly registered Russian kind aliases, emits English
  canonical kind tokens, preserves application names, and rejects unknown
  kinds, collections, terminals, empty segments, and unsupported profiles.
- Select an exact named project source set. Do not use default-root or
  `sourceDir` fallback.
- Resolve Configuration and Extension module terminals into a
  non-serializable `ClosedPlatformXmlTarget`. Cover common modules,
  object/manager/record-set/value-manager modules, common form and command
  modules, and nested form/command modules only where the current Platform XML
  layout and descriptor evidence prove them.
- Extract the existing module-layout and descriptor-evidence logic from
  `code.rs`; there must be one bidirectional mapping, not two copies.
- Revalidation must repeat source-map binding, owner evidence, non-symlink
  containment, and writable-path resolution against the closed handle.

**TDD sequence:**

1. Add domain tests for canonical English output, exact Russian alias
   normalization, application-name case preservation, unsupported terminal,
   malformed address, and unsupported profile.
2. Add provider tests using Configuration and Extension fixtures for identical
   logical module addresses, wrong source-set kind/format, missing descriptor,
   unregistered module, symlinked target, and source-map rebind failure.
3. Run:
   `cargo test -p unica-coder source_target -- --test-threads=1`
   and record the compile/test failure before adding production definitions.
4. Implement the smallest domain/profile and resolver code that makes the new
   tests pass.
5. Run:
   `cargo test -p unica-coder source_target -- --test-threads=1`
   and:
   `cargo test -p unica-coder platform_xml_source_targets -- --test-threads=1`.
6. Run existing `code` native-operation tests to prove the extraction did not
   change current behavior:
   `cargo test -p unica-coder infrastructure::native_operations::code::tests -- --test-threads=1`.
7. Commit with message:
   `feat(source): add logical Platform XML target resolver`.

## Task 2: Migrate `unica.code.patch` to logical targets

**Files:**

- Modify: `crates/unica-coder/src/application/tool_contracts.rs`
- Modify: `crates/unica-coder/src/application/operation_descriptors.rs`
- Modify: `crates/unica-coder/src/application/mod.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/code.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/common.rs`
- Modify: `plugins/unica/skills/code-patch/SKILL.md`
- Modify: `tests/ci/test_unica_skills.py`

**Contract:**

- Publish required `sourceSet`, `metadataPath`, `operation`, `selector`,
  `content`, and `position`. Do not publish `path` or `sourceDir`.
- Detect legacy selector fields before generic unknown-argument handling and
  return `legacy_target_removed` with the replacement selector.
- Build `CodePatchPlan` from the shared closed-handle resolver. Public typed
  result reports `sourceSet`, canonical `metadataPath`, and `targetKind`; it
  does not expose a physical path.
- Support both Platform XML Configuration and Extension modules.
- Keep format/support/containment checks on the resolved handle. The operation
  descriptor must explicitly declare handler-resolved guards so an empty path
  list cannot silently disable them.

**TDD sequence:**

1. Change schema tests first: assert `sourceSet + metadataPath`, absence of
   `path/sourceDir`, stable legacy error, and logical fields in typed output.
2. Add Configuration and CFE preview/apply fixtures. Assert the target module
   changes exactly once and descriptor/unrelated files remain byte-identical.
3. Add failure tests for locked support, changed owner/source map, symlink,
   invalid BSL, stale preimage, and unproven module role.
4. Run the focused tests and observe RED:
   `cargo test -p unica-coder code_patch -- --test-threads=1`.
5. Implement schema, descriptor, handler, and typed-result migration.
6. Run:
   `cargo test -p unica-coder code_patch -- --test-threads=1`
   and:
   `python3.12 -m unittest tests/ci/test_unica_skills.py`.
7. Commit with message:
   `feat(code): address patch targets logically`.

## Task 3: Add provider-neutral `source.resolve` and `source.children`

**Files:**

- Create: `crates/unica-coder/src/application/source_navigation.rs`
- Modify: `crates/unica-coder/src/application/mod.rs`
- Modify: `crates/unica-coder/src/application/ports.rs`
- Modify: `crates/unica-coder/src/application/tool_contracts.rs`
- Modify: `crates/unica-coder/src/infrastructure/application_ports.rs`
- Modify: `crates/unica-coder/src/infrastructure/platform_xml_source_targets.rs`
- Modify: `crates/unica-coder/src/interfaces/mcp.rs`

**Contract:**

- Add read-only `unica.source.resolve` and `unica.source.children` through
  application ports, not a filesystem-shaped public native handler.
- `resolve` requires `sourceSet` and `query`, supports `exact|prefix`, bounded
  `limit`, and returns canonical candidates with `targetKind`, typed
  `location`, and completeness. Exact mode never picks an ambiguous candidate.
- `children` requires `sourceSet`, accepts an optional exact `metadataPath`,
  traverses exactly one level, distinguishes collection nodes from item nodes,
  reports addressability and completeness, and uses opaque cursors.
- Configuration/Extension roots and proven Platform XML children are
  addressable. ExternalProcessor/ExternalReport source-set roots are virtual
  containers; enumerate multiple root descriptors read-only and fail closed
  when descriptor identity is ambiguous.
- A public location is either logical/addressable or observed/unaddressable.
  Physical relative paths may appear only in the observed branch, never as an
  identity or accepted selector.

**TDD sequence:**

1. Add application contract tests for both tool schemas and handler selection.
2. Add provider fixtures for exact/prefix resolution, ambiguous prefix, root
   traversal, collection vs item, Russian alias query, Configuration,
   Extension, and two artifacts in each external source-set kind.
3. Add MCP serialization tests ensuring bounded output and absence of private
   handle/provider fields.
4. Run and observe RED:
   `cargo test -p unica-coder source_navigation -- --test-threads=1`.
5. Implement domain request/results, ports, Platform XML enumeration, and tool
   registration.
6. Run:
   `cargo test -p unica-coder source_navigation -- --test-threads=1`
   and:
   `cargo test -p unica-coder interfaces::mcp::tests -- --test-threads=1`.
7. Commit with message:
   `feat(source): expose logical resolve and children`.

## Task 4: Add bounded resource snapshots and `source.read`

**Files:**

- Create: `crates/unica-coder/src/domain/source_resources.rs`
- Modify: `crates/unica-coder/src/domain/mod.rs`
- Create: `crates/unica-coder/src/application/source_resources.rs`
- Modify: `crates/unica-coder/src/application/mod.rs`
- Modify: `crates/unica-coder/src/application/ports.rs`
- Modify: `crates/unica-coder/src/application/tool_contracts.rs`
- Create: `crates/unica-coder/src/infrastructure/platform_xml_resources.rs`
- Modify: `crates/unica-coder/src/infrastructure/mod.rs`
- Modify: `crates/unica-coder/src/infrastructure/application_ports.rs`
- Modify: `crates/unica-coder/src/composition.rs`
- Modify: `crates/unica-coder/src/interfaces/mcp.rs`

**Contract:**

- Add read-only `unica.source.resources` and `unica.source.read`.
- Keep provider snapshot state in one long-lived
  `InfrastructureApplicationPorts`, scoped to the application instance.
- `resources` opens or pages one immutable manifest for
  `self|aggregate|registrations`, returning opaque UUID-derived IDs,
  `complete|partial|unavailable`, typed roles, media type, size, SHA-256 hash,
  text profile, `read|replace` capabilities, and limits.
- A Platform XML module target yields a complete self manifest with exactly one
  `bslModule`. Aggregate manifests may expose descriptor/registration resources
  as read-only and must mark the result partial unless completeness is proved.
- `read` validates the snapshot/resource pair, supports byte offset and capped
  length, returns actual range, total size, snapshot hash, EOF, content
  encoding, and observed BOM/EOL profile. It cannot read an arbitrary path.
- Enforce expiry, page/read bounds, deterministic cursor validation, workspace
  binding, and cancellation between phases. Errors use stable codes and never
  disclose a closed handle.

**TDD sequence:**

1. Add domain tests for role allowlist, completeness, limits, and serializable
   public values.
2. Add provider tests for opaque IDs, cross-snapshot forgery, wrong resource
   ID, expired snapshot, workspace mismatch, pagination, partial manifests,
   capped reads, UTF-8 BOM/CRLF, binary encoding, and cancellation.
3. Add application/MCP schema and typed-result tests; assert no path/handle or
   provider revision leaks.
4. Run and observe RED:
   `cargo test -p unica-coder source_resources -- --test-threads=1`.
5. Implement the provider-neutral port and Platform XML provider.
6. Run:
   `cargo test -p unica-coder source_resources -- --test-threads=1`
   and:
   `cargo test -p unica-coder interfaces::mcp::tests -- --test-threads=1`.
7. Commit with message:
   `feat(source): add bounded resource snapshots and reads`.

## Task 5: Add guarded single-BSL `source.apply` and typed invalidation

**Files:**

- Modify: `crates/unica-coder/src/domain/source_resources.rs`
- Modify: `crates/unica-coder/src/domain/events.rs`
- Modify: `crates/unica-coder/src/domain/cache.rs`
- Modify: `crates/unica-coder/src/application/source_resources.rs`
- Modify: `crates/unica-coder/src/application/mod.rs`
- Modify: `crates/unica-coder/src/application/ports.rs`
- Modify: `crates/unica-coder/src/application/tool_contracts.rs`
- Modify: `crates/unica-coder/src/infrastructure/platform_xml_resources.rs`
- Modify: `crates/unica-coder/src/infrastructure/application_ports.rs`
- Modify: `crates/unica-coder/src/infrastructure/workspace_services.rs`
- Modify: `crates/unica-coder/src/interfaces/mcp.rs`

**Contract:**

- Add mutating `unica.source.apply`; `dryRun` defaults to true and
  `dryRun:false` is required for publication.
- Accept one snapshot ID, one resource ID, `expectedHash`, UTF-8 replacement
  text, and no path. Enforce 1 MiB decoded content.
- Plan from the snapshot preimage, preserve exact BOM-prefix bytes and uniform
  EOL, parse replacement BSL, produce diff/changed ranges/post-hash, and use one
  exact-preimage `CompileTransaction`.
- Reject incomplete snapshots, stale revisions, hash mismatch, unsupported
  roles, mixed EOL replacement, multiple resources, owner/source-map changes,
  symlink escapes, format/support denial, parser failure, and cancellation
  before publication.
- Extend `HandlerOutcome` to carry handler-produced typed events. Do not
  synthesize a source replacement event from request arguments.
- A successful non-no-op apply emits exactly one
  `SourceResourcesReplaced` event with structured details. The cache maps it to
  BSL index/diagnostics invalidation and workspace services receive the typed
  event data. Preview/failure/cancellation/no-op emits none.

**TDD sequence:**

1. Add event/cache tests first for structured serialization and exact
   invalidation.
2. Add apply tests for every denial above plus preview/apply byte parity,
   event count/payload, no-op, exact unrelated-file preservation, rollback,
   and cancellation immediately before commit.
3. Add MCP schema/result tests and verify `dryRun` default behavior.
4. Run and observe RED:
   `cargo test -p unica-coder source_apply -- --test-threads=1`.
5. Implement the mutation plan, exact transaction, event transport, and cache
   invalidation.
6. Run:
   `cargo test -p unica-coder source_apply -- --test-threads=1`
   and:
   `cargo test -p unica-coder domain::cache::tests -- --test-threads=1`.
7. Commit with message:
   `feat(source): add guarded BSL resource replacement`.

## Task 6: Synchronize public skills, ADRs, invariants, quality limits, and acceptance

**Files:**

- Modify: `spec/decisions/0015-narrow-boundaries-for-code-patch-v1.md`
- Modify: `spec/decisions/0021-logical-source-addressing.md`
- Modify: `spec/decisions/0022-bounded-source-resource-access.md`
- Modify: `spec/decisions/README.md`
- Modify: `spec/architecture/invariants.md`
- Modify: `spec/architecture/quality-requirements.md`
- Modify: `spec/architecture/building-blocks.md`
- Modify: `spec/architecture/change-checklist.md`
- Create: `spec/acceptance/logical-source-addressing-and-resource-access.md`
- Modify: `docs/design/2026-07-29-logical-source-addressing-design.md`
- Modify: `docs/design/2026-07-30-bounded-source-resource-access-design.md`
- Create: `plugins/unica/skills/source-access/SKILL.md`
- Modify: `plugins/unica/skills/code-patch/SKILL.md`
- Modify: `spec/provenance/skill-upstreams.json`
- Modify: `plugins/unica/README.md`
- Modify: `tests/ci/test_architecture_registry.py`
- Modify: `tests/ci/test_architecture_sync_guard.py`
- Modify: `tests/ci/test_design_documents.py`
- Modify: `tests/ci/test_unica_skills.py`

**Contract:**

- Mark ADR-0021 and ADR-0022 `accepted`; mark ADR-0015 `superseded by
  ADR-0021`; update the decision index after renumbering both unmerged records
  around the ADR-0020 already accepted in the target branch.
- Mark both design documents `approved`.
- Add derived invariant rules with real CI checks for canonical logical
  addressing, closed-handle reauthorization, snapshot/resource binding,
  write-role allowlisting, preview/apply/event parity, and public-surface sync.
- Add a measured quality requirement recording the five-minute/100/50/64
  KiB/1 MiB bounds and cancellation checkpoints.
- Add acceptance traceability for RU/EN aliases, Configuration/Extension,
  multi-artifact external roots, exact/prefix/children, legacy rejection,
  private-field non-disclosure, forged/partial/stale snapshots,
  descriptor-write denial, symlink/owner swaps, exact byte parity, and one
  typed event.
- Add one Unica-owned `source-access` skill. It routes to a specialized writer
  first, uses resources/read for inspection, and uses apply only when no
  specialized writer exists, with an explicit fallback reason and preview
  before apply.
- Add a durable migration table to the package README:
  `path + sourceDir` → `sourceSet + metadataPath`.

**TDD sequence:**

1. Add/adjust CI assertions first for ADR lifecycle, registry owner/check
   fields, acceptance links, skill routing, provenance, and README migration.
2. Run the targeted Python tests and observe failures:
   `python3.12 -m unittest tests/ci/test_architecture_registry.py tests/ci/test_architecture_sync_guard.py tests/ci/test_design_documents.py tests/ci/test_unica_skills.py`.
3. Update normative documents, skills, provenance, and README.
4. Run the same targeted Python command until green.
5. Run:
   `python3.12 scripts/ci/check-architecture-sync.py`
   and:
   `python3.12 scripts/ci/check-plugin-publication-safety.py`.
6. Commit with message:
   `docs(architecture): accept logical source resource contracts`.

## Task 7: Add end-to-end MCP/package acceptance and run the full regression suite

**Files:**

- Modify: `tests/ci/test_unica_mcp_smoke.py`
- Modify: `scripts/ci/smoke-unica-mcp.py`
- Modify: `crates/unica-coder/tests/format_8_3_27_xml_corpus.rs`
- Modify: `spec/acceptance/format-profile-8-3-27.md`
- Modify: `.github/PULL_REQUEST_TEMPLATE.md` only if the existing template
  contains a public-contract checklist that needs a new checked item

**Contract:**

- Exercise all five source tools through JSON-RPC `tools/list` and
  `tools/call`, not only direct Rust functions.
- Cover one Configuration and one Extension BSL module end to end:
  resolve → resources → read → apply preview → apply → read current bytes.
- Assert old `code.patch` selectors fail with the migration code.
- Assert descriptor XML remains read-only and cannot be replaced.
- Assert packaged plugin smoke sees the same schemas/results as the local
  binary.
- Keep format-profile fixtures byte-identical except the selected BSL resource.

**TDD sequence:**

1. Add the smoke and corpus assertions, run:
   `python3.12 -m unittest tests/ci/test_unica_mcp_smoke.py`
   and record the initial failure.
2. Complete any missing integration wiring without weakening lower-level
   guards.
3. Run:
   `python3.12 -m unittest tests/ci/test_unica_mcp_smoke.py`
   and:
   `cargo test -p unica-coder --test format_8_3_27_xml_corpus -- --test-threads=1`.
4. Run focused full crate verification:
   `cargo test -p unica-coder -- --test-threads=1`.
5. Run the repository CI entry point documented by the current workflow. Use
   the exact tracked script/command discovered from `.github/workflows/` and
   record its exit status in the SDD ledger.
6. Verify `git diff --check`, public tool schemas, no leaked physical selector,
   and a clean worktree.
7. Commit with message:
   `test(source): prove MCP resource access end to end`.
