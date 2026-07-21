# Issue #5 Mandatory Extension-Point Discovery Design

- Date: 2026-07-21
- Product epic: [IngvarConsulting/unica#5](https://github.com/IngvarConsulting/unica/issues/5)
- Typed implementation: [IngvarConsulting/unica#161](https://github.com/IngvarConsulting/unica/issues/161)
- Historical prototypes: PR #83 and PR #117; neither is a merge base
- Status: approved direction; this document freezes the delivery split before implementation planning

## Problem

Unica exposes enough low-level metadata, form, and BSL tools to investigate a
typical 1C mechanism manually, but it does not guarantee that the investigation
happens before planning or mutation. An agent can therefore select an obvious
metadata object while missing the actual user flow, a separate tabular section,
a managed form binding, support state, or a standard data processor.

Issue #5 is the product outcome: before changing an existing typical
configuration, Unica must perform an evidence-backed discovery preflight and
make the selected extension point explicit. Issue #161 is the typed Rust/MCP
implementation stream. Closing PR #83 as superseded did not close the product
outcome.

## Approaches Considered

### 1. Revive PR #83

Rejected. The prototype proved the UT 11.5 scenario, but mixed related
artifacts, runtime flow, and actionable extension points; used one scalar score
for unrelated meanings; embedded domain-specific heuristics; parsed adapter
display output across layer boundaries; and ended with unresolved filesystem
security work.

### 2. Continue PR #117 as one delivery

Rejected as a merge strategy. PR #117 contains useful typed models, graph and
snapshot research, but combines production foundations with a very large
investigation archive and unfinished receipt/guard work. It is a source of
reviewed ideas and focused tests, not a branch to merge wholesale.

### 3. Narrow typed restart with independent delivery slices

Selected. Start from current `main`, selectively reuse proven ideas after fresh
review, and deliver the user-visible read-only preflight before designing
mutation authorization. Each slice has one public responsibility and its own
acceptance gate.

## Issue and Delivery Structure

Issue #5 remains the parent product epic. Issue #161 owns the typed discovery
pipeline. Delivery is split as follows:

1. **Slice A — read-only discovery MCP.** Typed request/result, evidence ports,
   evidence graph, bounded analysis snapshot, task-only exploration, public
   `unica.project.discover`, and package smoke coverage.
2. **Slice B — mandatory preflight skill.** A dedicated prompt-visible skill
   triggers before planning or mutation of a typical configuration, calls the
   public tool, resolves material missing checks through public `unica.*`
   tools, and records the selected and rejected extension points. Slice B plus
   Slice A closes the original product acceptance of issue #5.
3. **Slice C — proposal validation.** Typed `supported`, `contradicted`, or
   `unknown` verdicts for explicit proposals. This extends the public request
   without breaking Slice A clients.
4. **Slice D — receipts and mutation guards.** Durable receipts, leases,
   mutation-scope binding, stale-source rejection, observation, and rollout.
   This is a separate safety project and does not block the read-only product
   value of issue #5.

Slices A and B may be separate pull requests, but they share one product
acceptance fixture. Slice C and Slice D must not be pulled into the first MCP
implementation opportunistically.

## Slice A Public Contract

The only public server remains `unica`. The new read-only tool is
`unica.project.discover`; its operation descriptor is non-mutating and declares
read-only cache access. Slice A records its decision as ADR-0012 because
ADR-0010 and ADR-0011 already exist on current `main`.

### Request

The request is a strict object with unknown fields rejected:

```json
{
  "cwd": "/workspace",
  "mode": "explore",
  "task": "При поступлении товаров контролировать остаточный срок годности серий",
  "sourceDir": "src/configuration",
  "concepts": ["серии", "срок годности"],
  "searchTerms": ["ПодборСерийВДокументы"],
  "objects": ["Document.ПриобретениеТоваровУслуг"],
  "limits": {
    "maxFiles": 20000,
    "maxBytes": 268435456,
    "maxEvidence": 2000,
    "maxCandidates": 100,
    "maxGraphDepth": 12
  }
}
```

Required fields are `mode=explore` and non-empty `task`. `cwd` follows the
existing common MCP workspace contract. `sourceDir`, `concepts`,
`searchTerms`, `objects`, and `limits` are optional.

A call containing only `cwd`, `mode`, and `task` is a supported first-class
path. The application derives deterministic lexical concepts from the task and
also passes the full task through typed search providers. Explicit concepts and
search terms augment rather than replace task-derived exploration. The core
contains no domain dictionary for series, shelf life, documents, or other UT
11.5 concepts.

`sourceDir` follows existing analysis-tool semantics: it is a contained path
relative to the workspace. When omitted, the existing project-source resolver
must select exactly one eligible configuration source root or return a typed
ambiguity error. Runtime `sourceSet` names are not accepted as a second,
competing selector in Slice A.

`objects` contains canonical 1C references and is parsed into typed artifact
identities. `proposedExtensionPoints`, receipts, `dryRun`, `confirm`, raw
adapter arguments, and arbitrary paths are not accepted in Slice A. Later
optional fields may be added compatibly in Slice C.

Resource limits are validated once into bounded newtypes. JSON Schema and Rust
validation use the same units; text bounds are expressed as UTF-8 bytes and
must not be advertised as JSON Schema character `maxLength` limits unless the
runtime checks characters too. Slice A fixes these input bounds:

- `task`: 1..=8192 UTF-8 bytes;
- `concepts`: at most 64 unique entries, each 1..=256 UTF-8 bytes;
- `searchTerms`: at most 128 unique entries, each 1..=256 UTF-8 bytes;
- `objects`: at most 128 unique entries, each 1..=1024 UTF-8 bytes;
- `maxFiles`: default and maximum 20,000;
- `maxBytes`: default and maximum 268,435,456;
- `maxEvidence`: default and maximum 2,000;
- `maxCandidates`: default and maximum 100;
- `maxGraphDepth`: default and maximum 12.

Callers may lower resource limits but cannot raise them above these package
bounds. Schema descriptions state byte limits in prose; runtime validation is
authoritative for UTF-8 byte counts.

### Result

The typed payload is returned in `OperationResult.data.discovery` and contains:

- `schemaVersion` and terminal `status` (`complete` or `partial`);
- selected source root and an analysis-snapshot fingerprint;
- normalized concepts with provenance (`taskDerived` or `explicit`);
- provider outcomes and coverage;
- related artifacts;
- structural edges separated from proven runtime-flow edges;
- actionable extension-point candidates;
- warnings and missing checks;
- stable evidence identifiers and source locations.

There is no aggregate score or confidence scalar. A consumer judges a finding
from typed evidence, provider outcome, coverage, freshness, and explicit links.
The result never contains a receipt in Slice A.

## Typed Application Architecture

`DiscoverExtensionPointsUseCase` owns orchestration and consumes only typed
ports:

- metadata catalog and object structure;
- managed form bindings;
- BSL lexical/index search;
- definition resolution;
- call graph/runtime-flow evidence;
- support state.

Infrastructure providers collect facts. They do not call one another, parse
another adapter's display text, rank architectural choices, or return
presentation JSON to be reparsed by the application layer.

Every provider returns an exhaustive `ProviderOutcome<T>`:

- `Complete` — complete for the exact typed query;
- `Bounded` — a resource limit truncated coverage;
- `Unavailable` — the capability is not available in this workspace;
- `Failed` — the provider failed with a typed diagnostic;
- `ContractViolation` — returned data violated the port contract and is not
  eligible for graph promotion.

Only an empty `Complete` result can be negative evidence for its exact query.
`Bounded`, `Unavailable`, and `Failed` produce partial output and missing
checks, never a false negative. `ContractViolation` also makes the report
partial, adds a blocking diagnostic for that provider, and excludes all of its
records from graph promotion. Invalid requests, ambiguous/escaping source
selection, and inability to establish a contained source root fail the whole
operation before providers run.

The graph keeps `contains` and `defines` structural. Only compatible typed
platform callbacks, form command bindings, event subscriptions, or call-graph
facts can create runtime-flow edges. A lexical match may create a related
artifact but cannot create an actionable candidate by itself.

## Analysis Snapshot and Filesystem Safety

Slice A uses an **analysis snapshot**, not a mutation receipt. It records the
resolved mapping identity and the exact raw-byte hashes of every file whose
contents contribute evidence. If enumeration or reading reaches a bound, the
provider and overall report become partial.

All reads are contained beneath the selected source root, bounded by file and
byte limits, and performed through verified regular-file handles. Escaping
symlinks/reparse points, non-regular files, path swaps, and identity changes
between capture and verified read fail closed for the affected provider.
Platform-specific filesystem code remains behind the existing infrastructure
platform facade.

BOM and EOL bytes participate in content hashes. Platform-generated
`ConfigDumpInfo.xml` is excluded from source evidence. The analysis snapshot
does not authorize later mutation and makes no whole-workspace immutability
claim beyond its manifest and recorded coverage.

## Error Handling and Rust Refactoring Rules

- External request combinations deserialize into strict typed structures with
  exhaustive enum handling.
- `DiscoveryMode`, artifact kinds, provider outcomes, report status, evidence
  relation kinds, and concept provenance are enums rather than strings or
  boolean policy flags.
- Artifact IDs, evidence IDs, and bounded resource values use dedicated
  newtypes where mixing primitives could create an invalid graph or limit.
- Production paths propagate `Result` with typed discovery errors and do not
  use `unwrap`, `expect`, or `panic` for recoverable workspace/provider input.
- Functions borrow `&str`, `&Path`, and slices where ownership is unnecessary.
- Constructors with multiple optional policy inputs use a request/options
  struct or builder rather than positional parameters.
- Enum matches name every variant. Catch-all branches are reserved for open
  external value domains, not internal enums.
- Request parsing happens once; the typed request is passed to the use case
  rather than validating and reparsing the same JSON at another layer.

Refactoring is limited to boundaries needed by discovery. Existing public
tools and unrelated adapters are not reorganized as part of Slice A.

## Slice B Mandatory Skill

Add a dedicated `extension-point-discovery` skill whose trigger covers planning
or implementing changes to existing typical configurations, supported objects,
CFE interception, forms, documents, processors, handlers, and tabular sections.

The skill must:

1. call `unica.project.discover` before planning and before any mutating MCP or
   manual XML/BSL edit;
2. pass the original task and confirmed known objects, and optionally enrich
   concepts/search terms without inventing metadata names;
3. inspect candidates, evidence, warnings, provider outcomes, and missing
   checks;
4. close material missing checks only through public `unica.*` tools;
5. stop before mutation when an unresolved check could change the selected
   architecture;
6. record the selected point, rejected alternatives, supporting evidence,
   support/lock state, and unresolved non-material checks.

The skill does not call internal RLM binaries, SQLite, analyzers, or packaged
scripts directly. It does not treat discovery as permission to mutate.

## Testing Strategy

Development follows red/green/refactor in small commits.

### Contract tests

- task-only request is accepted;
- unknown fields, receipts, proposals, `dryRun`, `confirm`, absolute or escaping
  `sourceDir`, duplicate values, and out-of-range limits are rejected;
- JSON Schema validation and Rust deserialization accept and reject the same
  representative payloads, including Cyrillic boundary cases;
- public MCP registration exposes exactly one `unica.project.discover` tool.

### Domain and application tests

- every internal enum is handled exhaustively;
- structural evidence cannot promote itself to runtime flow;
- provider outcome combinations produce the correct complete/partial status;
- deterministic input produces byte-identical normalized output and digest;
- task-derived and explicit concepts retain separate provenance.

### Filesystem and provider tests

- contained reads, symlink/reparse rejection, non-regular targets, raw-byte
  hashing, BOM/EOL changes, bounds, file replacement, and partial-provider
  behavior;
- Platform XML, form binding, lexical BSL, definition, graph, and support-state
  providers return typed records rather than parsed display text.

### Product acceptance

The domain-neutral UT 11.5 fixture is data, not production heuristics. A
task-only request about shelf-life control must find:

- `Document.ПриобретениеТоваровУслуг.TabularSection.Серии`;
- `DataProcessor.ПодборСерийВДокументы`;
- the registration/selection form involved in the user flow;
- a warning/check showing that an implementation limited to `Товары.Серия`
  has insufficient evidence or coverage.

The same fixture must remain useful when BSL indexing is unavailable: metadata
and form evidence are returned, status is partial, and the missing BSL checks
are explicit.

### Package acceptance

- source and packaged MCP smoke tests cover the tool;
- the dedicated skill passes the official validator, provenance, routing, and
  task-trigger contract tests;
- prompt examples route through native `unica.project.discover` with no script
  fallback;
- Rust format, Clippy with warnings denied, Rust tests, Python CI, package
  validation, and platform-boundary checks pass.

## Completion and Issue Semantics

Slice A pull requests use `Refs #5` and `Refs #161`. Slice B may close #5 only
after the packaged task-only UT 11.5 scenario and mandatory skill gate both
pass. Issue #161 remains open for proposal validation and, if retained in its
scope, the separately accepted receipt/guard work.

No pull request may claim that discovery authorizes mutation, proves runtime
flow from lexical evidence, or provides freshness beyond the reported analysis
snapshot.
