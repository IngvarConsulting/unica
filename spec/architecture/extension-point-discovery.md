# Read-only extension-point discovery

This active architecture contract defines Slice A and the mandatory Slice B
preflight for issue #5. `unica.project.discover` is a non-mutating operation on
the one public MCP server, `unica`, and declares read-only cache access.

## Slice A request

The request is a strict object: unknown fields are rejected. `cwd` follows the
common workspace contract; `mode` is required and must equal `explore`; `task`
is required and non-empty. `sourceDir`, `concepts`, `searchTerms`, `objects`,
and `limits` are optional. `cwd`, `mode`, and `task` alone are a supported
first-class request.

The application derives deterministic lexical concepts from `task` and passes
the full task to typed search providers. Explicit concepts and search terms
augment, not replace, that exploration. `objects` holds canonical 1C
references parsed as typed artifact identities. Slice A rejects
`proposedExtensionPoints`, receipts, `dryRun`, `confirm`, raw adapter
arguments, arbitrary paths, and runtime `sourceSet` selectors.

`sourceDir` is a contained workspace-relative path. If absent, the
project-source resolver selects exactly one eligible configuration source root
or returns a typed ambiguity error. Invalid requests, ambiguous or escaping
source selection, and inability to establish a contained root fail before
providers run.

All text bounds below are UTF-8 byte limits. Runtime validation is authoritative
and validates once into bounded newtypes; JSON Schema must describe bytes in
prose, not character `maxLength`. Entries are unique.

| Field | Exact Slice A bound |
| --- | --- |
| `task` | 1..=8192 UTF-8 bytes |
| `concepts` | at most 64 entries; each 1..=256 UTF-8 bytes |
| `searchTerms` | at most 128 entries; each 1..=256 UTF-8 bytes |
| `objects` | at most 128 entries; each 1..=1024 UTF-8 bytes |
| `limits.maxFiles` | default and maximum 20,000 |
| `limits.maxBytes` | default and maximum 268,435,456 |
| `limits.maxEvidence` | default and maximum 2,000 |
| `limits.maxCandidates` | default and maximum 100 |
| `limits.maxGraphDepth` | default and maximum 12 |

Callers may lower limits but never raise them above these values.

## Result, providers, and graph

The typed result is at `OperationResult.data.discovery`: `schemaVersion`,
terminal `status` (`complete` or `partial`), selected source root,
analysis-snapshot fingerprint, normalized concepts with `taskDerived` or
`explicit` provenance, provider outcomes and coverage, related artifacts,
structural and proven runtime-flow edges, actionable extension-point candidates,
warnings, missing checks, and stable evidence IDs/source locations. It has no
aggregate score, confidence scalar, or receipt.

`DiscoverExtensionPointsUseCase` owns orchestration through typed ports for
metadata/object structure, managed-form bindings, BSL lexical/index search,
definition resolution, call-graph/runtime-flow evidence, and support state.
Providers only collect facts; they do not call each other, parse other adapter
display text, rank architecture, or return presentation JSON for reparsing.

Each provider returns exhaustive `ProviderOutcome<T>` facts:

- `Complete`: complete for the exact typed query; only an empty result is
  negative evidence for that query.
- `Bounded`: a resource limit truncated coverage.
- `Unavailable`: the capability is unavailable in this workspace.
- `Failed`: the provider failed with a typed diagnostic.
- `ContractViolation`: returned data violates its port contract and is ineligible
  for graph promotion.

`Bounded`, `Unavailable`, and `Failed` produce partial output and missing
checks, never a false negative. `ContractViolation` also makes the output
partial, adds a blocking provider diagnostic, and excludes its records from
graph promotion.

The graph keeps `contains` and `defines` structural. Only compatible typed
platform callbacks, form-command bindings, event subscriptions, and call-graph
facts promote evidence to runtime-flow edges. A lexical match can create a
related artifact but cannot by itself create an actionable candidate.

## Analysis snapshot boundary

Slice A captures an **analysis snapshot**, not a mutation receipt. It records
the resolved mapping identity and exact raw-byte hashes of every file that
contributes evidence. BOM and EOL bytes participate; generated
`ConfigDumpInfo.xml` is excluded. A bounded enumeration or read makes the
provider and result partial.

Reads stay beneath the selected source root through verified regular-file
handles. Escaping symlinks/reparse points, non-regular files, path swaps, and
identity changes between capture and verified read fail closed for the affected
provider. The snapshot neither claims whole-workspace immutability nor grants
permission to mutate. Platform filesystem code remains behind the existing
infrastructure facade.

## Slice B gate and package acceptance

The prompt-visible `extension-point-discovery` skill runs before planning or
changing typical configurations, supported objects, CFE interception, forms,
documents, processors, handlers, or tabular sections. It calls
`unica.project.discover` before planning and any mutating MCP/manual XML or BSL
edit; passes the original task and confirmed objects; and may enrich terms
without inventing metadata names.

It inspects candidates, evidence, warnings, outcomes, and missing checks;
resolves material checks only with public `unica.*` tools; and stops before
mutation if an unresolved check can change the selected architecture. It records
the selected and rejected points, evidence, support/lock state, and unresolved
non-material checks. It does not invoke internal RLM binaries, SQLite,
analyzers, or packaged scripts directly.

Package acceptance covers source and packaged MCP smoke, official skill
validation/provenance/routing/task-trigger checks, and native tool prompt
examples with no script fallback. The domain-neutral packaged UT 11.5 task-only
fixture must find `Document.ПриобретениеТоваровУслуг.TabularSection.Серии`,
`DataProcessor.ПодборСерийВДокументы`, and the registration/selection form; it
must warn when `Товары.Серия` alone has insufficient evidence or coverage. If
BSL indexing is unavailable, metadata and form evidence remain, status is
partial, and missing BSL checks are explicit.

Slices A and B together meet issue #5 product acceptance. Slice C adds typed
proposal verdicts (`supported`, `contradicted`, or `unknown`) compatibly. Slice
D covers durable receipts, leases, mutation-scope binding, stale-source
rejection, observation, and rollout, and requires its own accepted decision.
