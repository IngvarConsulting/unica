# Project Discovery And Discovery Receipts Implementation Plan

> Historical execution context for staged implementation. This document and
> its checkboxes are not evidence that delivery is complete. Current source of
> truth is code/tests/package metadata, then `spec/`, not this plan.

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> `superpowers:subagent-driven-development` to implement this plan task by task.
> Every production change follows `superpowers:test-driven-development`, and
> completion follows `superpowers:verification-before-completion`.

**Goal:** Deliver `unica.project.discover` as a typed, deterministic,
evidence-backed MCP tool with proposal validation, server-owned rolling
receipts, and an application-owned discovery guard whose first enforceable
resolver is `unica.cfe.patch_method`.

**Architecture:** `DiscoverExtensionPointsUseCase` orchestrates six typed
evidence ports and never parses display output. Platform XML and bounded BSL
providers build an evidence graph with explicit coverage and provenance.
Validation emits a receipt only for fully supported, unambiguous proposals
whose resolver binds one atomic grant to tool, target, mutation class, change
kind, destination source-set, and exact artifacts. A content-based composite
snapshot and exclusive receipt lease protect the complete handler window.

**Tech Stack:** Rust 2021, `serde`/`serde_json`, `sha2`, `roxmltree`, `fs2`,
existing MCP stdio transport, Python 3.12 package/contract tests.

**Non-negotiable delivery boundary:** The public tool is registered only in
Task 12, after explore, validate, receipt storage, lease, rolling advance, and
guard tests are green. EDT uses only a bounded diagnostic snapshot and returns
typed `unknown` / `unsupported_source_format`; unimplemented mechanism variants
also return typed `unknown`. Neither path can issue a receipt or fall back to
SQLite, display strings, or unbounded scans.
The implementation can ship all four guard modes, but package defaults remain
`observe`; promotion to `warn` or `deny` requires the live observation volumes
defined by the accepted spec and is not fabricated by tests.

---

### Task 1: Commit the accepted design and architecture decision

**Files:**
- Modify: `Cargo.lock`
- Modify: `crates/unica-coder/Cargo.toml`
- Modify: `spec/README.md`
- Modify: `spec/architecture/invariants.md`
- Modify: `spec/architecture/change-checklist.md`
- Create: `spec/architecture/extension-point-discovery.md`
- Create: `spec/decisions/0008-project-discovery-and-discovery-receipts.md`
- Modify: `spec/decisions/README.md`
- Modify: `spec/architecture/arc42/05-building-block-view.md`
- Modify: `spec/architecture/arc42/06-runtime-view.md`
- Modify: `spec/architecture/arc42/08-cross-cutting-concepts.md`
- Modify: `spec/architecture/arc42/09-architecture-decisions.md`
- Modify: `spec/architecture/arc42/10-quality-requirements.md`
- Modify: `spec/architecture/arc42/11-risks-and-technical-debt.md`
- Create: `docs/superpowers/plans/2026-07-17-project-discovery-receipts.md`
- Test: `tests/ci/test_product_contracts.py`

- [ ] **Step 1: Write failing architecture-sync assertions**

Add a test that requires ADR 0008, the accepted design link, and explicit
invariant/checklist entries for typed evidence, atomic grants, content
fingerprints, and lease-through-handler semantics.

```python
def test_project_discovery_architecture_is_synchronized(self):
    root = Path(__file__).resolve().parents[2]
    adr = (root / "spec/decisions/0008-project-discovery-and-discovery-receipts.md").read_text()
    invariants = (root / "spec/architecture/invariants.md").read_text()
    checklist = (root / "spec/architecture/change-checklist.md").read_text()
    self.assertIn("Status: accepted", adr)
    self.assertIn("typed evidence ports", invariants)
    self.assertIn("discovery receipt", checklist.lower())
```

Also parse every fenced `json` block in the accepted design with an
`object_pairs_hook` that rejects duplicate keys; documentation examples are
executable contract fixtures, not illustrative pseudo-JSON.

- [ ] **Step 2: Run the focused test and verify RED**

Run:
`python3.12 -m unittest tests.ci.test_product_contracts.ProductContractTests.test_project_discovery_architecture_is_synchronized -v`

Expected: FAIL because ADR 0008 and synchronization entries do not exist.

- [ ] **Step 3: Add ADR and synchronize the active architecture layer**

Record the accepted replacement of PR #83, six typed ports, source/destination
snapshot model, atomic grant, lease/CAS, rollout separation, narrow v1 proof
boundaries for mechanism families 7 and 8, and the rule that `workspaceEpoch`
is diagnostic-only.

- [ ] **Step 4: Re-run the focused test and spec hygiene checks**

Run:
`python3.12 -m unittest tests.ci.test_product_contracts -v`

Run: `git diff --check`

Expected: PASS; no draft language, duplicate JSON keys, or unresolved design
questions remain.

- [ ] **Step 5: Commit the architecture slice**

```bash
git add spec docs/superpowers/plans/2026-07-17-project-discovery-receipts.md tests/ci/test_product_contracts.py
git -c commit.gpgsign=false commit -m "docs: принять архитектуру project discovery"
```

### Task 2: Typed discovery contract, provenance, and deterministic identities

**Files:**
- Create: `crates/unica-coder/src/application/discovery/mod.rs`
- Create: `crates/unica-coder/src/application/discovery/contract.rs`
- Create: `crates/unica-coder/src/application/discovery/model.rs`
- Create: `crates/unica-coder/src/application/discovery/determinism.rs`
- Modify: `crates/unica-coder/src/application/mod.rs`
- Create: `crates/unica-coder/src/domain/discovery_registry.rs`
- Modify: `crates/unica-coder/src/domain/mod.rs`
- Modify: `crates/unica-coder/src/infrastructure/metadata_kinds.rs`
- Test: the four new modules

- [ ] **Step 1: Write failing serde and determinism tests**

Cover: unknown nested fields; explore forbids proposals; validate requires a
non-empty proposal array; unique proposal IDs; optional mutation intent;
canonical artifact refs; exact limit defaults/ranges; stable IDs under input
and record permutation; collapse only of facts identical including provenance,
location, and fingerprint; timestamps and `workspaceEpoch` excluded from
digests. Assert that IDs change with normalized request, analysis-contract
version, source/composite fingerprint, provider name/version/outcome digest, or
limits.
Exercise every accepted/rejected boundary for task, concepts, search terms,
known artifacts, proposals, proposal ID/intent, source-set name, resolver
strings, artifact refs, and list uniqueness.
Add a hard-coded canonical hash golden vector, stable-tag exhaustiveness tests,
and a collision-injection test proving same ID/different payload is fatal.
Round-trip every one of the 15 kind-specific canonical shapes and reject
cross-kind shapes, unregistered object/module kinds, and specialized roots
mislabelled as `metadata_object`.

```rust
#[test]
fn validate_rejects_unknown_nested_fields() {
    let error = serde_json::from_value::<DiscoverRequest>(json!({
        "mode": "validate", "task": "x", "concepts": ["x"],
        "proposals": [{"id": "p", "target": {"kind": "method",
          "ref": "CommonModule.X.Run", "confidence": 1}}]
    })).unwrap_err();
    assert!(error.to_string().contains("unknown field"));
}

#[test]
fn evidence_ids_are_stable_under_provider_permutation() {
    assert_eq!(canonical_evidence_ids(&facts_ab()), canonical_evidence_ids(&facts_ba()));
}
```

- [ ] **Step 2: Run focused tests and verify RED**

Run: `cargo test --locked -p unica-coder application::discovery -- --nocapture`

Expected: FAIL because discovery modules and DTOs do not exist.

- [ ] **Step 3: Implement strict input and output DTOs**

Implement `DiscoverMode`, `ArtifactKind`, `ArtifactRef`, `DiscoverLimits`,
`Proposal`, `MutationIntent`, `DiscoveryStatus`, `EvidenceLevel`, `Coverage`,
`CheckState`, `CheckOutcome`, `CheckSeverity`, `SupportState`, `EvidenceType`,
`Evidence`, `RelatedArtifact`, `FlowEdge`, `Candidate`,
`ProposalVerdict`, `ReceiptEligibility`, and `DiscoveryReport`. Use
`#[serde(deny_unknown_fields, rename_all = "camelCase")]` on every input DTO.
Use `BTreeMap`/`BTreeSet` and explicit sort keys at digest boundaries.

`ArtifactKind` contains all v1 identities: `metadata_object`,
`metadata_attribute`, `tabular_section`, `tabular_section_attribute`, `module`,
`method`, `form`, `form_command`, `common_command`, `event_subscription`,
`scheduled_job`, `http_route`, `exchange_plan`, `report`, and
`data_processor`. `MutationIntent` is a tagged enum with only the strict
`unica.cfe.patch_method` variant in schema v1; proposals for other tools omit
it. Its nested arguments reject unknown fields and include normalized/defaulted
`Context` and `IsFunction` in addition to the four required live-tool fields.
Model provider facts as a closed internal enum; the public evidence wire shape
is the strict subject/factCode/optional-object/provenance/freshness contract
from the spec, never arbitrary `Value`.

Move the existing metadata-kind registry into the neutral domain registry and
add the versioned module-kind registry there. Keep
`infrastructure/metadata_kinds.rs` as a thin compatibility re-export while
current callers migrate. ArtifactRef validation and future Platform XML
providers consume this one registry; application code never imports
infrastructure and no second list is allowed.

- [ ] **Step 4: Implement canonicalization and stable SHA-256 identifiers**

Use a versioned domain-separated streaming canonical encoder with hardcoded
field order, length-prefixed UTF-8/integers, and explicit `stable_tag()` values
for enums; do not hash `serde_json::Value` or serialized maps because the
workspace enables `preserve_order`. Derive `analysis_` and `ev_` plus the full
64 lowercase SHA-256 hex digest. Analysis IDs bind the normalized request,
contract version, composite fingerprint, deterministic limits, and sorted
provider identity/readiness/coverage/reason/record digests. Evidence IDs bind
the complete fact, location, provider/version, coverage, and source fingerprint.
Exclude timestamps, durations, display details, and `workspaceEpoch`.
Before evidence canonicalization, normalize the captured snapshot and every
accepted record to the current request's diagnostic epoch. Matching source-set
identity and content fingerprint remain authoritative; otherwise identical
records from different epochs deduplicate and never produce an identifier
collision or change the discovery outcome.
Reject empty components, traversal-like refs, invalid proposal IDs, and invalid
limit/cardinality/string ranges. Canonical report ordering sorts every public
collection and nested ID/reason/blocker list. Collapse only byte-identical facts
including provenance; identical IDs with differing payload are fatal. Do not
add domain synonyms or scalar scores.

- [ ] **Step 5: Re-run tests and commit**

Run: `cargo test --locked -p unica-coder application::discovery -- --nocapture`

```bash
git add crates/unica-coder/src/application crates/unica-coder/src/domain crates/unica-coder/src/infrastructure/metadata_kinds.rs docs/superpowers/plans/2026-07-17-project-discovery-receipts.md
git -c commit.gpgsign=false commit -m "feat: добавить typed discovery contract"
```

### Task 3: Typed evidence ports, graph, validation, and materiality

**Files:**
- Create: `crates/unica-coder/src/application/discovery/ports.rs`
- Create: `crates/unica-coder/src/application/discovery/evidence_graph.rs`
- Create: `crates/unica-coder/src/application/discovery/proposal_validator.rs`
- Create: `crates/unica-coder/src/application/discovery/use_case.rs`
- Modify: `crates/unica-coder/src/application/discovery/mod.rs`
- Create: `crates/unica-coder/src/domain/source_snapshot.rs`
- Modify: `crates/unica-coder/src/domain/mod.rs`
- Test: these modules with fake ports

- [ ] **Step 1: Write failing fake-port behavior tests**

Test all `ProviderOutcome` states and prove these distinctions:

- empty complete batch can contradict an exact proposal;
- bounded/unavailable batch produces `unknown`, never `contradicted`;
- lexical evidence alone never yields an actionable candidate;
- a connected target plus known support is actionable;
- a material blocker survives unrelated provider success;
- non-material optional degradation yields `partial` and may keep receipt
  eligibility;
- conflicting facts block a receipt;
- no actionable result is `insufficient`, not an operation error.

```rust
#[test]
fn unavailable_definition_is_unknown_not_contradicted() {
    let report = fixture()
        .definitions(ProviderOutcome::unavailable("index_building", true))
        .validate(method_proposal())
        .unwrap();
    assert_eq!(report.proposal_verdicts[0].verdict, Verdict::Unknown);
    assert!(!report.receipt_eligibility.eligible);
}
```

- [ ] **Step 2: Run and verify RED**

Run: `cargo test --locked -p unica-coder discovery::use_case -- --nocapture`

- [ ] **Step 3: Define exactly six named evidence ports**

Add `MetadataCatalogPort`, `CodeSearchPort`, `DefinitionPort`, `CallGraphPort`,
`FormInspectionPort`, and `SupportStatePort`. Each returns typed
`ProviderBatch<T>` or an explicit unavailable/failed/contract-violation state.
Every fact carries canonical identity, `SourceLocation`, provider name/version,
coverage, source fingerprint, and workspace epoch. Only contract violations are
fatal `Err`.

The application outcome algebra is exactly `complete`, `bounded`,
`unavailable`, `failed`, and `contract_violation`. A complete empty batch is
negative proof only for the exact typed query. Bounded, unavailable, and failed
outcomes remain inconclusive. Binding records bind exact mechanism details;
call records bind resolution, call type, and execution context into the stable
evidence digest.

The canonical binding compatibility matrix is:

| `BindingDetails` | Accepted `FlowKind` | Supplying evidence port |
| --- | --- | --- |
| `Structural` | `contains`, `defines` | `MetadataCatalogPort` |
| `EventSubscription` | `subscribes` | `MetadataCatalogPort` |
| `FormCommand` | `handles` | `FormInspectionPort` |
| `CommonCommand` | `handles` | `MetadataCatalogPort` |
| `ScheduledJob` | `handles` | `MetadataCatalogPort` |
| `HttpRoute` | `handles` | `MetadataCatalogPort` |
| `ExchangePlan` | `handles` | `MetadataCatalogPort` |

Every other `BindingDetails` x `FlowKind` x evidence-port combination is a
`ProviderContractViolation` and must be rejected before evidence-graph promotion.
Infrastructure adapters must emit only these combinations and must not guess a
relation from artifact names, display text, or provider availability.

`contains` and `defines` structural edges remain observed graph evidence; they
never populate `connection_ports`, establish runtime reachability, or make a
candidate actionable.

Runtime materiality follows evidence contribution: every runtime port present
in `connection_ports` for the selected target is material, while other
potential runtime ports are optional. If no runtime connection is established,
a conclusive negative requires complete exact coverage from
`MetadataCatalogPort`, `CallGraphPort`, and `FormInspectionPort`.

Keep non-evidence orchestration dependencies explicit and separate:
`ProjectSourceResolverPort`, `SourceSnapshotPort`, and `ReceiptIssuerPort`.
Define typed `ResolvedSourceSet`, `SourceSnapshot`, and
`DiscoveryExecutionContext` in the domain/application boundary now so this
slice compiles before concrete filesystem adapters exist. Fake implementations
prove that application code imports no infrastructure module.

`SourceSnapshot` reuses the domain `SourceFormat` and models exactly one
analysis snapshot plus canonically sorted/deduplicated mutation snapshots.
Content capture remains behind `SourceSnapshotPort` until Task 4.

- [ ] **Step 4: Implement evidence graph and proposal validator**

Retain conflicting facts. Build only typed `contains`, `defines`, `calls`,
`handles`, `subscribes`, and `uses` edges. Compute evidence levels from facts,
not ordering. Attach checks per affected candidate/proposal and apply the
specified status precedence.

- [ ] **Step 5: Implement use-case orchestration with injected providers**

`DiscoverExtensionPointsUseCase::execute` selects the analysis source-set,
normalizes the query plan, obtains one source snapshot, invokes ports, builds
the graph, validates proposals, and returns a typed report. It does not issue a
persisted receipt yet; inject a `NoopReceiptIssuer` returning an explicit
eligibility blocker until the receipt-store task.

- [ ] **Step 6: Re-run and commit**

Run: `cargo test --locked -p unica-coder discovery -- --nocapture`

```bash
git add crates/unica-coder/src/application/discovery crates/unica-coder/src/domain \
  spec/architecture/extension-point-discovery.md \
  docs/superpowers/plans/2026-07-17-project-discovery-receipts.md
git -c commit.gpgsign=false commit -m "feat: реализовать evidence graph и validation"
```

### Task 4: Contained project source resolution and content snapshots

**Files:**
- Modify: `crates/unica-coder/src/application/discovery/{mod.rs,model.rs,ports.rs,use_case.rs}`
- Modify: `crates/unica-coder/src/application/{mod.rs,tool_contracts.rs}`
- Modify: `crates/unica-coder/src/domain/project_sources.rs`
- Modify: `crates/unica-coder/src/domain/source_snapshot.rs`
- Modify: `crates/unica-coder/src/domain/discovery_registry.rs`
- Create: `crates/unica-coder/src/infrastructure/contained_fs.rs`
- Create: `crates/unica-coder/src/infrastructure/platform_xml.rs`
- Create: `crates/unica-coder/src/infrastructure/project_sources.rs`
- Create: `crates/unica-coder/src/infrastructure/source_snapshot.rs`
- Modify: `crates/unica-coder/src/infrastructure/mod.rs`
- Modify: `spec/architecture/extension-point-discovery.md`
- Modify: `tests/ci/test_product_contracts.py`
- Test: source modules, discovery orchestration, and product contracts

- [ ] **Step 1: Write failing containment and fingerprint tests**

Cover: duplicate source-set names; configured absolute/outside roots; symlink
escape; same-length same-mtime byte change; deterministic path ordering;
mapping/name/kind/format changes; composite configuration+extension snapshot;
analysis plus sorted/deduplicated multiple destination snapshots; generated and
ignored-corpus exclusions; file/byte/time bounds; unreadable material file;
concurrent file mutation during hashing; Unknown/Invalid/EDT format eligibility.
EDT configuration receives a complete diagnostic snapshot of the four
versioned marker paths (`.project`, `DT-INF/PROJECT.PMF`, and the two supported
`Configuration.mdo` locations), with no recursive scan or destinations. It is
sufficient only for the typed skipped/inconclusive unsupported report;
markerless EDT, EDT extensions, unknown/invalid layouts, external sources, and
ineligible destination roles fail with their exact typed source-readiness
reason before snapshot capture or evidence providers.

```rust
#[test]
fn content_change_with_unchanged_len_and_mtime_changes_fingerprint() {
    let before = fixture.snapshot().unwrap();
    fixture.replace_same_len_and_restore_mtime("CommonModules/X/Ext/Module.bsl");
    let after = fixture.snapshot().unwrap();
    assert_ne!(before.fingerprint, after.fingerprint);
}

#[test]
fn composite_snapshot_binds_analysis_and_destination() {
    let a = fixture.snapshot_pair("main", "ExtensionA").unwrap();
    let b = fixture.snapshot_pair("main", "ExtensionB").unwrap();
    assert_ne!(a.composite_fingerprint, b.composite_fingerprint);
}
```

- [ ] **Step 2: Run and verify RED**

Run: `cargo test --locked -p unica-coder source_snapshot -- --nocapture`

- [ ] **Step 3: Harden source-map identities**

Canonicalize source roots under the workspace, reject duplicate names and
escapes, and return a typed `ResolvedSourceSet`. Auto-select only one eligible
set; multiple eligible sets without `sourceSet` are an operation error.
Reject missing/non-directory configured roots and dangling or live
symlink/reparse components in the public map as well as discovery resolution;
do not use `Path::exists()` as an allow-missing security decision.

- [ ] **Step 4: Implement bounded content manifests**

Hash mapping identity, sorted workspace-relative regular-file paths, and each
file's SHA-256. Build a source-format-aware manifest from Platform XML
registration plus registered metadata files and their contained source
subtrees; do not traverse unrelated workspace files when a source root is `.`.
Hash mapping configuration separately. Never read `docs/research`, `docs/its`,
or follow symlinks/reparse escapes. Use server-owned deterministic file/byte
budgets `maxFiles=200000` and `maxBytes=4GiB`; `maxElapsed=120s` is a safety
abort that discards the entire authoritative snapshot rather than selecting a
timing-dependent prefix. Bound enumeration with
`maxTraversalEntries=1600000` and `maxTraversalDepth=64`, and reject any XML
document above `maxXmlBytes=64MiB` before DOM parsing. Open authoritative files
through directory handles (`openat` plus `O_NOFOLLOW`, with `O_NONBLOCK` on the
leaf, on Unix); on Windows open each single component relative to its already
opened parent directory handle, reject every reparse handle, retain the chain,
and verify the final handle's exact contained path and stable volume/file ID
before and after reading, failing closed when identity is unavailable. Re-observe
opened files around reads; concurrent identity, size, metadata, path-set, or
mapping change makes the snapshot unavailable/retryable. Preserve an in-memory
path-to-hash manifest for exact pre/post diffs. After the final hook, revalidate
present files and every absence tombstone; bound each final reread to the
previously captured file length plus one rather than the global byte budget.
Exclude `.git`, `.build`,
`target`, and `dist` directories only inside registered subtrees.

- [ ] **Step 5: Re-run and commit**

Run:

```bash
cargo test --locked -p unica-coder source_snapshot -- --nocapture
cargo test --locked -p unica-coder project_sources -- --nocapture
cargo test --locked -p unica-coder discovery -- --nocapture
cargo test --locked -p unica-coder
cargo fmt --all -- --check
cargo clippy --locked -p unica-coder --all-targets -- -D warnings
python3 tests/ci/test_product_contracts.py
```

```bash
git add crates/unica-coder/src/application/discovery \
  crates/unica-coder/src/application/mod.rs \
  crates/unica-coder/src/application/tool_contracts.rs \
  crates/unica-coder/src/domain \
  crates/unica-coder/src/infrastructure \
  crates/unica-coder/Cargo.toml Cargo.lock
git -c commit.gpgsign=false commit -m "feat: добавить content source snapshots"

git add spec/architecture/extension-point-discovery.md \
  docs/superpowers/plans/2026-07-17-project-discovery-receipts.md \
  tests/ci/test_product_contracts.py
git -c commit.gpgsign=false commit -m "docs: синхронизировать source snapshot contract"
```

### Task 5: Platform XML catalog, bindings, forms, and support providers

**Files:**
- Create: `crates/unica-coder/src/infrastructure/discovery/mod.rs`
- Create: `crates/unica-coder/src/infrastructure/discovery/platform_xml.rs`
- Create: `crates/unica-coder/src/infrastructure/discovery/platform_callbacks.rs`
- Create: `crates/unica-coder/src/infrastructure/discovery/support.rs`
- Modify: `crates/unica-coder/src/infrastructure/mod.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/common.rs`
- Test: new modules and `tests/fixtures/project_discovery/platform_xml/`

- [ ] **Step 1: Add failing provider fixtures/tests for seven declarative flows**

Fixtures must cover event subscription, form command/action, common command,
scheduled job, HTTP route, exchange-plan subscription, and report/data-
processor form ownership. Include wrong binding, malformed XML, lexical decoy,
registered hard decoy, and source-set mismatch cases.

- [ ] **Step 2: Run and verify RED**

Run: `cargo test --locked -p unica-coder discovery::platform_xml -- --nocapture`

- [ ] **Step 3: Implement typed XML readers**

Read `Configuration.xml` child registrations first, then only registered object
files. Parse local-name namespace-insensitively but retain path and best-effort
line provenance. Return typed metadata and binding facts; never render and
reparse human text. Report malformed registered files as failed material
checks, not absent objects.

- [ ] **Step 4: Add versioned platform callback catalog**

Model platform lifecycle and command callbacks by platform script variant,
metadata kind, module kind, method, export requirement, and signature. This is
a platform API registry, not a business-term dictionary. Unknown callback
variants remain `unknown`.

- [ ] **Step 5: Make support parsing error-aware**

Refactor existing `ParentConfigurations.bin` parsing so missing, malformed,
I/O failure, and explicit not-under-support are distinct typed outcomes. The
legacy display renderer consumes the typed result; discovery receives
`SupportFactState` directly.

- [ ] **Step 6: Re-run and commit**

Run: `cargo test --locked -p unica-coder discovery::platform_xml -- --nocapture`

```bash
git add crates/unica-coder/src/infrastructure tests/fixtures/project_discovery
git -c commit.gpgsign=false commit -m "feat: добавить typed platform xml evidence"
```

### Task 6: Bounded typed BSL definitions, search, and call edges

**Files:**
- Create: `crates/unica-coder/src/infrastructure/discovery/bsl.rs`
- Modify: `crates/unica-coder/src/infrastructure/discovery/mod.rs`
- Modify: `crates/unica-coder/src/infrastructure/workspace_services.rs`
- Modify: `crates/unica-coder/src/infrastructure/workspace_index.rs`
- Test: new module and BSL fixtures under `tests/fixtures/project_discovery/`

- [ ] **Step 1: Write failing typed-provider tests**

Cover exact definitions, export/signature, comments and strings as lexical
decoys, direct calls, qualified common-module calls, unresolved dynamic calls,
duplicate/conflicting definitions, bounds, unavailable index/service, and
stable ordering.

- [ ] **Step 2: Run and verify RED**

Run: `cargo test --locked -p unica-coder discovery::bsl -- --nocapture`

- [ ] **Step 3: Add typed workspace-service responses**

Expose typed query DTOs from the workspace/index service; keep SQLite paths
private. Preserve existing display-oriented `code.*` behavior as a separate
consumer. For graph MCP, accept `structuredContent` or one strictly parsed JSON
payload at the infrastructure boundary; never concatenate text sections and
parse them in the application layer. Bump `SERVICE_SCHEMA_VERSION` and carry
the structured value through `ServiceResponse` and
`WorkspaceServiceBslOutput`, so a live process with the old text-only contract
cannot be silently reused.

- [ ] **Step 4: Implement explicit bounded source fallback**

When the typed index/graph service is unavailable, a contained fallback may
scan only manifest-listed BSL files within time/file/byte/result bounds. It
must tokenize comments/strings, produce typed definitions and conservative
static calls, mark unsupported dynamic syntax as bounded/unknown, and identify
itself as a weaker provider. It must not open SQLite.

- [ ] **Step 5: Re-run and commit**

Run: `cargo test --locked -p unica-coder discovery::bsl -- --nocapture`

```bash
git add crates/unica-coder/src/infrastructure
git -c commit.gpgsign=false commit -m "feat: добавить typed bsl evidence providers"
```

### Task 7: Concrete discovery orchestration and eight mechanism families

**Files:**
- Modify: `crates/unica-coder/src/application/discovery/use_case.rs`
- Modify: `crates/unica-coder/src/infrastructure/discovery/mod.rs`
- Create: `crates/unica-coder/src/infrastructure/discovery/providers.rs`
- Create: `crates/unica-coder/src/application/discovery/mechanisms.rs`
- Test: application and infrastructure discovery modules

- [ ] **Step 1: Write failing end-to-end in-process discovery tests**

For each of eight families, include supported primary, supported alternative,
contradicted exact binding, unknown provider gap, lexical decoy, and hard decoy.
Families 7 and 8 use only the narrow v1 proof boundaries in the spec. Assert
required/forbidden related artifacts, edges, candidates, evidence levels,
verdict, checks, and receipt eligibility.

- [ ] **Step 2: Run and verify RED**

Run: `cargo test --locked -p unica-coder application::discovery::mechanisms::tests -- --nocapture`

Expected RED output must list the intended mechanism tests; zero executed tests
is a failed verification even when Cargo exits successfully.

- [ ] **Step 3: Wire concrete providers into the use case**

Build related artifacts from caller concepts/search terms/known artifacts,
traverse only typed edges to bounded depth, classify actionable hooks, and
validate exact proposals. Keep canonical stable ordering and retain every
material provider issue. Unsupported EDT and broad family variants return
`unsupported_source_format` or `unsupported_mechanism_variant` checks. EDT uses
the Task 4 diagnostic snapshot, runs no Platform XML/BSL inference provider,
and can never become receipt-eligible in v1.
Use a deterministic fake `ReceiptIssuerPort` in mechanism tests to evaluate
eligibility without claiming persistence; the real issuer is wired only after
the shared resolver and receipt store exist.

- [ ] **Step 4: Re-run and commit**

Run: `cargo test --locked -p unica-coder application::discovery::mechanisms::tests -- --nocapture`

```bash
git add crates/unica-coder/src/application/discovery crates/unica-coder/src/infrastructure/discovery
git -c commit.gpgsign=false commit -m "feat: реализовать project discovery mechanisms"
```

### Task 8: Shared CFE mutation-intent and target resolver

**Files:**
- Create: `crates/unica-coder/src/domain/cfe_method_patch.rs`
- Modify: `crates/unica-coder/src/domain/mod.rs`
- Create: `crates/unica-coder/src/application/discovery_guard/target_resolvers/mod.rs`
- Create: `crates/unica-coder/src/application/discovery_guard/target_resolvers/cfe_patch_method.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/cfe.rs`
- Test: resolver, handler, and application fake-port tests

- [ ] **Step 1: Write failing resolver parity tests**

Cover common/object/form modules, accepted aliases and case normalization,
invalid module paths, method identifiers, all interceptor kinds, extension
source-set mismatch, outside/EDT destination, exact artifact path, target
mismatch, all allowed execution contexts, procedure/function defaulting, and
exact mismatches for `Context` and `IsFunction`.

- [ ] **Step 2: Run and verify RED**

Run: `cargo test --locked -p unica-coder cfe_method_patch -- --nocapture`

- [ ] **Step 3: Extract one shared parser/path resolver**

Both receipt issuance, pre-mutation guard, and native handler must consume
`CfeMethodPatchPlan`. The plan owns normalized module identity, method,
interceptor/change kind, execution context, procedure/function kind,
destination source-set identity, one contained BSL artifact, and a canonical
resolver-arguments digest. Remove duplicate parsing from `cfe.rs`; reject
unknown context values and inconsistent proposal target/arguments.

- [ ] **Step 4: Reject duplicate interceptor definitions before writes**

Use the shared plan to derive decorator and generated method identity. Make a
duplicate decorator or generated procedure/function a pre-write error and add
parity cases to the existing MCP script suite.

- [ ] **Step 5: Re-run parity tests and commit**

Run: `cargo test --locked -p unica-coder cfe_method_patch -- --nocapture`

```bash
git add crates/unica-coder/src/domain crates/unica-coder/src/application/discovery_guard crates/unica-coder/src/infrastructure/native_operations/cfe.rs tests/ci/test_unica_mcp_script_parity.py
git -c commit.gpgsign=false commit -m "refactor: унифицировать cfe method patch scope"
```

### Task 9: Receipt model, atomic store, composite baseline, and lease

**Files:**
- Create: `crates/unica-coder/src/domain/discovery_receipts.rs`
- Modify: `crates/unica-coder/src/domain/mod.rs`
- Create: `crates/unica-coder/src/infrastructure/atomic_file.rs`
- Modify: `crates/unica-coder/src/infrastructure/runtime_jobs.rs`
- Create: `crates/unica-coder/src/infrastructure/discovery_receipts.rs`
- Modify: `crates/unica-coder/src/infrastructure/mod.rs`
- Create: `crates/unica-coder/src/application/receipts/mod.rs`
- Create: `crates/unica-coder/src/application/receipts/service.rs`
- Modify: `crates/unica-coder/src/application/discovery/use_case.rs`
- Modify: `crates/unica-coder/src/application/ports.rs`
- Modify: `crates/unica-coder/src/application/mod.rs`
- Test: all new modules with fake source/evidence ports

- [ ] **Step 1: Write failing state-machine and persistence tests**

Test issue only when every selected proposal is supported and has a resolvable
typed intent; no issue for contradicted/unknown/blocker/ambiguity/no resolver;
strict schema; corrupt record; path traversal ID; every mismatch reason; stale
composite fingerprint; unknown schema; atomic reader visibility; process-local
and OS-lock contention; persistent lock inode; stale revision; drop releases;
no TTL validity; analysis + N sorted/deduplicated destination snapshots.

```rust
#[test]
fn grants_do_not_create_cross_product_authority() {
    let receipt = fixture.issue(vec![
        grant("A", Before, "Ext1", "НаСервере", Procedure),
        grant("B", After, "Ext2", "НаКлиенте", Function),
    ]);
    for forbidden in [
        scope("A", After, "Ext1", "НаСервере", Procedure),
        scope("B", Before, "Ext2", "НаКлиенте", Function),
        scope("A", Before, "Ext2", "НаСервере", Procedure),
        scope("A", Before, "Ext1", "НаКлиенте", Procedure),
        scope("A", Before, "Ext1", "НаСервере", Function),
    ] {
        assert!(fixture.evaluate(&receipt, forbidden).is_err());
    }
}

#[test]
fn only_one_lease_reaches_the_handler_revision() {
    let first = fixture.try_lease(0).unwrap();
    assert_eq!(fixture.try_lease(0).unwrap_err(), ReceiptReason::ReceiptBusy);
    drop(first);
}
```

- [ ] **Step 2: Run and verify RED**

Run: `cargo test --locked -p unica-coder discovery_receipts::tests -- --nocapture`

Expected output must show store, lease, multi-grant, and issuance tests; zero
executed tests is failure.

- [ ] **Step 3: Extract and implement the shared atomic-file primitive**

Move the tested same-directory temp/write/sync/Windows `MoveFileExW`/parent-sync
logic out of `runtime_jobs.rs` into `atomic_file.rs`; both runtime jobs and
receipts use it. Persist receipts under
`${cache_root}/discovery/v1/<workspace-key>/{receipts,locks}`. Derive the
workspace key from a domain-separated canonical root digest with Windows path
normalization and exact Unix bytes. Validate `discovery_receipt_<uuid>` before
joining paths.

- [ ] **Step 4: Implement exclusive lease and revision service**

Combine an in-process non-blocking ID registry with `fs2::try_lock_exclusive`.
Never delete lock files. Under lock, reread schema/revision/state and recheck
the current composite snapshot. Hold the lease object until advance/revoke.

- [ ] **Step 5: Integrate real receipt issuance into validate mode**

Inject `ReceiptIssuerPort` in the application composition root and resolve every
mutation intent with the Task 8 resolver. Store only digests, canonical
identities, atomic grants including normalized parameters, fingerprints,
versions, and audit timestamps; never task/source text. Return the public
receipt view in `DiscoveryReport`. No application module imports infrastructure
directly.

- [ ] **Step 6: Re-run and commit**

Run: `cargo test --locked -p unica-coder discovery_receipts::tests -- --nocapture`

Run: `cargo test --locked -p unica-coder application::discovery -- --nocapture`

```bash
git add crates/unica-coder/src/application crates/unica-coder/src/domain crates/unica-coder/src/infrastructure
git -c commit.gpgsign=false commit -m "feat: добавить discovery receipt store и lease"
```

### Task 10: Discovery guard pipeline, rolling advance, and reconciliation

**Files:**
- Create: `crates/unica-coder/src/application/discovery_guard/mod.rs`
- Create: `crates/unica-coder/src/application/discovery_guard/model.rs`
- Create: `crates/unica-coder/src/application/discovery_guard/guard.rs`
- Modify: `crates/unica-coder/src/application/operation_descriptors.rs`
- Modify: `crates/unica-coder/src/application/mod.rs`
- Modify: `crates/unica-coder/src/application/ports.rs`
- Modify: `crates/unica-coder/src/domain/events.rs`
- Modify: `crates/unica-coder/src/infrastructure/mod.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/registry.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/cfe.rs`
- Test: application pipeline tests with fake handler/store/snapshot

- [ ] **Step 1: Write failing guard matrix and ordering tests**

Cover `off/observe/warn/deny × not_required/advisory_only/enforceable`, support
before discovery, dry-run no lease, missing/all mismatch reason codes,
two-step in-scope advance, failed-zero-diff unchanged, failed-partial revoke,
out-of-scope revoke, one concurrent handler, other linked receipts reconciled,
build no-change preserved, broad changed event revoked, environment/workspace
configuration precedence, default `observe`, and invalid-mode rejection.
Also prove a native handler carries exact typed effects through registry,
adapter, `HandlerOutcome`, and the application pipeline without parsing display
fields.

- [ ] **Step 2: Run and verify RED**

Run: `cargo test --locked -p unica-coder discovery_guard -- --nocapture`

- [ ] **Step 3: Extend operation descriptors and validate invariants**

Add `NotRequired`, `AdvisoryOnly`, `Enforceable` separately from configured
mode. Only `cfe-patch-method` is enforceable in v1. Descriptor-wide tests prove
every enforceable operation has a resolver and every public/native operation is
classified.

Read `UNICA_DISCOVERY_GUARD_MODE` first, then top-level `discoveryGuard` from
the nearest applicable `.v8-project.json`, and otherwise use `observe`.
Accepted values are exact; invalid configuration is an error and there is no
per-call override.

- [ ] **Step 4: Integrate guard into application-owned call order**

First introduce `MutationEffects { changed_artifacts, coverage }` in the native
outcome and propagate it through `registry.rs`, `NativeOperationAdapter`,
`HandlerOutcome`, and fake execution ports. `cfe.patch_method` returns its exact
canonical BSL path. `AdapterOutcome.changes/artifacts` remain display-only.

The order is arguments/workspace/source/path, support guard, scope resolution,
discovery guard and applied-call lease, handler, typed effects + post snapshot,
advance/revoke, events/cache, other-receipt reconciliation, invalidation, result
assembly. `deny` returns `Ok(OperationResult { ok: false, ... })`, not transport
`Err`. `observe` never changes handler outcomes; `warn` adds one stable warning.

- [ ] **Step 5: Verify post-mutation scope**

Compare pre/post manifests and typed effects. Successful exact in-scope writes
advance revision and composite fingerprint. Unknown or out-of-scope writes
revoke and return `post_mutation_scope_violation`. Failed/no-diff leaves the
record unchanged; failed/changed revokes.

Never acquire another receipt lock while the applied receipt lease is held.
Release it after advance/revoke, then reconcile other linked receipt IDs in
sorted order with non-blocking locks. A busy record remains active on disk but
its authoritative fingerprint is stale, so the next validation rejects it
before invoking a handler.

- [ ] **Step 6: Re-run and commit**

Run: `cargo test --locked -p unica-coder discovery_guard -- --nocapture`

```bash
git add crates/unica-coder/src/application crates/unica-coder/src/domain/events.rs
git -c commit.gpgsign=false commit -m "feat: добавить discovery guard и rolling receipts"
```

### Task 11: Shadow observation journal, counters, audit, and replay

**Files:**
- Create: `crates/unica-coder/src/domain/discovery_observations.rs`
- Modify: `crates/unica-coder/src/domain/mod.rs`
- Create: `crates/unica-coder/src/application/discovery_observations.rs`
- Modify: `crates/unica-coder/src/application/mod.rs`
- Modify: `crates/unica-coder/src/application/ports.rs`
- Create: `crates/unica-coder/src/infrastructure/discovery_observations.rs`
- Modify: `crates/unica-coder/src/infrastructure/mod.rs`
- Create: `crates/unica-coder/src/interfaces/discovery_observation_audit.rs`
- Modify: `crates/unica-coder/src/interfaces/mod.rs`
- Modify: `crates/unica-coder/src/main.rs`
- Test: all new modules and application parity fakes

- [ ] **Step 1: Write failing journal, redaction, and replay tests**

Cover append concurrency, OS-locking, schema rejection, corrupt-line reporting,
bounded rollover, aggregate counters, deterministic pure replay, descriptor and
source-format breakdown, sequence/timing fields excluded from input digests,
and absence of task text, source text, raw arguments, absolute paths, and raw
artifact names. Inject a failing observation port and prove `observe`, `warn`,
and `deny` handler/receipt outcomes remain byte-for-byte unchanged.
Assert `off` performs no guard evaluation and emits no shadow decision.

```rust
#[test]
fn telemetry_failure_never_changes_guard_or_handler_outcome() {
    let expected = fixture().observer(NoopObserver).apply();
    let actual = fixture().observer(FailingObserver("disk_full")).apply();
    assert_eq!(actual.authoritative_view(), expected.authoritative_view());
    assert_eq!(actual.operator_diagnostics, vec!["discovery_observation_write_failed"]);
}

#[test]
fn replay_reproduces_policy_without_source_text() {
    let record = observation_fixture().record();
    assert_eq!(replay(&record.replay_input), record.decision);
    assert!(!serde_json::to_string(&record).unwrap().contains("Procedure"));
}
```

- [ ] **Step 2: Run and verify RED**

Run: `cargo test --locked -p unica-coder discovery_observations::tests -- --nocapture`

Expected output lists journal, counters, redaction, telemetry-failure, and
replay tests; zero executed tests is failure.

- [ ] **Step 3: Implement non-authoritative observation port and journal**

Emit after the operation outcome is known. Store a schema-versioned JSONL
journal and atomic counters below the workspace-keyed discovery cache. Each
record contains only descriptor/source-format labels, mode/requirement/
decision/reason, hashed workspace/targets/resolver input/snapshots/receipt/
effects/handler outcome, receipt revision, parity, sequence, and timing. Use
the shared atomic-file primitive for counters and a persistent OS lock for
journal append/rollover. Reporting failure is operator-only and cannot alter
authoritative output.

- [ ] **Step 4: Implement pure replay and maintainer audit CLI**

Add `unica --discovery-observations-audit --workspace <path> [--replay]`
without registering an MCP tool. Strictly load records, report corrupt/unknown
schema entries, recompute decisions from comparison predicates, print aggregate
promotion metrics, and exit nonzero on parity/replay failure. It never prints
raw task/source/mutation text because those fields are not persisted.

- [ ] **Step 5: Re-run and commit**

Run: `cargo test --locked -p unica-coder discovery_observations::tests -- --nocapture`

```bash
git add crates/unica-coder/src/application crates/unica-coder/src/domain crates/unica-coder/src/infrastructure crates/unica-coder/src/interfaces crates/unica-coder/src/main.rs
git -c commit.gpgsign=false commit -m "feat: сохранять и воспроизводить discovery observations"
```

### Task 12: Public MCP schema and typed common result envelope

**Files:**
- Create: `crates/unica-coder/src/application/operation_result.rs`
- Modify: `crates/unica-coder/src/application/mod.rs`
- Modify: `crates/unica-coder/src/application/ports.rs`
- Modify: `crates/unica-coder/src/application/tool_contracts.rs`
- Modify: `crates/unica-coder/src/interfaces/mcp.rs`
- Modify: `tests/ci/test_unica_mcp_smoke.py`
- Modify: `tests/ci/test_product_contracts.py`

- [ ] **Step 1: Write failing schema, dispatch, and stdio tests**

Assert: exactly one `unica.project.discover`; strict nested schema; only `cwd`
from common args; explore/validate conditional validation on direct
`call_tool`; typed top-level `data`; separate `discoveryGuard`; discovery is
read-only; `discoveryReceipt` accepted only by classified mutations; no
public/internal analyzer server. Add source-readiness cases proving that
`DiscoveryError::SourceReadiness` returns a normal `ok=false` operation result,
not a transport error, with exact typed
`data.sourceReadiness={reasonCode,retryable,sourceSet,role}` and no discovery
report; clients never parse `errors[]` for the code.
Add the parallel snapshot-capture cases proving that
`DiscoveryError::SnapshotCapture` returns `ok=false` with exact typed
`data.snapshotCapture={reasonCode,retryable}`, no partial discovery report, and
no transport error; the display-only detail is not semantic client data.
Add a stdio test that sends a request line over 4 MiB and proves it is rejected
before JSON deserialization, while a boundary-sized valid request is still
handled.

- [ ] **Step 2: Run and verify RED**

Run: `cargo test --locked -p unica-coder tool_contracts -- --nocapture`

Run:
`python3.12 -m unittest tests.ci.test_unica_mcp_smoke tests.ci.test_product_contracts -v`

- [ ] **Step 3: Refactor the common envelope constructors**

Move repeated result assembly into `OperationResult::from_handler`,
`::discovery`, and `::policy_block`. Add typed optional `data` and
`discovery_guard` without overloading existing runtime diagnostics. Define
`OperationData::Discovery(report)` and
`OperationData::SourceReadiness(SourceReadinessData)` and
`OperationData::SnapshotCapture(SnapshotCaptureData)` as disjoint variants;
`SourceReadinessData` has exactly `reasonCode`, `retryable`, `sourceSet`, and
`role` (`analysis` or `destination`); `SnapshotCaptureData` has exactly
`reasonCode` and `retryable`.

- [ ] **Step 4: Register and dispatch `unica.project.discover` directly in application**

Add `ToolHandler::ProjectDiscover`, but do not route it through
`AdapterOutcome`. A completed analysis returns
`OperationData::Discovery(report)`; typed source-readiness failure returns
`ok=false` plus `OperationData::SourceReadiness`, while typed snapshot-capture
failure returns `ok=false` plus `OperationData::SnapshotCapture`; neither path
returns a partial report.
Generate nested JSON schema with `additionalProperties:false` at every level
and retain `serde`/semantic runtime validation for non-MCP calls.

Make `UnicaApplication` own injected execution ports, discovery dependencies,
receipt service, guard, and observation port through a composition-root
constructor plus test constructor. Update the exhaustive handler match in
`application/ports.rs` so `ProjectDiscover` is rejected there as
application-owned rather than accidentally routed to infrastructure.

- [ ] **Step 5: Re-run and commit**

Run: `cargo test --locked -p unica-coder --lib -- --nocapture`

Run:
`python3.12 -m unittest tests.ci.test_unica_mcp_smoke tests.ci.test_product_contracts -v`

```bash
git add crates/unica-coder/src/application crates/unica-coder/src/interfaces tests/ci
git -c commit.gpgsign=false commit -m "feat: опубликовать unica.project.discover"
```

### Task 13: Gold corpus, receipt state machine, and metamorphic anti-overfit suite

**Files:**
- Create: `tests/fixtures/project_discovery/corpus.json`
- Create/extend: `tests/fixtures/project_discovery/platform_xml/**`
- Create: `crates/unica-coder/src/application/discovery/corpus_tests.rs`
- Create: `crates/unica-coder/src/application/discovery/metamorphic_tests.rs`
- Modify: `spec/acceptance/unica-mcp-validation.md`
- Modify: `tests/ci/test_unica_mcp_smoke.py`
- Create: `scripts/ci/run-packaged-discovery-corpus.py`
- Create: `tests/ci/test_packaged_discovery_corpus.py`

- [ ] **Step 1: Add the declarative 48-case corpus and failing evaluator**

For eight families × six cases, record required/forbidden related artifacts,
edges, candidates, minimum evidence levels, exact verdict/check severity,
receipt eligibility, and guard decision. Add the 12 receipt state-machine cases
from the spec.

- [ ] **Step 2: Generate at least 20 deterministic variants per base case**

Systematically rename all domain identifiers with the query, permute files and
provider records, duplicate evidence, add 1–20 registered decoys, stale source
state, fault providers, and inject conflicts. Generated variants live in test
memory, not as thousands of checked-in files.

- [ ] **Step 3: Run and verify failures, then close every gap in production code**

Run: `cargo test --locked -p unica-coder application::discovery::corpus_tests -- --nocapture`

Expected output must show 48 base, 12 receipt, and generated metamorphic case
groups; zero executed tests is failure.

No expectation is weakened to make production pass. False supported, false
contradicted under incomplete coverage, incorrect receipt, scope expansion,
lost blocker, or nondeterminism is zero-tolerance.

- [ ] **Step 4: Add deterministic quality metrics assertions**

Compute the spec's candidate/edge/artifact precision and recall, verdict
recalls, unknown recall, deterministic replay, and receipt state-machine pass
rate. Assert thresholds and print per-family failures.

- [ ] **Step 5: Re-run and commit**

Run: `cargo test --locked -p unica-coder application::discovery::corpus_tests -- --nocapture`

Add a package-binary corpus runner that speaks MCP stdio to an explicit
`--binary` path and executes the same declarative corpus. Its CI test builds a
temporary package, locates only the packaged binary, and fails if the runner
falls back to `cargo run` or the source tree.

```bash
git add tests/fixtures/project_discovery crates/unica-coder/src/application/discovery spec/acceptance scripts/ci/run-packaged-discovery-corpus.py tests/ci/test_packaged_discovery_corpus.py
git -c commit.gpgsign=false commit -m "test: добавить discovery gold corpus"
```

### Task 14: Skill, provenance, package, and rollout synchronization

**Files:**
- Create: `plugins/unica/skills/project-discovery/SKILL.md`
- Create: `plugins/unica/skills/project-discovery/agents/openai.yaml`
- Modify: `plugins/unica/provenance/skill-upstreams.json`
- Modify: `plugins/unica/README.md`
- Modify: `scripts/ci/release-assessment.py`
- Create: `scripts/ci/discovery-shadow-replay.py`
- Modify: `tests/ci/test_unica_skills.py`
- Modify: `tests/ci/test_skill_provenance.py`
- Modify: `tests/ci/test_package_unica_plugin.py`
- Modify: `tests/ci/test_release_assessment.py`
- Modify: package metadata only where existing generated-package tests require it

- [ ] **Step 1: Write failing skill/provenance/package tests**

Require MCP-first usage, both modes, proposal validation, receipt handoff,
degradation semantics, no direct scripts, provenance entry, packaged inclusion,
and release assessment that says guard default `observe` and promotion pending
real shadow evidence.

- [ ] **Step 2: Run and verify RED**

Run:
`python3.12 -m unittest tests.ci.test_unica_skills tests.ci.test_skill_provenance tests.ci.test_package_unica_plugin tests.ci.test_release_assessment -v`

- [ ] **Step 3: Add the MCP-first discovery skill and package docs**

The skill teaches the agent to explore, select a proposal, validate it, pass
the receipt only to the exact mutation, and treat `unknown` as a blocker to
receipt issuance. It does not execute packaged scripts or promise deny mode.
Add matching OpenAI agent metadata. Extend the real release-assessment script,
not only its tests, to run discovery against the pinned BSP workspace, persist
only a sanitized typed-outcome/observation replay capsule, and invoke the audit
path without storing task or source text. `discovery-shadow-replay.py` is a
maintainer CI utility and is not referenced by the prompt-visible skill.

- [ ] **Step 4: Re-run package tests and inspect generated artifact**

Stage the new skill/agent/provenance files before invoking the package builder,
because it copies only `git ls-files`:

```bash
git add plugins/unica/skills/project-discovery plugins/unica/provenance/skill-upstreams.json plugins/unica/README.md scripts/ci tests/ci
python3.12 scripts/ci/build-unica-tools.py --target darwin-arm64 --repo-root . --out-dir /tmp/unica-discovery-tools --work-dir /tmp/unica-discovery-work
python3.12 scripts/ci/package-unica-plugin.py --repo-root . --tools-root /tmp/unica-discovery-tools --out-dir /tmp/unica-discovery-package --target darwin-arm64
```

Run:
`python3.12 -m unittest tests.ci.test_unica_skills tests.ci.test_skill_provenance tests.ci.test_package_unica_plugin tests.ci.test_release_assessment -v`

Unpack the produced archive to a temporary directory and verify
the skill, provenance, MCP manifest, native binary, and no accidental source or
research corpus. On a non-Apple host, use its supported native target instead
of claiming execution of a foreign binary.

- [ ] **Step 5: Commit**

```bash
git add plugins/unica tests/ci
git -c commit.gpgsign=false commit -m "docs: синхронизировать discovery skill и package"
```

### Task 15: Full verification, independent review, and delivery

**Files:** all changed files

- [ ] **Step 1: Run formatting and static checks**

Run: `cargo fmt --all -- --check`

Run: `cargo clippy --locked --workspace --all-targets -- -D warnings`

- [ ] **Step 2: Run the complete Rust suite**

Run: `cargo test --locked --workspace -- --nocapture`

Expected: all baseline and discovery tests pass, including the full generated
corpus and concurrency tests.

- [ ] **Step 3: Run the complete Python CI suite with local signing disabled**

Run:
`env GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=commit.gpgsign GIT_CONFIG_VALUE_0=false python3.12 -m unittest discover -s tests/ci -v`

- [ ] **Step 4: Run MCP/package smoke from the generated artifact**

Build the release binary and package through the repository's existing scripts,
invoke MCP `tools/list`, `explore`, `validate`, valid receipt mutation dry-run,
missing receipt observe/warn/deny matrix, stale receipt, and two-step advance.
Do not use the source-tree binary for the final package proof.

Run `scripts/ci/run-packaged-discovery-corpus.py --binary <packaged-unica>` and
require all 48 base cases, 12 receipt cases, and deterministic variants to pass
through that packaged binary. Run the pinned BSP discovery shadow workflow and
`<packaged-unica> --discovery-observations-audit --workspace <bsp> --replay`;
verify its capsule contains no source/task text.

- [ ] **Step 5: Run cross-platform-sensitive tests**

Exercise path case normalization, atomic replacement abstractions, persistent
lock files, non-UTF8 rejection behavior, and Windows destination-path fixtures.
If no Windows runner is available, keep deny disabled and record the missing
runner as a release-assessment limitation; do not claim cross-platform proof.

- [ ] **Step 6: Request independent spec and code review**

Use `superpowers:requesting-code-review` twice: first for spec/contract coverage,
then for code quality, concurrency, containment, false-proof, package, and
backward-compatibility risks. Fix every confirmed finding through a new failing
test and rerun the relevant focused and full suites.

- [ ] **Step 7: Audit scope and history**

Run: `git diff --check`

Run: `git status --short`

Run: `git log --oneline --decorate origin/main..HEAD`

Confirm only intended files changed and every task commit is reviewable.

- [ ] **Step 8: Finish the branch**

Use `superpowers:finishing-a-development-branch`. Push and open a ready PR only
after all local proof is green. The PR must state that the implementation is
final in `observe`, while `warn`/`deny` activation remains evidence-gated by
the accepted live thresholds.
