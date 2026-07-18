# Task 7 Project Discovery Mechanisms Implementation Design

> **Task 5B v5 back-propagation applied 2026-07-18.** The former rule “one
> MetadataCatalog invocation per captured source” and its RED are superseded.
> Task 7 now performs one composite Metadata invocation over the captured
> analysis plus exact destination pair groups. `SourceSetWide` remains local to
> an internal source group; FormInspection remains analysis-only. The rejected
> v4 EventSubscription closure is also superseded: Task 7 consumes the complete
> v5 descriptor and never reconstructs source compatibility from edge fragments.
> FormCommand and HTTP route facts are likewise pending: Task 7 consumes the
> v5 `form-command-handlers/v1` / `http-service-handlers/v1` join results and
> never promotes a handler merely because a same-named Definition exists.
> The immutable pre-v5 Task 7 snapshot is
> `dfe521ab491b4696b89728b5ed0089da57eec3320c2af7685c0dced7aef02736`;
> implement only from this v5-back-propagated file and its published final hash.

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> `superpowers:subagent-driven-development` (recommended) or
> `superpowers:executing-plans` to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** Реализовать детерминированную application-owned orchestration восьми
v1 mechanism families поверх Task 5/6 typed providers: сначала зафиксировать
48-case corpus, затем построить source-scoped staged queries, bounded BFS,
material checks, actionable candidates и exact proposal verdicts без публичной
регистрации MCP tool.

**Architecture:** Task 7 не добавляет новый parser и не позволяет adapter-у
выбирать архитектуру. `DiscoverExtensionPointsUseCase` вызывает точные provider
queries по стадиям, хранит каждую invocation вместе с query scope, строит
versioned mechanism instances и распространяет runtime reachability только от
typed entry seeds по направленным call edges. Application-owned traversal,
support-planning and candidate-selection bounds представляются отдельным
`DiscoveryTraversal` outcome; provider record ceilings остаются в outcome того
порта, чьи records были отброшены, с точным source-qualified scope.

**Tech Stack:** Rust, существующие discovery model/ports/determinism,
Task 5 Platform XML/support providers, Task 6 snapshot BSL providers,
`serde`/`serde_json` для test-only corpus, Python stdlib для schema/reference
gate. Новые runtime dependencies и публичные MCP contracts не нужны.

## Global Constraints

- Источник истины: code/tests/package metadata > active spec > historical plan.
- Task 7 начинается только после GREEN Task 5A/5B/5C и Task 6A/6B.
- До первого mechanisms wiring strict 48-case corpus seed и fixture references
  должны быть schema-valid.
- Analysis source в v1 только Platform XML. EDT вызывает zero evidence ports и
  возвращает существующий diagnostic-only insufficient report.
- Provider evidence использует только Task 4 immutable snapshots; Task 7 не
  читает файлы, SQLite, RLM или display text.
- Query-specific Definition/Call outcomes не теряют exact invocation scope.
- `ProviderGapScope::Artifacts` всегда содержит `SourceScopedArtifact`; один
  canonical ref в analysis/destination не схлопывается.
- Application root/frontier/node/mechanism/support/candidate bounds не создают
  `ProviderGap`. Единственное исключение — уже принятые per-port/global
  `maxEvidence` evidence-group ceilings: они считают physical records, но
  отбрасывают только целые `SemanticAtomicGroupIdV1` clusters, меняют outcome
  именно того evidence port, чей cluster реально отброшен, и получают source-
  qualified whole-cluster gap.
- `maxGraphDepth` считает только directed BSL `calls` hops. Typed base binding
  chain механизма не потребляет call depth.
- `maxCandidates` — deterministic public-candidate resource bound, не
  relevance score, top-k и не BFS bound. До Support он ограничивает только
  exploratory candidate prefix, для которого ещё нужна support
  classification; explicit proposal targets всегда добавляются вне этого
  budget и валидируются полностью.
- Structural `contains`/`defines` edges никогда не дают runtime reachability.
- Ни один standalone call edge не создаёт runtime root.
- Публичный `unica.project.discover`, tool schema, package metadata, skills и
  MCP registration остаются нетронутыми.
- Task 7 не выдаёт persistent receipt. Corpus использует deterministic probe
  issuer; native composition сохраняет `receipt_store_not_implemented`.

---

## 1. Mandatory gates and corrected assumptions

### 1.1 Active spec must stop contradicting Task 5

Перед Task 7 active binding matrix должна содержать только:

```text
SubscriptionSource + uses + MetadataCatalogPort
ExchangePlan --uses--> EventSubscription --subscribes--> CommonModule.method
```

Direct exchange callbacks/BSP orchestration остаются bounded
`unsupported_mechanism_variant`. Если старый row остаётся в spec или model,
Task 7 останавливается.

The same pre-Task 7 spec gate must state that declarative handler references are
pending until compatible Definition joins, name both FormCommand/HTTP policy
versions, retain `DefinitionShape.is_async`, and separate ScheduledJob Disabled
activation from its enabled binding descriptor. A spec that promotes a raw
Form/HTTP declarative `handles` fact to runtime connectivity or makes Use=false
depend on Definition is not accepted
merely because the ExchangePlan row was corrected.

### 1.2 Concrete providers are preconditions, not Task 7 substitutes

Task 7 потребляет, но не реализует заново:

- shared `PlatformConfigurationCatalogV1`;
- `PlatformXmlMetadataCatalogProvider` implementing `MetadataCatalogPort`;
- `PlatformXmlFormInspectionProvider` implementing `FormInspectionPort`;
- accepted `EventSubscriptionDescriptorV1`/binding facts from Task 5B v5 with
  exact `event-subscriptions/v1`, all 13 family-to-root mappings, all 21
  compatible event/family rows, three signature classes, canonical-unique exact
  selected sources, the five-field CommonModule profile, and exact Definition
  compatibility;
- accepted pending FormCommand/HTTP binding requirements plus the exact
  `form-command-handlers/v1` and `http-service-handlers/v1` tri-state
  compatibility results; no guessed handler defaults;
- accepted `ScheduledJobActivationV1` whose Disabled state is exact
  metadata-only No independent of binding/Definition material, plus the
  separate enabled binding requirement;
- `SnapshotSupportStateProvider` implementing staged `SupportStatePort`;
- `SnapshotBslEvidenceProvider` implementing query-specific
  `CodeSearchPort`, `DefinitionPort`, `CallGraphPort`;
- Task 6 `CallTarget`, the Task-5A-accepted `DefinitionShape.is_async`, exact RU/EN parser and
  service-v2 cache boundary.

If the landed Task 5/6 struct names differ, update this file map to their exact
accepted names before implementation; do not create wrapper duplicates merely
to satisfy this spelling.

Task 7 treats that accepted EventSubscription descriptor as an authoritative
validated whole fact. It does not parse Type strings, rerun the family/event
matrix, assign a fallback signature class, or assemble a descriptor from
`SubscriptionSource` edges. If the descriptor/Definition is missing, gapped,
registry-version-wrong, or incomplete, no family-2/family-7 instance exists.
Any per-source companion set must exactly equal the descriptor selected set, and
the public ExchangePlan `uses` set must exactly equal only its
ExchangePlanObject-filtered subset. A partial or independently reconstructed set
cannot seed reachability.

The same authority rule applies to every pending handler requirement. Task 7
does not implement a fallback compatibility matcher. In particular, FormCommand
requires an exact one-parameter `&AtClient Procedure` (sync or async) and HTTP
requires the exact synchronous unannotated one-parameter Function row, with the
parameter transfer/default and Export semantics frozen in Task 5B v5. Event and
ScheduledJob accept only their synchronous primary-backed rows. Any policy
version mismatch, `Unknown`, `No`, missing Definition material, or discarded
`is_async` observation prevents a runtime root; the typed diagnostic remains
scoped to its requirement.

### 1.3 Current graph promotion is unsafe

Current `EvidenceGraph::add_edge()` marks both endpoints connected for every
resolved `calls` edge. That makes a standalone known/search method actionable
without a platform/form entry. Task 7 replaces it with two-phase reachability:

1. collect/validate all typed edges and compatible base mechanisms;
2. seed reachability only from active base mechanisms;
3. propagate only in the directed caller -> callee direction.

### 1.4 Repeated one-hop queries require scoped invocations

One `CallGraphQuery` is one-hop. BFS therefore invokes Definition/Call ports
more than once. A flat per-port outcome cannot represent:

- `Complete(callers A)` plus `Unavailable(callers B)`;
- query-wide gap material to proposal A but unrelated to proposal B;
- complete empty proof for one exact caller frontier;
- analysis digest of the actual sequence of exact scopes.

Task 7 adds application-owned scoped invocation snapshots; it never converts a
query-local unavailable/failed issue into a generic bounded provider batch.

### 1.5 Application checks need their own closed identity

`maxCandidates`, traversal root/frontier/depth and support-subject truncation
are application decisions. Active `Check::validate()` currently permits only
evidence ports and `ProjectSourceResolverPort/source_readiness`. Add one closed
orchestration identity:

```text
provider = DiscoveryTraversal
code = graph_traversal | candidate_limit | support_selection
```

No other `DiscoveryTraversal` code is valid. This contract must be duplicated
in active spec/product tests before Task 7 implementation is accepted.

### 1.6 Proposal prose is not a typed expectation

`Proposal.intent` remains opaque. A corpus case cannot claim proposal
contradiction solely from a wrong event/action/verb that appears only in prose.
Task 7 contradicted cases use facts expressible by the current contract:

- exact target absent under complete scope;
- exact binding points to a different target;
- disabled scheduled job;
- incompatible canonical platform callback signature/context.

Event/action/verb mismatch remains a provider contract test unless a later ADR
adds a typed proposal expectation.

## 2. Exact file map

### Pre-Task 7 documentation gate

- Modify before Task 7 starts:
  `spec/architecture/extension-point-discovery.md`
  - replace `ExchangePlan + handles` with the exact
    `uses -> EventSubscription -> subscribes` chain;
  - document source-qualified provider gaps, the accepted Task 5A
    `DiscoveryTraversal/candidate_limit` tuple and per-destination metadata
    membership coverage. Form inspection remains analysis-only in v1.
- Modify the matching assertions in `tests/ci/test_product_contracts.py` in
  that prerequisite commit. Corpus seeding is forbidden while either file
  still accepts the old contract.

### Corpus gate

- Create: `tests/fixtures/project_discovery/fixture-cases.json`
  - strict registry of committed Task 5/6 micro-fixture case IDs and contained
    relative roots.
- Create: `tests/fixtures/project_discovery/corpus.json`
  - all 48 fixed mechanism cases and their complete expectations.
- Create: `tests/ci/test_project_discovery_corpus.py`
  - stdlib-only strict schema, cardinality, case-ID and fixture-reference gate.
- Create: `crates/unica-coder/src/application/discovery/corpus.rs`
  - `#[cfg(test)]` deny-unknown-fields Rust loader and fake-port case runner.

### Application contract and algorithms

- Modify: `crates/unica-coder/src/application/discovery/model.rs`
  - `TraversalGap`, source-scoped gap scope and closed reason codes;
  - `DiscoveryTraversal` check tuples;
  - exact validation/canonical ordering.
- Modify: `crates/unica-coder/src/application/discovery/ports.rs`
  - `ProviderQueryScope`, `ScopedProviderInvocation`, repeated-query
    collection and per-port canonical accumulator;
  - preserve Task 6 query-specific port signatures;
  - support query limit stays 4096 and is not narrowed by `maxEvidence`.
- Create: `crates/unica-coder/src/application/discovery/traversal.rs`
  - roots, stable batches, BFS, origin propagation and traversal coverage.
- Create: `crates/unica-coder/src/application/discovery/mechanisms.rs`
  - closed eight-family registry, exact base-path recognition, relevance and
    candidate policy.
- Modify: `crates/unica-coder/src/application/discovery/evidence_graph.rs`
  - edge collection separate from directed runtime reachability;
  - graph projection from accepted mechanism/traversal result.
- Modify: `crates/unica-coder/src/application/discovery/proposal_validator.rs`
  - exact scope coverage/materiality and fixed-point negative proof.
- Modify: `crates/unica-coder/src/application/discovery/use_case.rs`
  - replace eager six-port calls with the staged algorithm in section 6;
  - final global evidence limit, support stage, candidate limit and status.
- Modify: `crates/unica-coder/src/application/discovery/determinism.rs`
  - query/invocation/traversal/mechanism contract encodings;
  - analysis ID binds the execution snapshot.
- Modify: `crates/unica-coder/src/application/discovery/mod.rs`
  - declare test-only corpus plus traversal/mechanisms modules and focused
    contract tests.

### Native provider composition, still private

- Create: `crates/unica-coder/src/infrastructure/discovery/providers.rs`
  - crate-private `NativeDiscoveryProviders` composition over accepted Task
    5/6 adapters; no parsing and no orchestration policy.
- Modify: `crates/unica-coder/src/infrastructure/discovery/mod.rs`
  - crate-private construction export only.
- Modify: `crates/unica-coder/src/infrastructure/mod.rs`
  - only if Task 5/6 did not already declare `discovery`.
- Do not modify: `crates/unica-coder/src/interfaces/mcp.rs`.
- Do not modify: tool contracts/registry, `plugins/unica/.mcp.json`, plugin
  metadata, skills, provenance or third-party lock.

### Documentation and guardrails in the implementation commit

- Modify: `spec/architecture/extension-point-discovery.md`
  - add scoped invocations, traversal checks, directed reachability,
    depth/candidate semantics and corpus-before-wiring on top of the already
    corrected prerequisite matrix.
- Modify: `docs/superpowers/plans/2026-07-17-project-discovery-receipts.md`
  - make Task 7 steps reference the accepted contract rather than the old eager
    six-port wording.
- Modify: `tests/ci/test_product_contracts.py`
  - require the corrected spec statements and forbid public registration.

## 3. Corpus seed before wiring

### 3.1 Fixed families and variants

The seed contains exactly one case for every family x variant pair:

| Family tag | Family |
| --- | --- |
| `f1_document_lifecycle` | document lifecycle -> ObjectModule callback |
| `f2_event_subscription` | EventSubscription -> CommonModule handler |
| `f3_form_command` | registered form command/action -> FormModule handler |
| `f4_common_command` | CommonCommand -> canonical CommandModule callback |
| `f5_scheduled_job` | enabled ScheduledJob -> CommonModule handler |
| `f6_http_route` | HTTP route -> HTTPService Module handler |
| `f7_exchange_subscription` | ExchangePlan -> EventSubscription -> handler |
| `f8_report_processor_form` | Report/DataProcessor form command -> handler |

Variant tags are exactly:

```text
supported_primary
supported_alternative
contradicted
unknown_material_gap
lexical_decoy
registered_hard_decoy
```

The two supported rows per family are not implementer-selected. Their minimum
fixture semantics are frozen as follows; the fixture registry may add unrelated
nonmaterial fields but cannot substitute another mechanism shape:

| Family | `supported_primary` | `supported_alternative` |
| --- | --- | --- |
| f1 DocumentLifecycle | English synchronous BeforeWrite callback row | Russian synchronous ПередЗаписью row |
| f2 EventSubscription | Document BeforeWrite / 4-parameter class | Catalog+Constant BeforeWrite exact selected set / shared 2-parameter class |
| f3 FormCommand | synchronous non-exported `&AtClient Procedure(Command)` | asynchronous exported `&AtClient Procedure(Command)` |
| f4 CommonCommand | English synchronous CommandProcessing row | Russian synchronous ОбработкаКоманды row |
| f5 ScheduledJob | active predefined exported synchronous zero-arity Procedure | same exact active/profile row with exported synchronous zero-arity Function |
| f6 HttpRoute | GET + non-exported synchronous ModuleDefault Function(Request) | POST + exported synchronous ModuleDefault Function(Request) |
| f7 ExchangeSubscription | one selected ExchangePlan exact uses subset | ExchangePlan+Catalog shared-class selected set whose uses subset contains only the plan |
| f8 ReportProcessorForm | Report own Form/Command/Action chain | DataProcessor own Form/Command/Action chain |

Every Function/Procedure row above uses the parameter transfer/default shape
from Task 5B v5. Wider async/context/Val/default variants classified Unknown are
covered by permanent policy REDs, not silently substituted for one of these
positive corpus rows.

Case IDs are `<family-tag>__<variant-tag>`. The 48 IDs are generated only by
the Cartesian product of the two closed lists; the Python/Rust validators
reject missing, extra or duplicate pairs.

### 3.2 Fixture registry schema

`fixture-cases.json` is strict:

```rust
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct FixtureRegistryV1 {
    schema_version: u16,              // exactly 1
    cases: Vec<FixtureCaseV1>,         // sorted unique by id
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct FixtureCaseV1 {
    id: String,
    relative_root: String,             // contained path below project_discovery
    capabilities: Vec<FixtureCapability>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum FixtureCapability { PlatformXml, Bsl, Support }
```

Every root must exist, be contained, and contain only tracked fixture material.
The registry must reference the real IDs/names landed by Task 5/6; if those
fixtures have no stable IDs, stop and add IDs in their owning task first.

### 3.3 Corpus case schema

Each case has both explore and validate expectations so Task 7 cannot optimize
for only one mode:

```rust
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct CorpusV1 {
    schema_version: u16,               // exactly 1
    cases: Vec<CorpusCaseV1>,           // exactly 48, canonical IDs
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct CorpusCaseV1 {
    id: String,
    family: MechanismFamilyV1,
    variant: CorpusVariantV1,
    fixture_case_ids: Vec<String>,      // nonempty, sorted unique, registered
    faults: Vec<CorpusFaultV1>,         // sorted; usually only unknown case
    explore: CorpusInvocationV1,
    validate: CorpusInvocationV1,
    future_guard_expectation: FutureGuardExpectationV1,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct CorpusInvocationV1 {
    request: DiscoverRequest,           // invokes its strict custom Deserialize
    expected: CorpusExpectedV1,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct CorpusExpectedV1 {
    status: DiscoveryStatus,
    required_related: Vec<ArtifactExpectationV1>,
    forbidden_related: Vec<ArtifactRef>,
    required_edges: Vec<EdgeExpectationV1>,
    forbidden_edges: Vec<EdgeIdentityV1>,
    required_candidates: Vec<CandidateExpectationV1>,
    forbidden_candidates: Vec<ArtifactRef>,
    proposal_verdicts: Vec<ProposalExpectationV1>,
    checks: Vec<CheckExpectationV1>,
    receipt_eligible_with_probe_issuer: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum CorpusVariantV1 {
    SupportedPrimary,
    SupportedAlternative,
    Contradicted,
    UnknownMaterialGap,
    LexicalDecoy,
    RegisteredHardDecoy,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum FutureGuardExpectationV1 { AllowWithValidReceipt, NoReceiptIssued }

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ArtifactExpectationV1 {
    artifact: ArtifactRef,
    minimum_evidence_level: EvidenceLevel,
    exact_reason_codes: Vec<String>,
    required_evidence: Vec<EvidenceIdentityExpectationV1>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct EdgeIdentityV1 {
    from: ArtifactRef,
    to: ArtifactRef,
    kind: FlowKind,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct EdgeExpectationV1 {
    edge: EdgeIdentityV1,
    required_evidence: Vec<EvidenceIdentityExpectationV1>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct CandidateExpectationV1 {
    target: ArtifactRef,
    minimum_evidence_level: EvidenceLevel,
    support_state: SupportState,
    exact_reason_codes: Vec<String>,
    exact_blockers: Vec<String>,
    required_evidence: Vec<EvidenceIdentityExpectationV1>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ProposalExpectationV1 {
    proposal_id: String,
    verdict: Verdict,
    facts: ProposalFacts,
    exact_coverage_gaps: Vec<String>,
    exact_blockers: Vec<String>,
    required_evidence: Vec<EvidenceIdentityExpectationV1>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct CheckExpectationV1 {
    code: String,
    provider: String,
    state: CheckState,
    outcome: CheckOutcome,
    coverage: Coverage,
    severity: CheckSeverity,
    exact_affects: Vec<String>,
    reason_code: String,
    retryable: bool,
    exact_details: Vec<String>,
    required_evidence: Vec<EvidenceIdentityExpectationV1>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct EvidenceIdentityExpectationV1 {
    evidence_type: EvidenceType,
    source_set: String,
    subject: ArtifactRef,
    fact_code: String,
    object: Option<ArtifactRef>,
    location: Option<SourceLocation>,
    provider: EvidenceProvider,
    coverage: Coverage,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct CorpusFaultV1 {
    port: EvidencePort,
    stage: CorpusFaultStageV1,
    outcome: CorpusFaultOutcomeV1,
    reason_code: String,
    retryable: bool,
    scope: CorpusFaultScopeV1,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum CorpusFaultStageV1 {
    MetadataComposite,
    FormSourceSet { source_set: String },
    CodeSearch { source_set: String },
    InitialDefinition { source_set: String },
    CallGraph { source_set: String, depth: u8 },
    TraversedDefinition { source_set: String, depth: u8 },
    Support { round: u8 },
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum CorpusFaultOutcomeV1 { Bounded, Unavailable, Failed }

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", content = "artifacts", rename_all = "snake_case")]
enum CorpusFaultScopeV1 {
    Artifacts(Vec<SourceScopedArtifact>),
    SourceSetWide { source_set: String },
    QueryWide,
}
```

Every expectation list is explicitly present, sorted and unique; empty is not
the same as omitted. `exact_*` means set equality after the report's canonical
sort/dedup, while `required_evidence` resolves semantic evidence identities to
actual deterministic IDs and rejects a missing or ambiguous identity. Required
edges include exact from/to/kind. Candidate expectations include exact minimum
evidence level, support state, reason codes and blockers. Verdict expectations
include exact facts, coverage gaps and blockers. Check expectations include
exact provider/code/state/outcome/coverage/severity/reason/affects/details.
The evaluator also rejects unexpected related artifacts, edges, candidates,
proposal verdicts and checks; `required_*` names do not permit extra report
members outside the explicitly paired forbidden/exact sets.

`CorpusFaultV1` may inject only a typed bounded/unavailable/failed port outcome
at an exact stage and exact typed scope. It cannot inject raw evidence or
override an expected report. A conflict must be committed as conflicting typed
fixture facts, never synthesized by a fault. The unknown case for each family
must use a material gap, fixture conflict or deterministic bound; unrelated
degradation is covered separately.

`CorpusFaultScopeV1::QueryWide` means the enclosing exact invocation only. For
MetadataComposite it therefore means the whole composite query.
`SourceSetWide { source_set }` is valid only for MetadataComposite and must name
one exact captured internal group; it is material only to conclusions depending
on that group. It cannot name an unpaired sibling or appear on another stage.
Support's enclosing invocation contains its exact sorted
`SourceScopedArtifact` subjects, so QueryWide there means only that frozen
subject vector.

`future_guard_expectation` is fixed now so later Task 10 cannot shape it after
seeing output: supported validate cases use `AllowWithValidReceipt`; every
other case uses `NoReceiptIssued`. Task 7 validates but does not execute it.

### 3.4 Seed gate

Before mechanisms code:

```text
python3 -m unittest tests.ci.test_project_discovery_corpus -v
```

Must PASS schema/cardinality/reference checks. The test writes no normalized
copy and reads expectations only. Then the Rust seed loader:

```text
cargo test --locked -p unica-coder application::discovery::corpus -- --nocapture
```

must enumerate all 48 IDs, strict-deserialize both 48 explore and 48 validate
requests, and PASS schema/reference/expectation-shape validation before any
mechanism code exists. It does not execute the use case at this seed gate. The
behavior evaluator is added in Task 7.5 and must consume these frozen values
without modifying them. Zero cases, ignored tests, dynamically generated
expectations, missing fixture references or an evaluator that rewrites expected
output are failed gates.

## 4. Application-owned typed contracts

### 4.1 Scoped provider invocations

```rust
pub(crate) enum ProviderQueryScope {
    MetadataComposite {
        composite_snapshot_id: String,
        analysis_source_set: String,
        destination_source_sets: Vec<String>,
        pair_digest: String,
        presence_key_digest: String,
        form_material_scope_digest: String,
    },
    FormSourceSet { source_set: String },
    CodeSearch(CodeSearchQuery),
    Definition { depth: Option<u8>, query: DefinitionQuery },
    CallGraph { depth: u8, query: CallGraphQuery },
    Support { subjects: Vec<SourceScopedArtifact> },
}

pub(crate) struct ScopedProviderInvocation {
    pub(crate) port: EvidencePort,
    pub(crate) scope: ProviderQueryScope,
    pub(crate) query_digest: String,
    pub(crate) conclusion_scopes: BTreeSet<ConclusionScope>,
    pub(crate) outcome: CollectedProviderOutcome,
}

pub(crate) struct ScopedProviderRollup {
    pub(crate) port: EvidencePort,
    pub(crate) provider: EvidenceProvider,
    pub(crate) invocations: Vec<ScopedProviderInvocation>,
    pub(crate) retained_record_digests: Vec<String>,
}

pub(crate) enum ConclusionScope {
    Request,
    Proposal(String),
    Mechanism(MechanismKey),
}
```

Stable tags are fixed in declaration order: ProviderQueryScope 1..=6 and
ConclusionScope Request=1, Proposal=2, Mechanism=3. Strings and typed query
payloads follow the existing canonical encoder; enum debug/display text never
enters a digest.

Rules:

- query digest is domain-separated full SHA-256 over the port stable tag and
  exact typed scope;
- the MetadataComposite scope must equal the one captured composite snapshot;
  analysis source, canonical destination source groups, pair/presence/Form-
  material digests, and every internal group enter the query digest;
- every other invocation scope source-set must equal its query and captured
  snapshot;
- Definition/Call artifacts are exact method refs, sorted and unique;
- depth is absent for initial definitions and `1..=12` for definitions found
  after a call hop; CallGraph depth is `0..=11`;
- same `(port, query_digest)` may occur once only. A repeated request reuses
  the immutable outcome and unions its `conclusion_scopes`; it never calls the
  provider twice;
- all invocations of one port must report the same provider name/version;
- `QueryWide` provider gap is material only to that invocation's exact
  conclusion scopes, not every use of the port;
- `SourceSetWide(name)` is material only to conclusions depending on the exact
  matching internal group of MetadataComposite, or the exact source-set scope of
  another invocation; it never covers an equal ref in a sibling group;
- unavailable/failed invocations retain their own reason/retryable state.

Rollup is deliberately not a worst-state enum. It is the canonical vector of
all invocations plus the canonical retained-record prefix. `Complete(A)` and
`Unavailable(B)` therefore remain two outcomes. Checks are created per exact
invocation issue and are only a public projection: equal issue tuples may union
their exact material affects, but the execution snapshot never collapses their
invocations. A complete empty invocation proves absence only for its own exact
scope. The legacy flat `ProviderOutcomeSnapshot` is retained inside each
invocation; no synthetic port-wide snapshot replaces the vector.

### 4.2 Deterministic execution snapshot

```rust
pub(crate) struct ProviderInvocationSnapshot {
    pub(crate) query_digest: String,
    pub(crate) conclusion_scopes: BTreeSet<ConclusionScope>,
    pub(crate) outcome: ProviderOutcomeSnapshot,
}

pub(crate) struct AnalysisExecutionSnapshot {
    pub(crate) provider_rollups: Vec<ScopedProviderRollupSnapshot>,
    pub(crate) traversal_gaps: Vec<TraversalGap>,
}

pub(crate) struct ScopedProviderRollupSnapshot {
    pub(crate) port: EvidencePort,
    pub(crate) provider: EvidenceProvider,
    pub(crate) invocations: Vec<ProviderInvocationSnapshot>,
    pub(crate) retained_record_digests: Vec<String>,
}
```

Rollups sort by port stable tag; invocations sort by
`(query_digest, conclusion scopes, outcome digest)`; retained digests sort
bytewise. Duplicate ports or `(port, query_digest)` invocations are forbidden.
The invocation snapshot includes its canonical `conclusion_scopes`, so adding
a later origin to a cached caller outcome changes the execution snapshot even
though the provider is not called again. `analysisId` encodes:

```text
ANALYSIS_CONTRACT_VERSION = project-discovery-v2
MECHANISM_REGISTRY_VERSION = project-mechanisms/v1
TRAVERSAL_CONTRACT_VERSION = project-traversal/v1
SEMANTIC_ATOMIC_GROUP_REGISTRY = semantic-evidence-groups/v1
FORM_COMMAND_HANDLER_POLICY = form-command-handlers/v1
HTTP_SERVICE_HANDLER_POLICY = http-service-handlers/v1
```

plus the existing normalized request/source snapshots/limits and the complete
execution snapshot. This prevents identical evidence from receiving the old
analysis ID after reachability semantics change.

### 4.3 Traversal gaps

```rust
pub(crate) enum TraversalGapScope {
    Artifacts(Vec<SourceScopedArtifact>), // nonempty, sorted unique
    QueryWide,
}

pub(crate) struct TraversalGap {
    pub(crate) reason_code: TraversalGapReason,
    pub(crate) phase: TraversalPhase,
    pub(crate) scope: TraversalGapScope,
    pub(crate) depth: Option<u8>,
    pub(crate) conclusion_scopes: BTreeSet<ConclusionScope>,
}

pub(crate) enum TraversalPhase {
    AnchorSelection,
    RootSelection,
    FrontierSelection,
    NodeSelection,
    MechanismSelection,
    SupportSelection { round: u8 },
    CandidateSelection,
}

pub(crate) enum TraversalGapReason {
    TraversalRootLimit,
    TraversalFrontierLimit,
    TraversalNodeLimit,
    GraphDepthLimit,
    MechanismInstanceLimit,
    SupportSubjectLimit,
    SupportPlanningRoundLimit,
    CandidateLimit,
    UnsupportedConceptShape,
    UnsupportedKnownModuleExpansion,
}
```

Stable tags are fixed: TraversalGapScope Artifacts=1/QueryWide=2;
TraversalPhase declaration order 1..=7; TraversalGapReason declaration order
1..=10. Reordering the Rust enum without preserving these explicit tags is a
contract error.

Wire/check reason spellings are the snake-case enum names. Traversal gaps use
stable tags distinct from ProviderGap tags and participate in analysis ID.
Never place one in a provider outcome. `conclusion_scopes` is nonempty and
canonical. Artifact scope remains source-qualified; QueryWide is local to the
exact `phase`/depth/origins, not global discovery. Phase/reason combinations
are closed and validated (for example CandidateLimit only with
CandidateSelection and support reasons only with SupportSelection).
`GraphDepthLimit` carries
`depth=Some(maxGraphDepth)`; root/support/candidate/concept/mechanism limits use
None; frontier/node gaps carry the exact level. `CandidateLimit` carries only
`Request`, never a Proposal scope, because explicit proposals bypass the
candidate budget.

The full phase mapping is: root limit -> RootSelection; frontier and graph
depth -> FrontierSelection; node limit -> NodeSelection; mechanism instance
limit -> MechanismSelection; both support limits -> SupportSelection(round);
candidate limit -> CandidateSelection; unsupported concept/module expansion ->
AnchorSelection. Any other pair fails validation.

## 5. Anchors and relevance without domain dictionaries

### 5.1 Anchor sources

Task 7 builds context anchors from exactly:

1. every proposal target;
2. every `knownArtifacts` entry;
3. every CodeOccurrence subject produced for exact `searchTerms`;
4. generic exact concept matches against artifacts already observed by typed
   providers.

`task` is never parsed. No synonym, stemming, transliteration, business
dictionary, fuzzy match, substring or embedding is used.

### 5.2 Generic concept normalization

- Unicode lowercase;
- split concept on characters other than Unicode alphanumeric or `_`;
- split artifact canonical ref on `.` and `_`;
- a concept matches only when all its nonempty tokens occur as full artifact
  tokens in order;
- no CamelCase splitting;
- punctuation-only concept creates `unsupported_concept_shape` only when it is
  the sole possible anchor; otherwise it is a normal no-match.

This normalization may find `Sales.Order` for `sales order`; it does not equate
`ShelfLife` with `expiry`, `GoodsReceipt` with `receipt`, or `Posting` with
`Write`.

### 5.3 Runtime roots versus context roots

```rust
pub(crate) enum TraversalRootClass {
    RuntimeMechanism,
    ContextOnly,
}
```

- every fully proven base mechanism handler is a runtime root, even before
  relevance filtering; this is required to prove incoming reachability;
- exact method proposal/known/code anchors are context roots unless the same
  method is also a proven runtime root;
- a known Module cannot enumerate all of its methods under Task 6's exact
  method query. It remains related and receives
  `unsupported_known_module_expansion` unless a typed mechanism/search result
  supplies exact method roots;
- when one artifact has both classes, RuntimeMechanism wins for root ordering
  and all `ConclusionScope` origins are retained.

### 5.4 Relevance projection after traversal

A mechanism is relevant when an anchor intersects its base path or a reachable
call node. Projection rules:

- base-path anchor: retain the complete base path and all traversed descendants
  up to the accepted depth;
- downstream anchor: retain all directed shortest paths from the base handler
  to the anchored node, not sibling branches;
- multiple equal shortest paths are all retained in canonical edge order;
- a standalone context root without a typed runtime path remains related but
  never connected/actionable;
- ownership closure for a retained artifact is included without consuming call
  depth;
- evidence may contain additional canonical records used to prove coverage,
  but relatedArtifacts/flowEdges/candidates are the relevant projection.

## 6. Exact staged orchestration

### Stage 0: source and snapshot

Keep Task 4/5 behavior: resolve all analysis/destination identities, capture
one composite snapshot, normalize diagnostic epoch, return EDT diagnostic
report before constructing any evidence query.

The captured metadata worklist is the analysis source followed by all unique
requested mutation destinations in canonical source-set-name order. Equal
artifact refs in different entries remain distinct `SourceScopedArtifact`s.

### Stage 1: exact composite metadata seed

Invoke MetadataCatalogPort exactly once with the Task 5B v5 typed composite
query. That query contains the analysis full registered scan, every exact
analysis presence key, all exact destination membership pairs, and the frozen
Form material scopes. The provider processes deterministic internal groups:

1. analysis source group;
2. one group per paired destination source in canonical source-set order.

An unpaired captured sibling is not a group and is never read. A
`SourceSetWide(destination-A)` gap is local to conclusions that require
destination A; it does not degrade analysis or destination B. `QueryWide` is
legal only when the whole composite invocation is invalid. The one scoped
invocation stores the exact composite query digest, including group/pair/
presence/material-scope digests.

Then invoke FormInspectionPort exactly once with
`FormSourceSet { analysis_source_set }`, and CodeSearchPort once with the exact
Task 6 `CodeSearchQuery` over the analysis source. The v1 CfePatchMethod intent
does not authorize destination FormInspection/FormCommand scanning. Destination
MetadataCatalog facts contribute proposal and mutation safety facts but never
create analysis runtime roots, mechanism instances, or explore candidates.

Collect all three calls as scoped invocations. Do not invoke Support yet.
Contract violations are fatal; bounded/unavailable/failed states remain scoped
to exact internal groups/material. Complete analysis-group coverage proves
nothing about an equal ref in a destination group. Analysis FormInspection
coverage is not reused as a destination fact and no destination FormCommand
absence is claimed in v1.

### Stage 2: initial definitions and mechanism bases

Build the exact method set from:

- every typed binding/callback handler object from Stage 1;
- proposal/known/code-occurrence method anchors.

The first bullet means positive or still-pending handler requirements only. A
`ScheduledJobActivationV1::Disabled` contributes exact
`scheduled_job_disabled` negative authority but does not schedule its
MethodName for Definition solely because that disabled job exists. If the same
method is independently an anchor or another active requirement, normal
deduplication still queries it for that other scope. The same no-endpoint rule
applies to nonpredefined, profile-invalid, malformed-Use/Predefined/MethodName,
or otherwise metadata-Unknown jobs. Only exact Use=true, Predefined=true,
Global=false, Server=true with valid registered MethodName/owner contributes a
ScheduledJob Definition target.

Sort/deduplicate, apply the application root bound, split into stable chunks of
at most 2000, and invoke DefinitionPort for each chunk. Build base mechanism
instances only after definitions and the accepted Task 5B compatibility join
is known. Task 7 consumes the tri-state result for EventSubscription,
ScheduledJob, callbacks, FormCommand, and HTTPService; it never locally reduces
compatibility to kind/arity/context or ignores `is_async`, parameter transfer,
default, Export, owner, or policy version.

### Stage 3: bounded directed BFS

Initialize runtime roots from complete base mechanisms and context roots from
exact method anchors. Maintain the per-origin distances from section 7.3. For
call depth `d` beginning at zero:

1. canonical frontier = positively defined methods with at least one
   unprocessed origin at distance `d`; callers with a cached outcome need
   replay, not another provider invocation;
2. apply frontier/node bounds and stable chunks;
3. invoke `CallGraphQuery` once per chunk with `depth=d` and
   `max_records=request.limits.maxEvidence`;
4. retain only `(Resolved, Artifact(target))` as a directed call edge;
5. collect exact new target methods from retained call records;
6. invoke DefinitionPort in stable chunks for new targets at depth `d+1`;
7. add only uniquely and unconditionally defined targets to the next frontier;
8. propagate each origin only when its own distance is below maxGraphDepth;
   only Mechanism origins propagate runtime reachability;
9. merge repeated provider records through the per-port canonical accumulator.

Stop successfully only when the origin work queue is empty. If positively
defined nodes are created at distance `maxGraphDepth`, do not propagate those
origins over outgoing calls and add `graph_depth_limit` for those exact
analysis-scoped methods/origins. A cached call result obtained for a shorter
origin does not waive the longer origin's depth bound.

### Stage 4: preliminary graph and support scope

Build the preliminary relevant graph from the current per-port canonical
records. Form the distinct exploratory candidate targets, sort by the exact
candidate-selection key from section 7.2, and retain the first
`maxCandidates` only as the support-planning prefix. Record
`omitted_preliminary`; do not remove omitted targets, evidence or edges from
the graph. Derive support subjects from:

- every target in that exploratory prefix and its exact ownership chain;
- every proposal target/ownership chain in analysis;
- the same proposal target/ownership chain in each CFE destination source.

Explicit proposal targets are unioned after the exploratory prefix and never
consume `maxCandidates`. Sort/deduplicate and invoke the first SupportStatePort
round for the new subjects. Across all rounds, maintain one monotonically
growing set of at most `MAX_SUPPORT_QUERY_SUBJECTS=4096` semantic subjects;
never pass an oversized plan to the port. `maxEvidence` limits returned
records, not semantic subjects. Overflow creates application
`support_subject_limit`, never a SupportStatePort gap.

### Stage 5: final evidence, graph, verdicts and output bounds

1. Merge all support outcomes collected so far.
2. Enforce per-port atomic-group canonical maxEvidence across all invocations.
3. Enforce the one global atomic-group canonical maxEvidence across six ports.
4. Rebuild graph/mechanisms/reachability from retained evidence only.
5. Compute the current canonical public-candidate prefix and all explicit
   proposal ownership subjects. If any subject has never been support-queried,
   take the canonical new-subject prefix that still fits the global 4096
   semantic-subject budget, invoke one new SupportStatePort round, and return
   to step 1.
6. Stop the loop when the rebuilt prefix is unchanged and all selected/
   explicit subjects were queried, when the subject budget is exhausted, or
   after 64 rounds. Budget/round termination adds the exact application
   support-selection gap; it is never a fatal SupportQueryPlan construction
   error. Every productive round adds at least one previously unqueried subject.
7. On the stable retained graph, recompute exact materiality and proposal
   verdicts. A selected candidate whose support subject was omitted by the
   application gap remains Connected/Unknown with that scoped blocker.
8. Build checks from scoped provider invocations plus traversal gaps.
9. Canonicalize the stable public candidate set, retain the first
   `maxCandidates`, and record `omitted_final`. Merge preliminary/final omitted
   artifact scopes into one application candidate-limit gap/check.
10. Compute status and receipt eligibility from the final retained state.
11. Compute analysis ID from the execution snapshot; canonicalize report.

No stage may return a candidate/edge/evidence ID derived from a record removed
by a later per-port/global limit. Support rounds and their exact query outcomes
remain in the scoped rollup even if later global selection displaces the branch
that caused the query.

## 7. BFS, bounds and canonical order

### 7.1 Server-owned bounds

```rust
const MAX_TRAVERSAL_ROOTS: usize = 4_096;
const MAX_TRAVERSAL_FRONTIER_METHODS: usize = 4_096;
const MAX_TRAVERSAL_QUERY_METHODS: usize = 2_000; // Task 6 query contract
const MAX_TRAVERSAL_NODES: usize = 53_248;        // 4096 * (12 + 1)
const MAX_MECHANISM_INSTANCES: usize = 4_096;
const MAX_SUPPORT_PLANNING_ROUNDS: usize = 64;
const MAX_TRAVERSAL_GAPS: usize = 256;
```

All arithmetic is checked. When an exact artifact list exceeds 2000 gap
subjects, gap scope becomes QueryWide for that application traversal phase.
Before enforcing `MAX_TRAVERSAL_GAPS`, aggregate by `(reason stable tag, phase,
depth)`, union canonical conclusion scopes and artifact scopes, and promote an
oversized artifact union to QueryWide. More than 256 gaps after this closed
aggregation is an internal contract violation, not silent truncation.

### 7.2 Canonical order

- artifacts: `ArtifactRef::identity_key()`;
- roots: merged origin state, then exact runtime-first key from section 7.3;
- BFS: ascending depth, then artifact identity;
- query chunks: consecutive canonical slices of at most 2000;
- calls: evidence-record digest, then edge identity;
- mechanism instances: family tag, entry identity, handler identity, base-edge
  identities;
- candidates: `(kind stable tag, Unicode-lowercase canonical ref, exact
  canonical-ref UTF-8 bytes)`; deduplicate by the first two fields and keep the
  lexically smallest exact spelling;
- traversal gaps: reason tag, phase tag/round, scope tag/subjects, depth,
  conclusion scopes.

Provider/filesystem return order never changes a frontier, query or output.

### 7.3 Root state and exact Calls progression

Traversal state is source-qualified and origin-aware:

```rust
pub(crate) struct TraversalNodeState {
    pub(crate) method: SourceScopedArtifact,
    pub(crate) minimum_depth: u8,
    pub(crate) origin_depths: BTreeMap<ConclusionScope, u8>,
    pub(crate) queried_at_depth: Option<u8>,
}
```

Only analysis-source Methods may enter this map. A proven mechanism handler is
inserted at depth 0 with `ConclusionScope::Mechanism(key)` and is therefore a
runtime root. Exact proposal/known/search Method anchors are inserted at depth
0 with `Proposal(id)` or `Request`, but those origins are context-only. An
artifact with both origins keeps both. A node is runtime-reachable iff at least
one `Mechanism` origin reached it; `Request`/`Proposal` alone never promote it.

Root selection first merges equal method identities and all origins, then sorts
`(has-runtime-origin first, source-set, kind stable tag, Unicode-lowercase ref,
exact ref bytes, canonical origins)`. It keeps the first
`MAX_TRAVERSAL_ROOTS`; omitted roots create one exact root-limit gap carrying
their conclusion scopes. No provider is queried for an omitted root.

Progression is level-synchronous and uses these exact rules:

1. At level `d`, union every origin that reached a method at its own distance
   `d`; all equal-depth paths arrive before any caller query for that level.
2. A method is Definition-eligible only after a retained unique positive
   definition. Absent, ambiguous, dynamic or unavailable definitions do not
   enter the Call frontier.
3. Query each distinct caller method at most once, at its global minimum depth,
   and cache the immutable one-hop outcome. Later origins reaching that caller
   reuse it and are unioned into the invocation's conclusion scopes.
4. For every origin at distance `d < maxGraphDepth`, replay only retained
   `CallResolution::Resolved + CallTarget::Artifact(Method)` edges whose caller
   exactly equals the queried method. The callee receives that origin at
   `d + 1`; dynamic/unresolved/ambiguous/non-Method targets remain evidence but
   never become edges or frontier nodes.
5. If an origin reaches a positively defined method at distance exactly
   `maxGraphDepth`, retain that node/path but do not propagate that origin over
   cached or new outgoing edges. Add `graph_depth_limit` for that exact method,
   depth and origin. Another origin that reaches the same method at a smaller
   distance may still use the cached one-hop result within its own budget.
6. When a later origin reaches an already queried caller, replay its cached
   edges and cached gap to that origin without a provider call. Continue this
   origin work queue to a fixed point; `(method, origin, depth)` is processed
   once. This prevents cycles and makes shared-tail materiality independent of
   discovery order.
7. Base binding edges consume zero depth. The first BSL Calls edge from a base
   handler is hop 1. Therefore maxGraphDepth=N retains at most N directed Calls
   hops per origin; it never means N mechanism/ownership edges.

The fixed point is complete only when every retained runtime root was included,
every origin work item below the bound has a Complete call outcome, every new
callee has a Complete exact definition outcome, no traversal/provider limit is
material, and the origin queue is empty. Merely having queried each distinct
method once is insufficient if a cached outcome has not yet been replayed to a
later origin.

### 7.4 Per-port accumulator

At the end of each BFS depth, merge all records seen for a port through Task 5B
v5's closed `SemanticAtomicGroupIdV1` classifier. Platform XML cross-fact
clusters retain their exact group identity; records from other ports use the
registry's `StandaloneFact` unless their accepted provider contract defines a
stronger closed cluster. Fact tag is an inner-record order key, never permission
to split a CFE polarity/companion, EventSubscription descriptor/derived-uses set,
or complete Form catalog.

For a per-port ceiling, sort groups by the exact canonical group key and retain
an atomic canonical prefix counted in physical evidence records. On the first
whole group that does not fit `maxEvidence`, drop that group and every later
group; never skip an oversized group for a smaller later one. For the one global
six-port ceiling, sort the already retained groups by
`(minimum full evidence-record digest in group, port stable tag,
SemanticAtomicGroupIdV1 canonical bytes, complete cluster digest)` and apply the
same whole-group prefix-stop rule. A group never spans ports. Dropped groups
create exact source-scoped evidence-limit gaps over their closed whole-cluster
material subjects. The accumulator retains query invocation snapshots even when
their groups are later omitted.

Every record carries the set of `(port, query_digest)` invocations that returned
it. Equal digests deduplicate as one budget/output record only when the full
records are byte-identical; a digest collision is fatal. Group classification is
performed after that semantic dedup and before either ceiling. Limit projection
rewrites every owning invocation that contributed any record to a dropped group
to Bounded, adds the exact whole-group source-qualified evidence-limit gap,
removes **all** records of that group from its final outcome snapshot, and
recomputes its outcome digest. An invocation that returned the same retained
group keeps its records even if another invocation also owns them. There is no
synthetic port-wide gap, no partly retained cluster, and no stale Complete
invocation after one of its groups was removed.

The next frontier is derived only from call/definition records retained after
that depth merge. A later canonical displacement may orphan earlier deeper
records; Stage 5 rebuild discards every orphaned edge/candidate. Its invocation
snapshot remains in the analysis ID because the call occurred, but its issue is
not automatically material merely because `conclusion_scopes` once named that
branch. Final affects are recomputed against retained paths and exact proposal
proof obligations. An orphan-only issue becomes Warning with empty affects;
the separate evidence-limit gap, when material, owns the final Unknown result.

### 7.5 Edge and candidate truncation

- There is no independent `maxEdges`. Every public FlowEdge requires at least
  one retained evidence record, so global maxEvidence already bounds edges.
- maxCandidates never removes evidence, related artifacts, flow edges or
  proposal verdicts; its preliminary use only bounds exploratory support
  planning for targets that cannot fit in the public result.
- explicit proposal targets and their analysis/destination ownership subjects
  are always support-queried outside the exploratory budget.
- classify the selected exploratory candidates, sort by the exact candidate
  key, and retain exactly the first `maxCandidates` public candidates.
- N candidates at limit N is complete; N+1 creates `candidate_limit` scoped to
  the union of omitted preliminary/final analysis artifacts.
- omitted candidate refs cannot appear in public `check.affects`, so the
  candidate-limit check has empty affects and warning severity.
- a proposal target omitted from the public candidate list is still validated
  from the full graph; it is not pinned ahead of canonical candidates and its
  receipt eligibility is unchanged.

`maxCandidates` is already encoded in the normalized request. The v2 analysis
ID additionally binds the canonical CandidateLimit traversal-gap scope, while
the display check's `omitted_*` counts are derived and are not a second digest
input. Provider snapshots/evidence IDs do not change merely because candidate
output was truncated. Any later change to selection key, preliminary/explicit
semantics or gap encoding requires another analysis/traversal contract bump.

## 8. Closed mechanism registry v1

```rust
pub(crate) enum MechanismFamilyV1 {
    DocumentLifecycle,
    EventSubscription,
    FormCommand,
    CommonCommand,
    ScheduledJob,
    HttpRoute,
    ExchangeSubscription,
    ReportProcessorForm,
}

pub(crate) struct MechanismKey {
    pub(crate) family: MechanismFamilyV1,
    pub(crate) entry: ArtifactRef,
    pub(crate) handler: ArtifactRef,
}

pub(crate) struct MechanismInstance {
    pub(crate) key: MechanismKey,
    pub(crate) owners: Vec<ArtifactRef>,
    pub(crate) base_edges: Vec<EdgeIdentity>,
    pub(crate) entry_candidates: Vec<ArtifactRef>,
    pub(crate) evidence_ids: Vec<String>,
}
```

`MechanismFamilyV1` stable tags are the declaration order 1..=8. MechanismKey
encoding is `(family tag, exact entry ArtifactRef, exact handler ArtifactRef)`;
MechanismInstance encoding appends canonical owners/base edges/candidates/
evidence IDs. The registry version, not Rust enum spelling, owns these tags.

Every list is sorted/unique. Instances are constructed only from retained
evidence and exact compatible definitions.

| Family | Required base path | Candidate-capable base targets |
| --- | --- | --- |
| DocumentLifecycle | Document owner --handles--> canonical ObjectModule callback | handler Method |
| EventSubscription | EventSubscription --subscribes--> CommonModule Method | subscription, handler Method |
| FormCommand | non-Report/DataProcessor FormCommand --handles--> own FormModule Method | command, handler Method |
| CommonCommand | CommonCommand --handles--> compatible canonical CommandModule callback | command, handler Method |
| ScheduledJob | enabled ScheduledJob --handles--> CommonModule Method | job, handler Method |
| HttpRoute | HTTPRoute --handles--> own HTTPService Module Method | route, handler Method |
| ExchangeSubscription | ExchangePlan --uses--> EventSubscription --subscribes--> Method | subscription, handler Method; ExchangePlan related only |
| ReportProcessorForm | Report/DataProcessor owner -> Form -> FormCommand --handles--> FormModule Method | command, handler Method; owner/form related only |

For EventSubscription and ExchangeSubscription, “validated” means the retained
Task 5B v5 whole descriptor plus its compatible Definition. The graph may consume
the descriptor's already-proved selected set and derived exact ExchangePlan
subset; it must never infer validity from a lone `uses` or `subscribes` edge,
recompute the 13-family/21-row registry, or use a default signature class. A gap
in any descriptor/Definition/companion material prevents both families from
seeding runtime reachability.

For FormCommand and ReportProcessorForm, “compatible” is exclusively a retained
`form-command-handlers/v1` Yes join: exact own FormModule, Procedure, one
by-reference nondefaulted parameter, explicit AtClient, either sync/async, and
Export nonmaterial. For HttpRoute it is exclusively a retained
`http-service-handlers/v1` Yes join: exact same-service Module, synchronous
unannotated Function, one by-reference nondefaulted parameter, and Export
nonmaterial. A complete hard mismatch remains No; a wider unproven shape remains
Unknown exactly as Task 5B classified it. Neither outcome is recalculated from
the raw Definition in this registry.

All handlers require exact DefinitionPresent and registered owner existence.
CommonCommand/Document callbacks, EventSubscription, and ScheduledJob additionally
require the accepted synchronous compatibility result; Form commands and HTTP
routes require the two policies above. A complete `Use=false` job is exact
`scheduled_job_disabled`, No runtime activation; it never becomes an instance
and is not confused with Unknown nonpredefined or malformed-Use cases. Its
negative comes from the separate activation fact and survives absent/gapped
Definition or malformed non-activation siblings. Unknown
script variants, unsupported direct exchange callbacks, BSP print pipelines and
untyped report conventions never become instances.

A subscription participating in family 7 may also form a family-2 instance.
Relevance anchors decide which path is projected; candidate identities still
deduplicate globally.

Downstream uniquely defined Methods reached by directed calls are
candidate-capable with reason `reachable_via_call`. They inherit the family
path but never turn a ContextOnly root into a runtime root.

## 9. Evidence graph and candidate semantics

### 9.1 Two-phase graph

Phase A stores:

- presence/absence/definition/support facts and conflicts;
- structural edges as observed only;
- verified binding edges;
- resolved call edges as observed directed edges;
- pending platform callbacks and exact runtime rejections.

Phase B:

1. create base mechanisms from section 8;
2. mark their entry/base-handler path runtime-connected;
3. promote complete ExchangePlan chain only when both required edges exist;
4. traverse Calls caller -> callee from runtime handlers;
5. mark only reached nodes connected;
6. project relevant edges and ownership closure.

### 9.2 Evidence levels

- Lexical: CodeOccurrence only.
- Observed: exact positive metadata/definition/binding observation.
- Connected: exact target lies on a retained directed runtime mechanism path.
- Actionable: Connected + target is candidate-capable for that mechanism +
  exact existence/ownership + one known projected support state + no material
  conflict/provider/traversal gap.

An unsupported/degraded sibling branch does not demote a proven candidate.
One material gap on the candidate's path keeps it Connected with exact blocker.

### 9.3 Candidate reason codes

Closed family reason codes:

```text
document_lifecycle_callback
event_subscription_handler
form_command_handler
common_command_handler
scheduled_job_handler
http_route_handler
exchange_subscription_handler
report_processor_form_handler
reachable_via_call
actionable_extension_point
```

No family is inferred from artifact spelling alone; the classifier must possess
the exact typed base facts.

## 10. Materiality and proposal verdicts

### 10.1 Scope coverage index

```rust
pub(crate) struct ScopeCoverageIndex {
    pub(crate) invocations: Vec<ScopedProviderInvocation>,
    pub(crate) traversal: TraversalCoverage,
}

pub(crate) struct TraversalCoverage {
    pub(crate) complete_callers: BTreeSet<SourceScopedArtifact>,
    pub(crate) runtime_fixed_point: bool,
    pub(crate) gaps: Vec<TraversalGap>,
}
```

`ProviderGapScope::QueryWide` is interpreted against its enclosing invocation.
`SourceSetWide(name)` intersects only conclusions requiring that exact captured
source set. Artifact gaps compare both source-set and artifact. Equal refs from
analysis and destination never intersect accidentally.

`conclusion_scopes` records why an invocation was scheduled; it is not itself
a final blocking decision. Stage 5 intersects the invocation's exact query/gap
scope with the rebuilt retained mechanism paths and each proposal's positive
or negative proof obligation. Scopes reachable only through a later-displaced
or orphaned branch receive no proposal/candidate affects. They remain visible
as unrelated warnings and in the deterministic execution snapshot.

### 10.2 Exact positive and negative proof

Method existence is Yes only with unique DefinitionPresent plus all registered
owners. It is No only with exact DefinitionAbsent/complete exact definition
scope or complete exact absence proof. Any relevant gap/conflict is Unknown.

Method runtime reachability is:

- Yes when target lies on a retained directed path from a base mechanism;
- No only when all whole-analysis entry providers are complete, all runtime
  roots were included, every reached caller query is complete, traversal
  reached an empty fixed point before maxGraphDepth, and target is not reached;
- Unknown otherwise.

For declarative targets, an exact complete supplying Metadata/Form scope can
prove No without BSL fixed point. Structural edges alone never prove Yes.
A complete ScheduledJob `Use=false` is the dedicated declarative No
`scheduled_job_disabled`; missing/malformed/conflicted Use and
`Use=true, Predefined=false` remain Unknown and cannot borrow that negative.

### 10.3 Two unrelated proposals

Every node/invocation retains proposal conclusion scopes. If caller frontier A
for proposal A is unavailable and proposal B has a separate complete direct
mechanism:

- A is Unknown with only A in blocking check affects;
- B remains Supported with no copied coverage gap;
- overall validate status is Insufficient because one selected proposal is
  Unknown;
- receipt eligibility is false all-or-nothing;
- evidence/checks still show B's satisfied proof.

No first-proposal/global-port shortcut is allowed.

### 10.4 Verdict order

1. exact exists=No or runtimeReachable=No -> Contradicted;
2. conflict/blocker/material gap -> Unknown;
3. exists=Yes + runtimeReachable=Yes + known compatible support + no material
   gap -> Supported;
4. otherwise Unknown.

Support unknown does not erase an already conclusive existence/runtime
contradiction. Proposal prose never supplies facts.

## 11. Checks, status and receipt eligibility

### 11.1 Provider checks

Build the semantic issue tuple from exact invocation outcome, port code,
provider, state/outcome/coverage/severity/reason/retryable/details/evidenceIds.
Group only equal tuples, union their final material public affects, sort/dedup,
and split consecutive affects into chunks of at most 128; an empty-affects
issue still emits one check. Exact duplicate checks may then deduplicate. This
public projection does not replace the scoped execution rollup: a Complete(A)
scope can coexist with Unavailable(B), and negative proof still consults their
separate exact queries. Check affects contains only final retained public
candidate/proposal IDs derived from exact scope.

### 11.2 DiscoveryTraversal tuples

Graph/support gaps use:

```text
provider=DiscoveryTraversal
state=passed
outcome=inconclusive
coverage=bounded
retryable=false
```

`graph_traversal` reasons:
`traversal_root_limit`, `traversal_frontier_limit`,
`traversal_node_limit`, `graph_depth_limit`,
`mechanism_instance_limit`, `unsupported_concept_shape`,
`unsupported_known_module_expansion`.

`support_selection` reasons: `support_subject_limit` and
`support_planning_round_limit`. Both are application gaps; their affects are
the selected public candidate/proposal IDs whose exact subjects remain
unqueried after the deterministic stop.

`candidate_limit` uses reason `candidate_limit`, always Warning with empty
affects because omitted candidates are not public IDs and explicit proposal
validation bypasses both candidate prefixes. It emits exactly one check when
either count is nonzero. `details` is the sorted nonempty subset of
`omitted_preliminary=N` and `omitted_final=N`, with positive base-10 N;
evidenceIds is empty.

Material graph/support gaps are Blocking; unrelated ones Warning. A no-gap
traversal emits exactly one canonical check with
`graph_traversal/DiscoveryTraversal/passed/satisfied/complete/info`, empty
affects/details/evidenceIds, reason `complete`, `retryable=false`. The closed
validator rejects every other DiscoveryTraversal provider/code/reason/tuple.

### 11.3 Status

- Explore Insufficient iff no Actionable retained candidate.
- Validate Insufficient iff at least one selected proposal is Unknown.
- Otherwise Partial iff any inconclusive/conflict/degraded check remains,
  including unrelated warning/candidate truncation.
- Otherwise Complete.

A blocking issue for one non-actionable explore branch cannot erase another
actionable candidate; it makes the report Partial. A fully contradicted
validate proposal is conclusive and may yield Complete.

### 11.4 Receipt eligibility in Task 7

Eligibility probe is invoked only in validate when every proposal is Supported
and every check material to those proposals is satisfied/not-applicable. An
unrelated warning/candidate limit does not block. If one proposal is Unknown or
Contradicted, probe is not invoked.

Corpus uses `EligibilityProbeIssuer` to assert the gate. Native private
composition uses no-op issuer returning:

```text
eligible=false
blockers=[receipt_store_not_implemented]
```

No receipt ID/store/lease is implemented here.

## 12. Native composition without public MCP

```rust
pub(crate) struct NativeDiscoveryProviders {
    pub(crate) source_resolver: FilesystemProjectSourceResolver,
    pub(crate) snapshots: FilesystemSourceSnapshots,
    pub(crate) metadata: PlatformXmlMetadataCatalogProvider,
    pub(crate) forms: PlatformXmlFormInspectionProvider,
    pub(crate) bsl: SnapshotBslEvidenceProvider,
    pub(crate) support: SnapshotSupportStateProvider,
}
```

The composition object only owns adapters and provides crate-private references
for a future Task 12 application entry point. It does not parse requests,
select roots, call another adapter or register a tool. One in-process fixture
smoke constructs the real set and executes the use case directly.

Product-contract tests must prove:

- no `ProjectDiscover` registry/tool-contract arm;
- no `unica.project.discover` in plugin MCP/schema/skills;
- no discovery import of workspace index/SQLite/display BSL parsing;
- one public MCP server remains `unica`.

## 13. RED -> GREEN implementation sequence

### Task 7.0: Freeze corpus before code

- [ ] Verify every section-1 prerequisite and the pre-Task 7 spec/product gate;
  if any fails, STOP before creating corpus or mechanisms code.
- [ ] Create strict fixture registry/corpus and Python test.
- [ ] Add all 48 fixed IDs, real Task 5/6 fixture references, both mode
  requests and full expected sets.
- [ ] Run Python gate; expected PASS with exactly 48 cases and zero dangling
  fixture IDs.
- [ ] Add the Rust deny-unknown seed loader; assert 48 IDs, 96 strict requests,
  closed enums and canonical expectation lists without executing discovery.
- [ ] Run the Rust corpus seed test; expected PASS and never zero tests.
- [ ] Commit only corpus/schema/seed loader; record the commit SHA as
  `CORPUS_SEED_COMMIT`:
  `test: зафиксировать corpus project discovery`.
- [ ] From this point through Task 7.5, any diff to `fixture-cases.json` or
  `corpus.json` is a STOP requiring owner review; implementation must conform
  to the seed, not rewrite it.

### Task 7.1: Scoped invocations and traversal contracts

- [ ] Add RED tests:
  - `query_wide_gap_is_local_to_one_call_frontier`;
  - `equal_refs_in_two_source_sets_do_not_share_materiality`;
  - `complete_analysis_scan_does_not_prove_destination_absence`;
  - `metadata_runs_once_for_composite_snapshot_and_form_runs_analysis_only`;
  - `metadata_source_set_wide_gap_is_local_to_one_composite_group`;
  - `complete_and_unavailable_call_invocations_both_survive`;
  - `orphaned_invocation_gap_has_no_final_proposal_affects`;
  - `query_digest_changes_for_depth_and_exact_scope`;
  - `discovery_traversal_check_rejects_unknown_tuples`;
  - `analysis_id_changes_for_traversal_contract_and_gaps`.
- [ ] Run application discovery tests and record RED missing types.
- [ ] Implement model/ports/determinism contracts and canonical accumulator.
- [ ] Re-run expected GREEN.
- [ ] Commit: `feat: добавить scoped orchestration discovery`.

### Task 7.2: Two-phase graph and mechanism registry

- [ ] Add RED tests for each section-8 base path plus:
  - `standalone_resolved_call_is_observed_not_connected`;
  - `connected_caller_propagates_to_callee_only`;
  - `reverse_call_direction_never_propagates`;
  - `structural_edges_never_seed_runtime`;
  - `disabled_job_is_exact_scheduled_job_disabled_no_and_never_seeds_runtime`;
  - `missing_or_malformed_use_is_unknown_not_disabled_no`;
  - `disabled_job_ignores_nonactivation_gaps_and_schedules_no_definition`;
  - `exchange_requires_uses_and_subscribes`;
  - `event_subscription_requires_complete_v1_descriptor_and_definition`;
  - `event_subscription_descriptor_covers_all_13_families_21_rows_and_3_classes`;
  - `exchange_uses_set_matches_descriptor_exchange_plan_subset`;
  - `task7_never_recomputes_event_source_compatibility`;
  - `incompatible_or_defaulted_signature_class_never_seeds_runtime`;
  - `task7_requires_retained_form_command_policy_yes`;
  - `form_command_sync_and_async_rows_seed_but_guessed_module_default_does_not`;
  - `task7_requires_retained_http_service_policy_yes`;
  - `http_function_request_seeds_but_procedure_arity_async_and_context_variants_do_not`;
  - `task7_never_recomputes_form_or_http_definition_policy`;
  - `definition_async_bit_changes_join_and_analysis_identity`;
  - `report_owner_chain_is_not_flattened`;
  - unsupported family-7/8 variants stay absent with scoped gap.
- [ ] Run mechanisms tests; expected RED.
- [ ] Implement mechanism registry then graph Phase B.
- [ ] Re-run expected GREEN.
- [ ] Commit: `feat: классифицировать typed discovery mechanisms`.

### Task 7.3: Staged BFS and repeated query semantics

- [ ] Add recording fake-port RED tests:
  - exact Stage 1 call order/once semantics;
  - initial Definition targets contain only exact methods;
  - roots/frontiers are canonical under provider permutations;
  - depth 1 queries root callers once and does not query depth-1 callers;
  - depth N reaches N calls; N+1 creates exact graph-depth gap;
  - new call targets are definition-checked before next frontier;
  - dynamic/unresolved/ambiguous calls never enter frontier;
  - shared caller is provider-queried once and cached outcome is replayed to a
    later origin;
  - an origin arriving at max depth cannot borrow a shorter origin's cached
    outgoing edge;
  - a cycle processes each `(method, origin, depth)` once;
  - BFS fixed point permits negative proof;
  - unavailable one batch does not stop unrelated canonical batches;
  - two unrelated proposals retain separate affects/verdicts.
- [ ] Run traversal/use-case tests; expected RED.
- [ ] Implement Stage 1-3 and scoped material propagation.
- [ ] Re-run expected GREEN.
- [ ] Commit: `feat: добавить bounded bfs project discovery`.

### Task 7.4: Support, maxEvidence/maxCandidates and status

- [ ] Add RED boundary tests:
  - support subjects use full ownership and 4096, not maxEvidence;
  - support N/N+1 creates DiscoveryTraversal gap, not ProviderGap;
  - global evidence displacement triggers a deterministic new support round
    for the replacement candidate and then reaches a stable prefix;
  - support rounds query no subject twice and 64/65 stops with the exact
    application round-limit gap rather than a fatal port error;
  - per-port/global evidence N/N+1 rebuild removes orphan edges;
  - per-port limit between CFE half polarity and its different-fact-tag whole
    companion drops the complete atomic group in both insertion orders;
  - global limit between EventSubscription descriptor and its derived uses set,
    and between Form polarity/contains/Action rows, retains all or none;
  - an oversized earlier atomic group prefix-stops before a smaller later group;
    both whole material scopes are gapped and no skip-and-continue occurs;
  - preliminary/final candidates N/N+1 retain the exact canonical prefix;
  - provider permutations and case-equivalent refs choose the same candidate
    via the exact-ref tie-break;
  - explicit proposal outside the preliminary prefix is still support-queried
    and fully validated;
  - candidate limit emits the one exact `DiscoveryTraversal/candidate_limit`
    tuple with positive preliminary/final counts;
  - candidate limit leaves proposal verdict/receipt probe unchanged;
  - maxCandidates never removes edges/related/evidence;
  - actionable candidate plus unrelated blocked branch is Partial, not
    Insufficient;
  - any Unknown selected proposal makes validate Insufficient/all-or-nothing
    ineligible.
- [ ] Implement Stage 4-5, final rebuild/check/status/eligibility logic.
- [ ] Re-run focused application tests expected GREEN.
- [ ] Commit: `feat: завершить material discovery conclusions`.

### Task 7.5: Real private composition and corpus GREEN

- [ ] Add the behavior evaluator over the already frozen corpus. Its first run
  must name all 48 IDs/both modes. If RED, failures may identify only missing
  Task 7 behavior and must be fixed without changing the seed; already-GREEN
  behavior is acceptable. Zero/ignored cases or generated expectations fail
  the gate.
- [ ] Add in-process smoke using accepted concrete Task 5/6 providers and an
  immutable fixture snapshot.
- [ ] Create `NativeDiscoveryProviders`; no interface registration.
- [ ] Run all 48 corpus cases; expected GREEN in explore and validate branches.
- [ ] Run provider permutation and two unrelated real-fixture scopes.
- [ ] Update active spec/historical plan/product contracts with Task 7 scoped
  invocation/traversal semantics in the same commit; do not defer the
  prerequisite Task 5 corrections to this step.
- [ ] Prove the seed was not changed:
  `git diff --exit-code "$CORPUS_SEED_COMMIT" -- tests/fixtures/project_discovery/fixture-cases.json tests/fixtures/project_discovery/corpus.json`.
- [ ] Commit: `feat: собрать project discovery mechanisms`.

## 14. Required acceptance matrix

1. All 48 fixed corpus IDs execute in both modes.
2. Eight positive primary and eight alternatives have exact typed base paths.
3. Eight contradicted cases use expressible negative proof, never proposal
   prose.
4. Eight unknown cases preserve the exact material source-scoped gap.
5. Eight lexical decoys create no typed runtime path.
6. Eight registered hard decoys do not become relevant/actionable by similar
   names.
7. Standalone calls/structural edges never seed reachability.
8. Family 7 requires uses + subscribes; family 8 requires specialized full
   owner/form/command/action chain.
9. BFS depth, root/frontier/node and candidate limits pass N/N+1.
10. Iterative support planning reaches a deterministic stable candidate prefix;
    subject/round bounds produce scoped application gaps and never an oversized
    or fatal SupportQueryPlan.
11. Provider/file/record/root permutations produce identical query sequence,
    execution snapshot, analysis ID, report and receipt eligibility.
12. Analysis and every paired destination are internal groups in one composite
    Metadata invocation, while FormInspection runs once for analysis only;
    Complete metadata in one group proves nothing in another and a local
    SourceSetWide gap does not poison siblings.
13. Shared callers are queried once, cached outcomes are replayed per origin,
    and a longer origin cannot exceed its own maxGraphDepth via a shorter one.
14. Per-port and global maxEvidence retain/drop complete
    `SemanticAtomicGroupIdV1` clusters; no CFE half, Event descriptor/uses set,
    or Form catalog is split by fact tag or invocation reuse.
15. Complete empty scope proves only its exact query; bounded/unavailable/
    failed never becomes absence.
16. One degraded proposal scope does not contaminate an unrelated supported
    proposal, while receipt stays all-or-nothing.
17. Later-orphaned invocation issues stay visible but have no final proposal or
    candidate affects.
18. EDT invokes zero evidence providers.
19. FormCommand/ReportProcessorForm and HttpRoute roots require retained exact
    Task 5B policy Yes outcomes; Event/Scheduled/callback roots retain their
    synchronous policy, and `DefinitionShape.is_async` changes analysis identity.
20. Complete ScheduledJob Use=false is exact `scheduled_job_disabled` No;
    it survives non-activation gaps and schedules no Definition solely for the
    job, while missing/malformed Use and nonpredefined runtime state remain
    Unknown.
21. No public MCP/package/skill surface changes.

## 15. Final verification

- [ ] Run:

```text
python3 -m unittest tests.ci.test_project_discovery_corpus -v
cargo test --locked -p unica-coder application::discovery::corpus -- --nocapture
cargo test --locked -p unica-coder application::discovery::mechanisms -- --nocapture
cargo test --locked -p unica-coder application::discovery::traversal -- --nocapture
cargo test --locked -p unica-coder application::discovery::use_case -- --nocapture
cargo test --locked -p unica-coder infrastructure::discovery -- --nocapture
cargo test --locked -p unica-coder application::discovery -- --nocapture
cargo test --locked -p unica-coder
cargo fmt --all -- --check
cargo clippy --locked -p unica-coder --all-targets -- -D warnings
python3 tests/ci/test_product_contracts.py
git diff --check
```

- [ ] Verify the corpus test output names exactly 48 unique IDs and both modes.
- [ ] Verify no `workspace_index`, `rusqlite`, `result_text`, `mcp_tool_text`
  or public discovery registration enters the new modules.
- [ ] Review every scoped issue against two unrelated proposals/source sets.
- [ ] Review analysis-ID fixtures after the contract version bump.

## 16. Hard STOP conditions

Stop implementation and show the owner when any condition is true:

- active spec/model still permits `ExchangePlan + handles`;
- Task 5 shared catalog/providers/support or Task 6 query-specific snapshot BSL
  providers are not GREEN;
- 48-case corpus seed/schema/fixture references are absent or written after
  mechanisms output exists;
- corpus contradicted expectations require parsing `Proposal.intent`;
- repeated Definition/Call invocations lose query scope or are flattened to one
  lossy port outcome;
- QueryWide gap from one frontier blocks unrelated proposal scopes;
- Complete analysis metadata is reused as destination absence proof,
  FormInspection runs for a destination in v1, or destination metadata creates
  analysis mechanisms;
- an orphaned/displaced branch's invocation issue retains final proposal or
  candidate affects without another retained proof dependency;
- application candidate/root/frontier/support truncation is emitted as a
  provider gap;
- per-port/global maxEvidence sorts or drops individual records and can split a
  closed `SemanticAtomicGroupIdV1` cluster;
- support semantic scope is narrowed by maxEvidence instead of 4096 safety
  bound;
- SupportQueryPlan receives more than 4096 subjects, the same subject is
  re-queried, or support planning exceeds 64 rounds without the typed
  application gap;
- standalone call, structural edge, disabled job or partial exchange chain
  becomes runtime-connected;
- FormCommand/HTTP route becomes runtime-connected from raw Definition
  kind/arity/context instead of an exact versioned Task 5B policy Yes, or any
  compatibility join discards `DefinitionShape.is_async`;
- any candidate becomes Actionable without a directed typed base path, exact
  existence/ownership and known support;
- maxGraphDepth is consumed by base binding edges or silently treated as
  complete when a nonempty frontier remains;
- a later origin re-invokes an already cached caller, or borrows cached outgoing
  calls beyond that origin's own maxGraphDepth;
- maxCandidates affects traversal, evidence, flow edges, proposal verdict or
  receipt eligibility;
- negative runtime proof is emitted before complete runtime roots reach a true
  fixed point;
- final report references evidence removed by per-port/global limit;
- proposal/candidate materiality drops source-set identity;
- family 7 direct callbacks/BSP exchange or family 8 BSP/print conventions are
  inferred lexically;
- EDT runs any evidence provider;
- public MCP/tool/package/skill registration is modified in Task 7;
- analysis semantics change without the analysis/mechanism/traversal/atomic-
  grouping version being encoded in analysis ID.

## 17. Design result

Task 7 is implementable only after the gates in section 1. Its correctness
boundary is application orchestration: providers return exact facts for exact
queries, while Task 7 owns roots, traversal, relevance, mechanism
classification, materiality and output limits. This separation is required for
receipt-grade conclusions and for later Task 8-10 resolver/receipt/guard work.
