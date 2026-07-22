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
the full task to typed search providers. Task-derived concepts are domain-neutral and use no business-domain dictionary. Explicit concepts and search terms
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

Every rejected request crosses the public tool boundary as
`discovery_request_<reason>: <detail>`, with a distinct stable reason for
unknown or missing fields, invalid types or mode, text/item/duplicate failures,
invalid artifact/source identities, and out-of-range limits. Runtime discovery
errors keep their separate `discovery_<reason>` codes.

## Result, providers, and graph

The typed result is at `OperationResult.data.discovery`: `schemaVersion`,
terminal `status` (`complete` or `partial`), selected source root,
analysis-snapshot fingerprint, normalized concepts with `taskDerived` or
`explicit` provenance, provider outcomes and coverage, related artifacts,
structural and proven runtime-flow edges, actionable extension-point candidates,
each with a deterministic advisory `recommendation { summary, basis }`,
warnings, missing checks, and stable evidence IDs/source locations. A basis can
come only from accepted metadata structure, managed-form binding, or proven
runtime-flow facts; lexical/support evidence cannot create one. Recommendations
do not rank a winner or override blocking/material gaps. The result has no
aggregate score, confidence scalar, proposal verdict, or receipt.

`DiscoverExtensionPointsUseCase` owns orchestration through typed ports for
metadata/object structure, managed-form bindings, BSL lexical/index search,
definition resolution, call-graph/runtime-flow evidence, and support state.
Providers only collect facts; they do not call each other, parse other adapter
display text, rank architecture, or return presentation JSON for reparsing.

Every provider/query returns exactly one exhaustive `ProviderOutcome<T>` fact:

- `Complete`: complete for the exact typed query; only an empty result is
  negative evidence for that query.
- `Bounded`: the returned data is sound but scope is incomplete because a
  resource, trust, or snapshot-freshness boundary prevents a complete result.
- `Unavailable`: the capability is unavailable in this workspace.
- `Failed`: the provider failed with a typed diagnostic.
- `ContractViolation`: returned data violates its port contract and is ineligible
  for graph promotion.

`Bounded`, `Unavailable`, and `Failed` produce partial output and missing
checks, never a false negative. `ContractViolation` also makes the output
partial, adds a blocking provider diagnostic, and excludes its records from
graph promotion.

The application reserves a deterministic share of the remaining global
`maxEvidence` budget for each later provider. Providers retain strong exact or
inflected task matches and their structural ancestors ahead of weak prefixes
and unrelated facts. `maxCandidates` uses the same private match-strength tiers
with canonical identity as the tie-breaker; no score or confidence crosses the
public boundary. Compound platform identifiers are segmented from their
display spelling before case normalization, and three-character acronyms are
not treated as near-inflections. All typed relevance matching in one request
shares a cancellation-aware work budget of
`clamp(limits.maxEvidence * 4096, 8192, 8388608)` units; each term comparison
costs 32 units plus both normalized byte lengths. Exhaustion preserves accepted
facts but makes the affected provider `Bounded(discovery_match_work_bound)`.

Metadata-catalog facts are fail-closed `Contains` hierarchy claims over only
metadata objects, tabular sections, attributes, forms, and commands. Each
artifact has one typed parent, cannot parent itself, and participates in no
cycle. A normal child must be a canonical direct descendant of its parent;
the sole non-prefix root transition is a registered metadata object beneath a
top-level `Configuration.*` artifact.

The BSL lexical parser stores method ownership as sparse declaration-to-end
ranges rather than one entry per source line. It rejects a line immediately at
64 KiB plus one byte, bounds signatures to 64 lines and 64 KiB, and reports
those resource limits as `Bounded` while preserving prior fully analyzed file
coverage. Lexical facts are retained in canonical order with at most
`maxEvidence + 1` distinct matches before the final deterministic truncation.
BSL files are scanned in canonical path order under one cumulative lexical-work
budget of `min(limits.maxBytes * 16, 536870912)` units. Every normalized-line
comparison and matching-column comparison is charged its haystack and pattern
byte lengths plus a 256-unit dispatch floor; the floor prevents empty lines or
thousands of short task-derived terms from bypassing the bound. Exhaustion is
`Bounded` with `bsl_lexical_work_bound` and retains only facts and coverage from
files that completed before the exhausted file.

Existing-index definition lookup reads its status through a contained 64 KiB
regular-file boundary, accepts only a canonical non-link SQLite database below
the cache root, copies a stable verified-handle image within `maxBytes`, rejects
WAL-mode/live-sidecar images, and deserializes the copy into one private
read-only connection. Exact queries share one cumulative SQLite VM-work budget;
TEXT fields are byte-bounded, and query-term truncation at 128 identities is
reported explicitly as `Bounded`. Each captured source file is parsed at most
once. Individually source-validated hits are returned as data-bearing
`Bounded(bsl_definition_freshness_unverified)` because the legacy index status
does not bind its generation to the analysis snapshot. An empty such lookup is
also `Bounded`, never negative evidence; missing or stale indexes are
`Unavailable`. Status publishing uses same-directory atomic replacement;
transient/incomplete status I/O is `Unavailable`, while structurally invalid
status and malformed schema/rows fail closed as `ContractViolation`.

The graph keeps `contains`, `defines`, and data bindings structural. Compatible
typed form actions/callbacks establish runtime roots; only directed
`Method -> Method` `Calls` facts extend those roots. `maxGraphDepth` counts only
those call hops: the node at depth N remains, its outgoing calls are omitted,
and RuntimeFlow becomes `Bounded(graph_depth_limit)`. Structural edges do not
consume depth, `maxCandidates` does not affect traversal, standalone calls do
not establish candidates, and event subscriptions fail closed until Slice A
has a typed source shape. A lexical match can create a related artifact but
cannot by itself create an actionable candidate.

## Analysis snapshot boundary

Slice A captures an **analysis snapshot**, not a mutation receipt. It records
the resolved mapping identity and raw SHA-256 hashes of evidence-contributing files only. BOM and EOL bytes participate. The snapshot excludes only actual runtime-sidecar `ConfigDumpInfo.xml` content, as identified by `config_dump_info_xml_kind(bytes)`; legitimate external metadata with that filename remains evidence. A bounded enumeration or read makes the provider and result partial.

Implicit source selection reads exactly `v8project.yaml` beneath the canonical
workspace through a verified regular-file handle with no link following, a
fixed 1 MiB byte ceiling, and cancellation polling around 64 KiB chunks. This
manifest is selection input, not evidence: its bytes do not consume request
`maxBytes` and are not included among analysis-snapshot contributor hashes.

Reads stay beneath the selected source root through verified regular-file
handles. Escaping symlinks/reparse points, non-regular files, path swaps, and
identity changes between capture and verified read fail closed for the affected
provider. Verified file bodies are read in 64 KiB chunks with cancellation
polls around I/O and incremental SHA-256 updates. Each XML, BSL, or support-state
evidence file has a 16 MiB transparent-read limit in addition to request
`maxBytes`; reaching either byte limit returns a stable `Bounded` inventory that
preserves its fully verified prefix and is never reported as malformed provider
data or a contract violation.

Inventory enumeration also has a cumulative traversal-entry budget of
`1024 + 8 * limits.maxFiles` (at most 161,024 for Slice A). Every verified child
entry consumes it, including irrelevant files and directories, so neither flat
fanout nor deep nesting can grow the pending set outside the request-derived
ceiling. The first N+1 child returns `Bounded` with
`source_inventory_traversal_bound`, preserves the fully verified evidence
prefix, and remains subordinate to cancellation and no-follow validation. A
typed `SourceInventoryBound::TraversalEntries` marker substantiates this work
bound across the application contract without falsely adding irrelevant
entries to evidence-only `filesSeen`; downstream providers inherit that marker
as incomplete inventory scope and cannot report a false `Complete` result.
Ordinary file/per-file/aggregate-byte bounds are accepted only when
`filesSeen` records the exact N+1 eligible-file probe; merely returning exactly
`maxFiles` or `maxBytes` does not prove that more source exists.

The snapshot neither claims whole-workspace immutability nor grants permission
to mutate. Platform filesystem code remains behind the existing infrastructure
facade.

## Slice B gate and package acceptance

The prompt-visible `extension-point-discovery` skill runs before planning or
changing typical configurations, supported objects, CFE interception, forms,
documents, processors, handlers, or tabular sections. It calls
`unica.project.discover` before planning and any mutating MCP/manual XML or BSL
edit; passes the original task and confirmed objects; and may enrich terms
without inventing metadata names.

It inspects candidate recommendations, evidence, warnings, outcomes, and missing checks;
resolves material checks only with public `unica.*` tools; and stops before
mutation if an unresolved check can change the selected architecture. It records
the selected and rejected points, copied recommendation, evidence, support/lock state, and unresolved
non-material checks. It does not invoke internal RLM binaries, SQLite,
analyzers, or packaged scripts directly.

Package acceptance covers source and packaged MCP smoke, official skill
validation/provenance/routing/task-trigger checks, and native tool prompt
examples with no script fallback. The domain-neutral packaged UT 11.5 task-only
fixture must find `Document.ПриобретениеТоваровУслуг.TabularSection.Серии`,
`DataProcessor.ПодборСерийВДокументы`, and
`DataProcessor.ПодборСерийВДокументы.Form.РегистрацияИПодборСерийПоОднойСтрокеТоваров`; it
must warn when `Товары.Серия` alone has insufficient evidence or coverage. If
BSL indexing is unavailable, metadata and form evidence remain, status is
partial, and missing BSL checks are explicit.

Slices A and B together meet issue #5 product acceptance. Slice C adds typed
proposal verdicts (`supported`, `contradicted`, or `unknown`) compatibly. Slice
D covers durable receipts, leases, mutation-scope binding, stale-source
rejection, observation, and rollout, and requires its own accepted decision.
