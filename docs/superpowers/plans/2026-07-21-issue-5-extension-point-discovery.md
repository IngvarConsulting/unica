# Issue #5 Extension-Point Discovery Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deliver the read-only `unica.project.discover` task-only preflight and the mandatory prompt-visible `extension-point-discovery` skill, with packaged UT 11.5 acceptance, so the pull request can close issue #5 while only referencing the remaining #161 proposal/receipt work.

**Architecture:** A typed discovery request is parsed once in the application layer and dispatched through dedicated discovery ports, never through presentation `stdout`. A pure use case combines bounded provider outcomes into evidence, structural and runtime-flow graphs, candidates, warnings, missing checks, and an analysis snapshot. Infrastructure supplies contained Platform XML/form/BSL/support facts; the plugin skill makes the public read-only tool a mandatory gate before planning or mutation.

**Tech Stack:** Rust 2021, `serde`, `serde_json`, `roxmltree`, `sha2`, `rusqlite`, existing Unica MCP/application/platform infrastructure, Python 3.12 `unittest`, Codex plugin skills.

## Global Constraints

- Preserve one public MCP server named `unica`; the only new public tool is the non-mutating `unica.project.discover` and every prompt-visible fallback remains a public `unica.*` call.
- Implement Slice A and Slice B only. Do not add proposal validation, `proposedExtensionPoints`, discovery receipts, leases, mutation guards, `dryRun`, `confirm`, `sourceSet`, raw adapter arguments, arbitrary paths, scores, or confidence scalars.
- Required request fields are `mode="explore"` and a non-empty `task`; a request containing only `cwd`, `mode`, and `task` is first-class.
- Optional request fields are only `cwd`, `sourceDir`, `concepts`, `searchTerms`, `objects`, and `limits`; unknown fields are rejected.
- Validate UTF-8 byte bounds exactly: task `1..=8192`; at most 64 unique concepts of `1..=256` bytes; at most 128 unique search terms of `1..=256` bytes; at most 128 unique object references of `1..=1024` bytes.
- Limit defaults and maxima are exactly `maxFiles=20000`, `maxBytes=268435456`, `maxEvidence=2000`, `maxCandidates=100`, and `maxGraphDepth=12`; callers may lower but not raise them.
- JSON Schema describes byte bounds in prose and does not use character-based `maxLength` for them; runtime validation is authoritative.
- Reject absolute, escaping, ambiguous, missing, or otherwise uncontained `sourceDir` before providers run. Default selection is discovery-specific: exactly one eligible configuration source root, not the existing generic “named main wins” rule.
- Derive deterministic lexical concepts from the task without a business-domain dictionary; explicit concepts and search terms augment task-derived exploration and retain provenance.
- Every provider returns exactly one exhaustive `ProviderOutcome<T>` variant: `Complete`, `Bounded`, `Unavailable`, `Failed`, or `ContractViolation`.
- Only an empty `Complete` batch is negative evidence. Every other non-complete outcome makes the report partial and produces a typed diagnostic/missing check; `ContractViolation` records are excluded from graph promotion.
- Keep `contains`, `defines`, and data/form binding relationships structural. Only typed callback, action, event-subscription, or call-graph facts may become runtime-flow edges; lexical matches alone are never actionable candidates.
- Return typed data only at `OperationResult.data.discovery`, with `schemaVersion`, `complete|partial`, source, snapshot, concept provenance, provider outcomes/coverage, related artifacts, separate structural/runtime-flow edges, candidates with advisory typed recommendations, warnings, missing checks, and stable evidence IDs/locations.
- The analysis snapshot records resolved mapping identity and raw SHA-256 hashes of evidence-contributing files only. BOM and EOL bytes affect hashes; the snapshot is not a mutation receipt and makes no whole-workspace freshness claim.
- Evidence reads are bounded, contained, and made through verified regular-file handles. Reject symlinks/reparse points, non-regular files, final paths outside the selected root, and path/handle identity changes. Host-specific code stays under `infrastructure/platform`.
- Exclude only actual runtime-sidecar `ConfigDumpInfo.xml` content using `config_dump_info_xml_kind(bytes)`; do not exclude a legitimate external metadata object by filename alone.
- Production discovery paths return typed errors and contain no recoverable-input `unwrap`, `expect`, or `panic`; borrow `&str`, `&Path`, and slices where ownership is unnecessary; match every internal enum exhaustively.
- Keep deterministic ordering with structs, sorted vectors, `BTreeMap`, and `BTreeSet`; identical inputs and provider facts must serialize identically and produce the same digest.
- Follow strict red/green/refactor: write one failing behavior test, run it and confirm the intended failure, add the minimum production code, rerun focused and neighboring tests, then refactor while green.
- The mandatory skill calls discovery before planning and before mutating MCP/manual XML/BSL work, resolves material gaps only through public `unica.*`, stops when a material gap remains, and records selected/rejected points, evidence, support state, and non-material gaps.

---

## File map

- Create `spec/architecture/extension-point-discovery.md`: active architecture contract distilled from the approved design.
- Create `spec/decisions/0012-read-only-extension-point-discovery.md`: accepted Slice A/B decision and explicit Slice C/D exclusion.
- Modify `spec/decisions/README.md` and `tests/ci/test_product_contracts.py`: ADR registration and product-contract guard.
- Create `crates/unica-coder/src/domain/discovery.rs`; modify `domain/mod.rs`: immutable identifiers, enums, provider facts, snapshot/report DTOs.
- Create `crates/unica-coder/src/application/discovery/{mod.rs,contract.rs,ports.rs,use_case.rs}`; modify `application/mod.rs`, `application/ports.rs`, and `application/tool_contracts.rs`: strict request/schema, use case, typed application dispatch, and `OperationResult.data.discovery`.
- Create `crates/unica-coder/src/infrastructure/platform/contained_file.rs`; modify `infrastructure/platform/mod.rs`: verified bounded raw reads.
- Create `crates/unica-coder/src/infrastructure/discovery/{mod.rs,inventory.rs,metadata.rs,forms.rs,bsl.rs,support.rs}`; modify `infrastructure/mod.rs`, `infrastructure/application_ports.rs`, `infrastructure/source_roots.rs`, `infrastructure/workspace_index.rs`, and the narrow support parser seam in `native_operations/common.rs`.
- Modify `crates/unica-coder/src/interfaces/mcp.rs`: public listing/schema/call assertions.
- Create `tests/fixtures/extension-point-discovery/ut115/**`; modify `tests/ci/test_unica_mcp_smoke.py`: source task-only acceptance.
- Create `plugins/unica/skills/extension-point-discovery/{SKILL.md,agents/openai.yaml}`; modify provenance, skill, package, release-smoke, release-assessment, README, and acceptance-contract files named in Task 8.

### Task 1: Freeze the active architecture and issue boundary

**Files:**
- Create: `spec/architecture/extension-point-discovery.md`
- Create: `spec/decisions/0012-read-only-extension-point-discovery.md`
- Modify: `spec/decisions/README.md`
- Modify: `tests/ci/test_product_contracts.py`

**Interfaces:**
- Consumes: approved design `docs/superpowers/specs/2026-07-21-issue-5-extension-point-discovery-design.md`.
- Produces: ADR-0012 and an active architecture document that later code/tests can cite verbatim.

- [ ] **Step 1: Write the failing product-contract test**

Add `ProductContractTests.test_extension_point_discovery_decision_is_registered_and_narrow`:

```python
def test_extension_point_discovery_decision_is_registered_and_narrow(self) -> None:
    repo_root = Path(__file__).resolve().parents[2]
    index = (repo_root / "spec/decisions/README.md").read_text(encoding="utf-8")
    adr = repo_root / "spec/decisions/0012-read-only-extension-point-discovery.md"
    architecture = repo_root / "spec/architecture/extension-point-discovery.md"

    self.assertIn("ADR-0012", index)
    self.assertTrue(adr.is_file())
    self.assertTrue(architecture.is_file())
    joined = adr.read_text(encoding="utf-8") + architecture.read_text(encoding="utf-8")
    for token in (
        "unica.project.discover",
        "OperationResult.data.discovery",
        "ProviderOutcome",
        "analysis snapshot",
        "extension-point-discovery",
        "Slice C",
        "Slice D",
    ):
        self.assertIn(token, joined)
    self.assertNotIn("discovery authorizes mutation", joined)
```

- [ ] **Step 2: Verify RED**

Run `python3.12 -m unittest tests.ci.test_product_contracts.ProductContractTests.test_extension_point_discovery_decision_is_registered_and_narrow -v`.

Expected: FAIL because ADR-0012 and the architecture document do not exist.

- [ ] **Step 3: Write the architecture and ADR**

The architecture document must state the request/result fields, exact numeric bounds, provider-outcome semantics, graph-promotion rule, snapshot boundary, packaged UT acceptance, and Slice B stop/selection gate. ADR-0012 must use:

```markdown
# ADR-0012: Read-only extension-point discovery precedes typical configuration changes

- Status: accepted
- Date: 2026-07-21
- Issues: [#5](https://github.com/IngvarConsulting/unica/issues/5), [#161](https://github.com/IngvarConsulting/unica/issues/161)

## Decision

Unica exposes one non-mutating `unica.project.discover` operation and one mandatory
prompt-visible `extension-point-discovery` preflight. Typed providers return
`ProviderOutcome<T>` facts; the application creates evidence and an analysis snapshot
at `OperationResult.data.discovery`. This delivery does not authorize mutation.

Proposal validation remains Slice C. Receipts, leases, and mutation guards remain
Slice D and require a separate accepted decision.
```

Register the exact filename in `spec/decisions/README.md`.

- [ ] **Step 4: Verify GREEN**

Run the focused unittest from Step 2 and `python3.12 -m unittest tests.ci.test_product_contracts -v`.

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add spec/architecture/extension-point-discovery.md spec/decisions/0012-read-only-extension-point-discovery.md spec/decisions/README.md tests/ci/test_product_contracts.py
git commit -m "docs: accept read-only extension point discovery"
```

### Task 2: Add the strict typed request and schema

**Files:**
- Create: `crates/unica-coder/src/application/discovery/mod.rs`
- Create: `crates/unica-coder/src/application/discovery/contract.rs`
- Create: `crates/unica-coder/src/domain/discovery.rs`
- Modify: `crates/unica-coder/src/domain/mod.rs`
- Modify: `crates/unica-coder/src/application/mod.rs`
- Modify: `crates/unica-coder/src/application/tool_contracts.rs`

**Interfaces:**
- Consumes: raw MCP `Map<String, Value>` only at `parse_discover_request`.
- Produces: `DiscoverRequest`, `DiscoveryMode`, dedicated limit newtypes, `DiscoveryContractError`, `discover_input_schema()`, `discover_allowed_args()`, and `parse_discover_request(&Map<String, Value>) -> Result<DiscoverRequest, DiscoveryContractError>`.

- [ ] **Step 1: Write failing request tests**

In `application/discovery/contract.rs`, add tests that assert:

```rust
#[test]
fn task_only_request_derives_a_typed_explore_request() {
    let request = parse(json!({
        "cwd": "/workspace",
        "mode": "explore",
        "task": "При поступлении контролировать срок годности серий"
    })).expect("task-only request");
    assert_eq!(request.mode(), DiscoveryMode::Explore);
    assert_eq!(request.task(), "При поступлении контролировать срок годности серий");
    assert_eq!(request.limits().max_files().get(), 20_000);
    assert!(request.concepts().is_empty());
}

#[test]
fn cyrillic_task_limit_is_measured_in_utf8_bytes() {
    assert!(parse(request_with_task("я".repeat(4_096))).is_ok());
    let error = parse(request_with_task("я".repeat(4_097))).unwrap_err();
    assert_eq!(error.code(), DiscoveryContractErrorCode::TextBytesOutOfRange);
}
```

Add table tests rejecting missing/wrong mode, empty task, normalized duplicates, malformed canonical objects, absolute or `..` source paths, forbidden Slice C/D/common fields, unknown fields, zero limits, and every maximum plus one.

- [ ] **Step 2: Verify RED**

Run `cargo test -p unica-coder --lib application::discovery::contract::tests`.

Expected: compile failure because the typed contract module does not exist.

- [ ] **Step 3: Implement the typed contract**

Use these exact public-in-crate shapes:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DiscoveryMode { Explore }

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DiscoverRequest {
    cwd: Option<PathBuf>,
    mode: DiscoveryMode,
    task: String,
    source_dir: Option<PathBuf>,
    concepts: Vec<String>,
    search_terms: Vec<String>,
    objects: Vec<ArtifactId>,
    limits: DiscoveryLimits,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MaxFiles(u32);
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MaxBytes(u64);
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MaxEvidence(u16);
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MaxCandidates(u16);
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MaxGraphDepth(u8);

pub(crate) fn parse_discover_request(
    args: &Map<String, Value>,
) -> Result<DiscoverRequest, DiscoveryContractError>;
pub(crate) fn discover_input_schema() -> Value;
pub(crate) fn discover_allowed_args() -> &'static [&'static str];
```

Start `domain/discovery.rs` with the dedicated `ArtifactId(String)` newtype and its borrowed `as_str` accessor. Trim then normalize case for uniqueness while preserving a deterministic normalized value. `ArtifactId::parse` rejects separators `/`, `\\`, empty dot segments, leading/trailing dots, and identifiers without at least a kind and name. `sourceDir` is relative, non-empty, and contains only normal/current path components.

- [ ] **Step 4: Add schema/runtime parity tests**

Test that the schema has exactly the seven optional/required properties, `required == ["mode", "task"]`, `additionalProperties == false`, `mode.enum == ["explore"]`, nested `limits.additionalProperties == false`, exact numeric maxima, array maxima, and byte-limit descriptions without `maxLength`. For every representative accepted/rejected payload, assert the schema's local structural evaluator and `parse_discover_request` agree.

- [ ] **Step 5: Wire only the schema seam and verify GREEN**

Special-case `tool.name == "unica.project.discover"` before `COMMON_ARGS` in `input_schema_for_tool`, `allowed_args`, and `required_args`. Do not register `ToolHandler::ProjectDiscover` yet. The generic validator may enforce the advertised outer shape, but only `parse_discover_request` constructs the typed request.

Run:

```bash
cargo test -p unica-coder --lib application::discovery::contract::tests
cargo test -p unica-coder --lib application::tool_contracts::tests
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/unica-coder/src/application/discovery crates/unica-coder/src/application/mod.rs crates/unica-coder/src/application/tool_contracts.rs crates/unica-coder/src/domain/discovery.rs crates/unica-coder/src/domain/mod.rs
git commit -m "feat: define typed project discovery request"
```

### Task 3: Build the evidence model, ports, and pure use case

**Files:**
- Modify: `crates/unica-coder/src/domain/discovery.rs`
- Modify: `crates/unica-coder/src/domain/mod.rs`
- Create: `crates/unica-coder/src/application/discovery/ports.rs`
- Create: `crates/unica-coder/src/application/discovery/use_case.rs`
- Modify: `crates/unica-coder/src/application/discovery/mod.rs`

**Interfaces:**
- Consumes: `DiscoverRequest`, `DiscoveryEnvironment`, and typed provider traits.
- Produces: `DiscoverExtensionPointsUseCase::execute(&DiscoverRequest, &DiscoveryEnvironment) -> Result<DiscoveryReport, DiscoveryError>`, `ProviderOutcome<T>`, provider fact batches, stable IDs, and the serializable report used by Task 7.

- [ ] **Step 1: Write failing provider-status and graph tests**

Use fake ports to cover each exhaustive outcome. The core assertions are:

```rust
#[test]
fn unavailable_flow_provider_keeps_metadata_and_makes_report_partial() {
    let ports = FakePorts::with_metadata(series_metadata())
        .with_runtime_flow(ProviderOutcome::Unavailable(diagnostic("index_missing")));
    let report = execute(&ports, task_only()).expect("partial report");
    assert_eq!(report.status, DiscoveryStatus::Partial);
    assert!(report.candidates.iter().any(|item| item.target == series_id()));
    assert!(report.missing_checks.iter().any(|item| item.provider == ProviderKind::RuntimeFlow));
}

#[test]
fn lexical_fact_never_creates_a_runtime_flow_edge_or_candidate() {
    let report = execute(&FakePorts::only_lexical(lexical_hit()), task_only()).unwrap();
    assert!(report.runtime_flow_edges.is_empty());
    assert!(report.candidates.is_empty());
    assert_eq!(report.related_artifacts.len(), 1);
}

#[test]
fn contract_violation_excludes_all_records_from_that_provider() {
    let report = execute(&FakePorts::contract_violating_forms(), task_only()).unwrap();
    assert!(report.evidence.iter().all(|item| item.provider != ProviderKind::ManagedForms));
    assert!(report.warnings.iter().any(|item| item.blocking));
}
```

- [ ] **Step 2: Verify RED**

Run `cargo test -p unica-coder --lib application::discovery::use_case::tests`.

Expected: compile failure for missing discovery model/ports/use case.

- [ ] **Step 3: Define the typed domain model**

Use `#[serde(rename_all = "camelCase")]` on structs and snake-case serialization on enums. Define dedicated `ArtifactId`, `EvidenceId`, `ContentHash`, and mapping/snapshot fingerprint newtypes. Define exhaustive enums for `ArtifactKind`, `ConceptProvenance`, `ProviderKind`, `ProviderOutcomeKind`, `DiscoveryStatus`, `StructuralRelationKind`, `RuntimeFlowRelationKind`, `EvidenceKind`, `SupportStateKind`, and `MissingCheckMateriality`.

The report shape is:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DiscoveryReport {
    pub schema_version: u32,
    pub status: DiscoveryStatus,
    pub source: DiscoverySource,
    pub analysis_snapshot: AnalysisSnapshot,
    pub concepts: Vec<DiscoveryConcept>,
    pub provider_outcomes: Vec<ProviderReport>,
    pub related_artifacts: Vec<RelatedArtifact>,
    pub structural_edges: Vec<StructuralEdge>,
    pub runtime_flow_edges: Vec<RuntimeFlowEdge>,
    pub candidates: Vec<ExtensionPointCandidate>,
    pub warnings: Vec<DiscoveryWarning>,
    pub missing_checks: Vec<MissingCheck>,
    pub evidence: Vec<Evidence>,
}
```

No field contains the original task, a receipt, a score, or confidence.

- [ ] **Step 4: Define provider ports**

```rust
pub(crate) enum ProviderOutcome<T> {
    Complete(T),
    Bounded { data: T, diagnostic: ProviderDiagnostic },
    Unavailable(ProviderDiagnostic),
    Failed(ProviderDiagnostic),
    ContractViolation(ProviderDiagnostic),
}

pub(crate) trait SourceInventoryPort { fn inventory(&self, query: &DiscoveryQuery<'_>) -> ProviderOutcome<SourceInventory>; }
pub(crate) trait MetadataCatalogPort { fn metadata(&self, query: &DiscoveryQuery<'_>, files: &SourceInventory) -> ProviderOutcome<FactBatch<MetadataFact>>; }
pub(crate) trait ManagedFormPort { fn forms(&self, query: &DiscoveryQuery<'_>, files: &SourceInventory) -> ProviderOutcome<FactBatch<FormFact>>; }
pub(crate) trait BslSearchPort { fn search(&self, query: &DiscoveryQuery<'_>, files: &SourceInventory) -> ProviderOutcome<FactBatch<BslFact>>; }
pub(crate) trait DefinitionPort { fn definitions(&self, query: &DiscoveryQuery<'_>) -> ProviderOutcome<FactBatch<DefinitionFact>>; }
pub(crate) trait RuntimeFlowPort { fn runtime_flow(&self, query: &DiscoveryQuery<'_>) -> ProviderOutcome<FactBatch<RuntimeFlowFact>>; }
pub(crate) trait SupportStatePort { fn support(&self, query: &DiscoveryQuery<'_>, files: &SourceInventory) -> ProviderOutcome<FactBatch<SupportFact>>; }
```

`DiscoveryPorts<'a>` holds references to these seven ports. `FactBatch<T>` contains records plus exact `AnalyzedFile` contributors and `ProviderCoverage`.

- [ ] **Step 5: Implement concept derivation, promotion, status, and stable hashes**

Derive Unicode alphanumeric tokens of at least three characters, split identifier/camel-case segments, retain full task for search ports, and use language-neutral normalized prefixes only for matching. Keep only full normalized concepts in the report, tagged `taskDerived`; merge explicit concepts tagged `explicit` without erasing either provenance.

Generate stable evidence IDs from provider kind, evidence kind, target, relation, location, and raw content hash. Generate the analysis fingerprint from mapping identity plus sorted evidence-contributing `(path, rawHash, bytes)` entries. Promote only typed structural/runtime facts; relevant metadata/form/callback facts may create candidates, while a standalone lexical fact remains related evidence.

- [ ] **Step 6: Verify GREEN and determinism**

Run:

```bash
cargo test -p unica-coder --lib application::discovery::use_case::tests
cargo test -p unica-coder --lib domain::discovery::tests
```

Expected: PASS, including a test that serializes the same report twice and compares both bytes and snapshot fingerprints.

- [ ] **Step 7: Commit**

```bash
git add crates/unica-coder/src/domain/discovery.rs crates/unica-coder/src/domain/mod.rs crates/unica-coder/src/application/discovery
git commit -m "feat: add typed discovery evidence core"
```

### Task 4: Add verified contained inventory and raw-byte snapshots

**Files:**
- Create: `crates/unica-coder/src/infrastructure/platform/contained_file.rs`
- Modify: `crates/unica-coder/src/infrastructure/platform/mod.rs`
- Create: `crates/unica-coder/src/infrastructure/discovery/inventory.rs`
- Create: `crates/unica-coder/src/infrastructure/discovery/mod.rs`
- Modify: `crates/unica-coder/src/infrastructure/mod.rs`
- Modify: `scripts/ci/check-rust-platform-boundary.py` only if its allowlist requires the new platform module.

**Interfaces:**
- Consumes: a canonical selected source root and `maxFiles`/`maxBytes` newtypes.
- Produces: `read_contained_regular_file(root: &Path, path: &Path, max_bytes: u64) -> Result<VerifiedFile, ContainedFileError>` and `ContainedSourceInventoryPort`.

- [ ] **Step 1: Write failing platform and inventory tests**

Cover: regular contained read; outside path; symlink/reparse file and directory; FIFO/non-regular target where supported; per-file/aggregate bounds; deterministic traversal; path replacement via a test-only observer between pre-open and post-open checks; BOM and LF/CRLF producing different hashes; actual sidecar exclusion and legitimate external `ConfigDumpInfo.xml` preservation.

The raw-hash assertion is:

```rust
let bom_lf = read_fixture(&[0xef, 0xbb, 0xbf, b'a', b'\n']);
let plain_crlf = read_fixture(b"a\r\n");
assert_ne!(bom_lf.raw_sha256, plain_crlf.raw_sha256);
assert_eq!(bom_lf.bytes_read, 5);
```

- [ ] **Step 2: Verify RED**

Run:

```bash
cargo test -p unica-coder --lib infrastructure::platform::contained_file::tests
cargo test -p unica-coder --lib infrastructure::discovery::inventory::tests
```

Expected: compile failure for missing verified-read/inventory APIs.

- [ ] **Step 3: Implement the platform facade**

`VerifiedFile` contains `relative_path`, exact `bytes`, `raw_sha256`, `bytes_read`, and a neutral verified identity. Unix opens with `O_NOFOLLOW` and compares `(dev, ino)` before/open/after; Windows opens the reparse point and compares `(volumeSerial, fileIndex)` plus reparse attributes; unsupported hosts fail closed. Resolve the final opened-handle path and require it to remain beneath the canonical root. Platform cfg blocks must not escape `infrastructure/platform`.

- [ ] **Step 4: Implement deterministic inventory**

Walk directories in sorted order, reject link/reparse/non-directory entries, count every inspected evidence-eligible file once, and stop with `ProviderOutcome::Bounded` while preserving already verified records when a caller limit is reached. Read XML, BSL, and `Ext/ParentConfigurations.bin`; classify `ConfigDumpInfo.xml` from bytes. A path/identity/security violation returns `ContractViolation` and no inventory records.

- [ ] **Step 5: Verify GREEN and platform boundary**

Run the two focused Rust commands from Step 2 plus `python3.12 scripts/ci/check-rust-platform-boundary.py`.

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/unica-coder/src/infrastructure/platform crates/unica-coder/src/infrastructure/discovery crates/unica-coder/src/infrastructure/mod.rs scripts/ci/check-rust-platform-boundary.py
git commit -m "feat: capture bounded discovery source inventory"
```

### Task 5: Implement Platform XML, managed-form, and support providers

**Files:**
- Create: `crates/unica-coder/src/infrastructure/discovery/metadata.rs`
- Create: `crates/unica-coder/src/infrastructure/discovery/forms.rs`
- Create: `crates/unica-coder/src/infrastructure/discovery/support.rs`
- Modify: `crates/unica-coder/src/infrastructure/discovery/mod.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/common.rs`

**Interfaces:**
- Consumes: verified `SourceInventory` bytes and typed `DiscoveryQuery`.
- Produces: `PlatformXmlMetadataProvider`, `ManagedFormProvider`, `SupportStateProvider`, and typed facts with exact paths/line or XML-fragment locations.

- [ ] **Step 1: Write failing provider tests from real XML bytes**

Metadata tests parse a document with both `Товары.Серия` and separate `Серии`, a data processor, and its form. Form tests cover DataPath, command/action, and event-handler bindings without flattening them into display text. Support tests cover not-on-support, locked, editable, removed, malformed, and bounded input.

Assert exact typed facts, for example:

```rust
assert!(metadata.records.iter().any(|fact| {
    fact.artifact == ArtifactId::parse(
        "Document.ПриобретениеТоваровУслуг.TabularSection.Серии"
    ).unwrap() && fact.relation == MetadataRelation::Contains
}));
assert!(forms.records.iter().any(|fact| matches!(
    fact.binding,
    FormBinding::Event { ref handler, .. } if handler == "ПроверитьСрокГодности"
)));
assert_eq!(support.for_object(&document_id), Some(SupportStateKind::Locked));
```

- [ ] **Step 2: Verify RED**

Run:

```bash
cargo test -p unica-coder --lib infrastructure::discovery::metadata::tests
cargo test -p unica-coder --lib infrastructure::discovery::forms::tests
cargo test -p unica-coder --lib infrastructure::discovery::support::tests
```

Expected: compile failure for missing provider modules.

- [ ] **Step 3: Implement metadata and form parsers**

Parse `MetaDataObject` descriptors from supplied bytes with `roxmltree`. Build canonical IDs from actual root kind/name and recursively typed child objects. Enumerate forms only from declared metadata relationships and canonical `Forms/<name>/Ext/Form.xml`; do not guess arbitrary paths or reuse presentation analyzers. Preserve structural DataPath/contains relationships separately from event/action runtime-flow facts.

- [ ] **Step 4: Refactor the narrow support seam**

Promote object rules from raw `u8` to:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SupportObjectRule { Locked, Editable, OffSupport }
```

Expose a pure `parse_support_state_bytes(bytes: &[u8]) -> Result<ParsedSupportState, SupportParseError>`. Existing info/guard callers keep their behavior by formatting or matching this typed model. Discovery never calls `support_status_for_path` and never reparses display strings.

- [ ] **Step 5: Add the structural warning rule and verify GREEN**

The use case may emit `alternative_relevant_tabular_section` only from actual project facts: a task-relevant nested attribute and a distinct task-relevant tabular section share the same metadata object. The generic warning states that an attribute-only point lacks coverage and cites both evidence IDs; it is not a task dictionary and does not claim a proposed point was rejected.

Run the three focused provider commands from Step 2 plus `cargo test -p unica-coder --lib infrastructure::native_operations::common::tests`. Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/unica-coder/src/infrastructure/discovery crates/unica-coder/src/infrastructure/native_operations/common.rs crates/unica-coder/src/application/discovery/use_case.rs
git commit -m "feat: collect typed metadata form and support evidence"
```

### Task 6: Implement typed BSL lexical/index boundaries and explicit flow gaps

**Files:**
- Create: `crates/unica-coder/src/infrastructure/discovery/bsl.rs`
- Modify: `crates/unica-coder/src/infrastructure/discovery/mod.rs`
- Modify: `crates/unica-coder/src/infrastructure/workspace_index.rs`
- Modify: `crates/unica-coder/src/infrastructure/internal_adapters.rs`

**Interfaces:**
- Consumes: verified BSL bytes, full task/search terms, selected source identity, and an existing RLM status/database when safely available.
- Produces: typed lexical and definition facts; a runtime-flow provider that returns typed graph facts only after validated graph data, otherwise an explicit non-complete outcome.

- [ ] **Step 1: Write failing lexical, definition, and flow-boundary tests**

Cover BSL procedure/function definition extraction with exact lines, Cyrillic task matching, no-match `Complete`, truncated `Bounded`, missing/stale/out-of-root index `Unavailable`/`ContractViolation`, typed SQLite rows, and malformed graph data `ContractViolation`. Assert that lexical and definitions stay structural and that only `RuntimeFlowFact` becomes a runtime edge.

- [ ] **Step 2: Verify RED**

Run:

```bash
cargo test -p unica-coder --lib infrastructure::discovery::bsl::tests
cargo test -p unica-coder --lib infrastructure::workspace_index::tests
```

Expected: missing typed BSL query/provider APIs.

- [ ] **Step 3: Extract typed RLM rows**

Add:

```rust
pub(crate) struct IndexedMethodHit {
    pub name: String,
    pub method_kind: IndexedMethodKind,
    pub exported: bool,
    pub line: u32,
    pub end_line: u32,
    pub module_path: PathBuf,
    pub object_name: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IndexedMethodKind { Procedure, Function }

pub(crate) fn search_indexed_methods(
    db_path: &Path,
    query: &str,
    limit: u16,
) -> Result<Vec<IndexedMethodHit>, IndexQueryError>;
pub(crate) fn find_indexed_definitions(
    db_path: &Path,
    name: &str,
    limit: u16,
) -> Result<Vec<IndexedMethodHit>, IndexQueryError>;
```

Refactor existing code-search/definition formatting to consume these typed rows, preserving their public text behavior. Discovery consumes rows directly and validates every module path against the selected root/snapshot.

- [ ] **Step 4: Implement lexical and runtime-flow providers**

Lexical scanning uses verified inventory bytes, records exact method/line locations, and returns `Bounded` whenever inventory or evidence limits truncate coverage. The runtime-flow port does not start an index or workspace service and does not parse another adapter's display text. If no validated existing typed graph source is available, return `Unavailable("runtime_flow_unavailable")`; fake-port application tests still exercise promotion of valid typed graph facts. This explicit gap is material and keeps the report partial.

- [ ] **Step 5: Verify GREEN and compatibility**

Run the focused commands from Step 2 plus `cargo test -p unica-coder --lib infrastructure::internal_adapters::tests`. Expected: PASS with unchanged existing public adapter output snapshots.

- [ ] **Step 6: Commit**

```bash
git add crates/unica-coder/src/infrastructure/discovery/bsl.rs crates/unica-coder/src/infrastructure/discovery/mod.rs crates/unica-coder/src/infrastructure/workspace_index.rs crates/unica-coder/src/infrastructure/internal_adapters.rs
git commit -m "refactor: expose typed BSL discovery evidence"
```

### Task 7: Register the public tool and pass task-only UT acceptance

**Files:**
- Modify: `crates/unica-coder/src/application/mod.rs`
- Modify: `crates/unica-coder/src/application/ports.rs`
- Modify: `crates/unica-coder/src/infrastructure/application_ports.rs`
- Modify: `crates/unica-coder/src/infrastructure/source_roots.rs`
- Modify: `crates/unica-coder/src/interfaces/mcp.rs`
- Create: `tests/fixtures/extension-point-discovery/ut115/v8project.yaml`
- Create: `tests/fixtures/extension-point-discovery/ut115/decoy/ExactExpectedNames.txt`
- Create: `tests/fixtures/extension-point-discovery/ut115/src/**`
- Modify: `tests/ci/test_unica_mcp_smoke.py`

**Interfaces:**
- Consumes: the Task 2 request parser, Task 3 use case/report, and Task 4-6 providers.
- Produces: exactly one public `ToolHandler::ProjectDiscover`, typed `OperationData`, typed `ApplicationPorts::discover_extension_points`, and real MCP task-only results at `data.discovery`.

- [ ] **Step 1: Write failing registry/dispatch/MCP tests**

Add tests that require exactly one tool, strict forbidden-field rejection before any fake provider call, non-mutating/no-write cache metadata, and typed serialization:

```rust
assert_eq!(tools().iter().filter(|tool| tool.name == "unica.project.discover").count(), 1);
let tool = tools().into_iter().find(|tool| tool.name == "unica.project.discover").unwrap();
assert!(!tool.mutating);
assert!(tool.cache_access.writes.is_empty());

let payload = serde_json::to_value(app.call_tool("unica.project.discover", &args).unwrap()).unwrap();
assert_eq!(payload["data"]["discovery"]["schemaVersion"], 1);
assert!(payload.get("stdout").is_none());
```

- [ ] **Step 2: Verify RED**

Run:

```bash
cargo test -p unica-coder --lib application::tests
cargo test -p unica-coder --lib interfaces::mcp::tests
```

Expected: missing registry/typed dispatch/data failures.

- [ ] **Step 3: Add typed application dispatch**

Add:

```rust
#[derive(Debug, Serialize)]
pub struct OperationData { discovery: DiscoveryReport }

#[derive(Debug, Clone, Copy)]
pub enum ToolHandler {
    NativeOperation { operation: &'static str, event: Option<DomainEventKind> },
    ProjectStatus,
    ProjectMap,
    ProjectDiscover,
    BuildRuntime { command: &'static [&'static str], event: Option<DomainEventKind> },
    RuntimeAdapter,
    RuntimeJob { action: RuntimeJobAction },
    CodeAdapter { command: &'static [&'static str] },
    StandardsAdapter { operation: &'static str },
}

fn discover_extension_points(
    &self,
    request: &DiscoverRequest,
    context: &WorkspaceContext,
    cancellation: &CancellationToken,
) -> Result<DiscoveryReport, DiscoveryError>;
```

Insert that method into the existing `ApplicationPorts` trait and implement it in every production/test port implementation without changing the signatures of the trait's current methods.

In `call_tool`, match `ProjectDiscover` before generic `validate_tool_arguments`/`invoke_handler`: call `parse_discover_request` once, discover the workspace from its typed `cwd`, validate context/source selection, invoke the typed port, read the cache report, and return `OperationResult { data: Some(OperationData { discovery }), .. }`. Every existing result literal uses `data: None`. The generic infrastructure handler's `ProjectDiscover` arm returns an internal dispatch error so raw invocation cannot bypass the typed path.

- [ ] **Step 4: Add discovery-specific source selection**

When `sourceDir` is absent, select exactly one `SourceSetKind::Configuration`; zero or multiple eligible roots returns a typed source error listing candidates. Do not change `select_default_source_set`, because existing `project.map`/runtime contracts intentionally keep their current rule. Resolve the selected root canonically and contained before inventory.

- [ ] **Step 5: Add the UT 11.5 fixture and source MCP acceptance**

The fixture contains a Platform XML document with `Товары.Серия` and separate `Серии`, `DataProcessor.ПодборСерийВДокументы`, the declared `DataProcessor.ПодборСерийВДокументы.Form.РегистрацияИПодборСерийПоОднойСтрокеТоваров` managed form with real event and command bindings, a BSL manager module, support state, and a decoy outside `src`.

Call exactly:

```json
{
  "cwd": "tests/fixtures/extension-point-discovery/ut115",
  "mode": "explore",
  "task": "При поступлении товаров контролировать остаточный срок годности серий"
}
```

Assert `partial`, metadata/form evidence, explicit missing BSL runtime-flow check, all three required candidates with advisory typed recommendations, the insufficient `Товары.Серия` warning, stable evidence locations, raw snapshot hashes, and absence of the decoy.

- [ ] **Step 6: Verify GREEN**

Run:

```bash
cargo test -p unica-coder --lib application::tests
cargo test -p unica-coder --lib interfaces::mcp::tests
python3.12 -m unittest tests.ci.test_unica_mcp_smoke -v
```

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/unica-coder/src/application crates/unica-coder/src/infrastructure/application_ports.rs crates/unica-coder/src/infrastructure/source_roots.rs crates/unica-coder/src/interfaces/mcp.rs tests/fixtures/extension-point-discovery tests/ci/test_unica_mcp_smoke.py
git commit -m "feat: expose task-only extension point discovery"
```

### Task 8: Add the mandatory skill and packaged acceptance

**Files:**
- Create: `plugins/unica/skills/extension-point-discovery/SKILL.md`
- Create: `plugins/unica/skills/extension-point-discovery/agents/openai.yaml`
- Modify: `plugins/unica/provenance/skill-upstreams.json`
- Modify: `plugins/unica/README.md`
- Modify: `plugins/unica/references/use-cases/metadata-modeling.md`
- Modify: `spec/acceptance/unica-mcp-validation.md`
- Modify: `tests/ci/test_unica_skills.py`
- Modify: `tests/ci/test_skill_provenance.py` only if a focused Unica-owned assertion is needed.
- Modify: `tests/ci/test_package_unica_plugin.py`
- Modify: `scripts/ci/smoke-unica-mcp.py`
- Modify: `tests/ci/test_smoke_unica_mcp.py`
- Modify: `scripts/ci/release-assessment.py`
- Modify: `tests/ci/test_release_assessment.py`

**Interfaces:**
- Consumes: public `unica.project.discover` and the Task 7 fixture contract.
- Produces: mandatory implicit skill routing, complete provenance, packaged files, release allowlists, and a task-only call against the shipped native runtime.

- [ ] **Step 1: Write failing skill/routing/package tests**

Add `extension-point-discovery` to `SCENARIO_SKILLS`, not `IN_SCOPE_TOOLS`. Dedicated tests require trigger terms for planning/implementation of typical/supported configurations, CFE, forms, documents, processors, handlers, and tabular sections; a first JSON example with exactly `mode`, `task`, and optional `cwd`; result-field inspection; public fallback allowlist; material-gap stop; selection record; and no forbidden Slice C/D/internal/script/local-shell tokens.

Add packaging assertions that both new skill files are in the generated plugin and that no `scripts` directory exists under the skill.

- [ ] **Step 2: Verify RED**

Run:

```bash
python3.12 -m unittest tests.ci.test_unica_skills -v
python3.12 -m unittest tests.ci.test_skill_provenance -v
python3.12 -m unittest tests.ci.test_package_unica_plugin -v
```

Expected: missing skill/provenance/package failures.

- [ ] **Step 3: Write the skill and agent metadata**

The frontmatter has only `name` and the comprehensive trigger `description`. The body starts with `## MCP routing`, requires the task-only public call before planning and before mutation/manual edits, explains `data.discovery`, restricts gap closure to public tools, stops on unresolved material gaps, and gives the exact selection record. It explicitly says discovery is not mutation permission.

`agents/openai.yaml` contains:

```yaml
interface:
  display_name: "Discovery точек расширения 1С"
  short_description: "Обязательный preflight типовых доработок 1С"
  default_prompt: "Используй $extension-point-discovery до планирования изменений, найди и обоснуй типовую точку расширения 1С."
policy:
  allow_implicit_invocation: true
```

- [ ] **Step 4: Add Unica-owned provenance and docs**

Add an entry with `skill: "extension-point-discovery"`, `primarySource: "unica"`, local paths, and contract paths. Explain that PR #83 was historical research, not an upstream code baseline. Update packaged README/use-case/acceptance prose without changing `.mcp.json`, plugin manifest, runtime manifest, or tool locks.

- [ ] **Step 5: Add shipped-runtime task-only smoke**

Add `unica.project.discover` to `REQUIRED_TOOLS` and release-assessment expected tools. Extend `smoke-unica-mcp.py` so its temporary workspace contains the minimal task-only fixture and its MCP input includes a third `tools/call`; validate `data.discovery`, partial status, required document/processor/form targets, warning, and missing BSL check. Update fake-server tests to reject malformed or missing discovery data. This script already runs against the extracted packaged native binary in release CI, so the gate tests shipped code rather than only `cargo run`.

- [ ] **Step 6: Verify GREEN and official skill shape**

Run:

```bash
python3.12 -m unittest tests.ci.test_unica_skills tests.ci.test_skill_provenance tests.ci.test_package_unica_plugin tests.ci.test_smoke_unica_mcp tests.ci.test_release_assessment -v
python3.12 /Users/korolev/.codex/skills/.system/skill-creator/scripts/quick_validate.py plugins/unica/skills/extension-point-discovery
```

Expected: PASS and validator exit 0.

- [ ] **Step 7: Commit**

```bash
git add plugins/unica spec/acceptance/unica-mcp-validation.md tests/ci scripts/ci
git commit -m "feat: require packaged extension point preflight"
```

### Task 9: Whole-branch verification and PR readiness

**Files:**
- Modify only files required to fix findings from the mandated reviews.

**Interfaces:**
- Consumes: Tasks 1-8 and the approved design.
- Produces: a clean, review-approved branch ready to push and open as one PR.

- [ ] **Step 1: Run the Rust gates**

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace -- --test-threads=1
```

Expected: all exit 0 with no warnings.

- [ ] **Step 2: Run the Python/package/platform gates**

```bash
python3.12 -m unittest discover -s tests/ci --durations 20
python3.12 scripts/ci/check-rust-platform-boundary.py
python3.12 scripts/ci/check-skill-upstreams.py
python3.12 /Users/korolev/.codex/skills/.system/skill-creator/scripts/quick_validate.py plugins/unica/skills/extension-point-discovery
git diff --check
```

Expected: all exit 0.

- [ ] **Step 3: Run the mandated reviews and fix every blocking finding**

Use `superpowers:requesting-code-review` for a whole-branch review and the `rust-expert-best-practices-code-review` rules for a focused Rust review. Re-run the covering focused tests after every correction and repeat review until both report no Critical/Important finding.

- [ ] **Step 4: Verify issue/PR language**

The PR body must contain `Closes #5` and `Refs #161`, state that proposal validation and receipts/guards remain out of scope, list the packaged task-only acceptance, and avoid claiming discovery authorizes mutation or proves runtime flow when the provider outcome is unavailable.

- [ ] **Step 5: Commit review corrections**

```bash
git add crates/unica-coder plugins/unica scripts/ci spec tests/ci tests/fixtures/extension-point-discovery
git commit -m "fix: address extension discovery review"
```

Skip this commit only when review produces no file changes.
