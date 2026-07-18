# ADR-0008: Project Discovery and Discovery Receipts

- Status: accepted
- Date: 2026-07-17
- Supersedes: unmerged implementation from PR #83

## Context

Unica can validate and apply a technically correct change while the caller has
selected the wrong 1C application mechanism or extension point. Prompt guidance
cannot prove that a proposed target exists, is reachable from the relevant
runtime flow, is supported for mutation, or still belongs to the source state
that was investigated.

PR #83 established the useful product direction, but its implementation mixed
related artifacts with extension points, collapsed unlike facts into a scalar
score, embedded domain-specific vocabulary, parsed display output, read adapter
storage directly, and could not safely bind investigation to an applied
mutation. It is superseded rather than repaired in place.

## Decision

### Typed discovery boundary

The single public MCP server `unica` exposes one two-mode tool,
`unica.project.discover`:

- `explore` returns related artifacts, typed runtime-flow edges, and actionable
  candidates but never a receipt;
- `validate` reevaluates explicit proposals against the current source state and
  may issue a receipt only for fully supported, unambiguous proposals without a
  material blocker.

`DiscoverExtensionPointsUseCase` owns orchestration and consumes six typed
evidence ports: `MetadataCatalogPort`, `CodeSearchPort`, `DefinitionPort`,
`CallGraphPort`, `FormInspectionPort`, and `SupportStatePort`. Infrastructure
providers return typed facts, coverage, freshness, canonical identities, and
provenance. The use case does not parse human-readable adapter output, read an
adapter's SQLite store, perform an unbounded fallback scan, or add
domain-specific synonyms.

Proposal verdicts are exactly `supported`, `contradicted`, or `unknown`.
Absence is contradictory only when the relevant provider has complete, fresh
coverage. Evidence levels and material checks remain separate; there is no
public scalar score or confidence.

### Source snapshots and atomic grants

Each analysis and mutation source-set has a content fingerprint derived from a
domain-separated canonical manifest containing mapping identity, sorted
contained paths, and file-content SHA-256 digests. A composite fingerprint
covers the analysis source-set and every destination source-set. The existing
`workspaceEpoch` is diagnostic-only and is not stale-state authority.

A successful validation stores a server-owned rolling discovery receipt. Every
authorization unit is an atomic grant that binds all of the following without
cross-product expansion:

- public tool and canonical target;
- mutation class and change kind;
- normalized output-affecting parameters;
- destination source-set identity;
- exact allowed artifacts;
- task/evidence digests and the analysis/source snapshot baseline.

The receipt is evidence that the architecture was investigated. It is not user
authorization and does not replace the independent `dryRun: false` requirement.
Version 1 has no receipt TTL; schema compatibility, revision, revocation, scope,
and content fingerprints determine validity.

### Lease, mutation, and rolling revision

For an applied enforceable mutation that presents a valid receipt, the
application validates it and acquires an exclusive receipt lease before
invoking the handler. It keeps the lease through handler execution, typed-effect
and before/after-manifest checks, and the atomic post-mutation receipt update. A
compare-and-swap performed only after the handler is insufficient because two
callers could already have mutated the same revision. An `observe` or `warn`
call allowed without a receipt has no receipt to lease or advance.

The post-handler order is fixed: handler invocation; capture typed mutation
effects; capture the post-mutation source snapshot; advance or revoke the
presented receipt while the same exclusive receipt lease remains held; release
the current receipt lease; then perform domain event emission, cache
invalidation/reporting, other-receipt reconciliation, workspace-service
invalidation, shadow observation append, and result construction. This keeps
the authoritative receipt transition inside its lease while ensuring no second
receipt lock is acquired under the current lease.

Dry-run never acquires or advances a receipt. A successful in-scope mutation
advances its revision and fingerprints. A failed handler advances nothing only
when the manifest proves that no source changed; partial writes revoke the
receipt. An out-of-scope effect revokes it. Other receipts linked to affected
source-sets are reconciled after releasing the current lease so lock ordering
cannot deadlock.

### Guard and rollout

Operation descriptors classify discovery independently as `not_required`,
`advisory_only`, or `enforceable`. The first enforceable resolver is
`unica.cfe.patch_method`; it resolves the exact method, interceptor type,
execution context, method kind, destination extension, and one module artifact.
Broad edit operations remain advisory until their resolvers produce equally
exact target scopes.

The application evaluates argument/path/source-set validity, then the support
guard, then the discovery guard, then the handler. Guard modes are server or
workspace configuration: `off`, `observe`, `warn`, and selective `deny`.
Dry-run is always available without a receipt, and no per-call bypass is
exposed. Package defaults remain `observe`; promotion requires the gold-corpus
and real shadow thresholds from the accepted design.

### Shadow observations

After the handler outcome is known, guard evaluation may append a versioned
shadow observation. This journal is non-authoritative: recording failure is an
operator diagnostic and never changes the handler outcome, receipt decision,
or rollout mode.

The workspace-keyed store is an OS-locked, schema-versioned JSONL journal with
bounded rollover and aggregate counters. Records contain stable names, reason
codes, revisions, timing/sequence metadata, and hashes of resolver inputs,
snapshots, receipt state, typed effects, and outcomes. They must never contain
task text or source text, raw mutation arguments, absolute paths, or unhashed
artifact names. A pure sanitized replay record stores only guard comparison
predicates, so deterministic replay does not require source text. Corrupt or
unknown-schema records are reported and excluded, never rewritten as valid
evidence.

Audit and replay are maintainer-only packaged CLI capabilities, not public MCP
tools and not skill routing targets. Synthetic tests cannot substitute for the
live observations required for rollout promotion.

### Version 1 proof boundary

The corpus covers eight mechanism families, but two families intentionally have
narrow positive proof boundaries in version 1:

- exchange/data-exchange is supported only for an exchange-plan source linked
  by a registered event subscription to an exact common-module handler;
- report/print/data-processor is supported only for a registered form owned by
  a report or data processor and an exact form-command/action-to-handler
  binding.

Other variants in those families return `unknown` with
`unsupported_mechanism_variant` until a dedicated typed provider exists.
Lexical similarity never substitutes for a binding.

## Consequences

- Discovery remains domain-neutral and transport-neutral, while providers can
  evolve independently behind typed ports.
- A receipt cannot authorize a neighboring target or a new cross-product of
  otherwise valid fields.
- Content hashing and a lease spanning the mutation add bounded I/O and lock
  cost to enforceable applied operations.
- Provider degradation remains visible as `partial`, `insufficient`, or
  `unknown` instead of being hidden by heuristic fallback.
- Code can implement every rollout mode, but enabling `warn` or `deny` is an
  operational promotion that cannot be claimed from synthetic tests alone.
- PR #83 may supply scenario ideas and provenance context, but its discovery
  core and implementation commits are not merged or cherry-picked.

## Verification

- Strict public-schema, deterministic identity, typed-provider, receipt
  state-machine, and guard tests.
- A 48-case gold corpus, 12 receipt/guard cases, and at least 20 deterministic
  metamorphic variants for every base case.
- Package smoke for the single public MCP and maintainer-only observation
  audit/replay command.
- Architecture contract tests that parse every fenced JSON example and reject
  duplicate keys.
