# 8. Сквозные концепции

## Single Public MCP

The LLM sees one server and does not coordinate multiple MCP caches or indexes.
This is the primary token and context saving mechanism.

## Dry Run Safety

Mutating tools default to dry-run. Skills pass `dryRun: false` only for explicit
user-requested mutations.

## Cache Ownership

The orchestrator owns cache state. Adapter calls must report through application
use cases so domain events and cache invalidation cannot be bypassed.

## Internal Adapter Pattern

Adapters are typed boundaries around existing engines. They may use CLI or MCP
protocol internally, but their names and cache lifecycle are not exposed to LLM.

Python/PowerShell/Bash operation files are not a runtime adapter class for
developer operations. Donor scripts can be kept only as fixture reference models
for native `unica.*` MCP handlers.

## Workspace-scoped Services

Some internal adapters may run behind hidden workspace services. These services
are owned by `unica`, scoped by workspace and source root, and coordinated
through volatile cache state. They are not public MCP registrations and must not
appear in skills as routing targets.

The lifecycle rule is lazy start, reuse while live, invalidate on domain events,
and natural exit after idle or max-age limits. Cheap read-only tools that do not
need warm analyzer/index state must not start the service.

## Source Of Truth Order

When documents disagree, use this order:

1. current code and tests;
2. package manifests and `.mcp.json`;
3. active `spec/`;
4. README and skill prose;
5. archived or research docs.

## Typed Discovery Evidence

Project discovery is an application use case over typed evidence ports. Provider
facts retain canonical identity, location, source fingerprint, coverage,
freshness, versioned provenance, and stable outcome codes. Infrastructure must
not infer architecture, parse display output, read another adapter's storage,
or hide a missing provider behind an unbounded scan.

Related artifacts, runtime flow edges, and actionable extension points are
separate domains. Lexical evidence can help find an artifact but cannot prove a
platform callback, command binding, or reachable handler. Conflicts and material
provider gaps remain explicit blockers.

## Discovery Receipts And Concurrency

A discovery receipt is server-owned evidence of a validated investigation, not
user authorization. Its atomic grants bind tool, target, mutation class, change
kind, normalized output-affecting parameters, destination source-set, and exact
allowed artifacts without independent-list expansion.

Receipt freshness uses a content-based composite fingerprint of every linked
analysis and destination source-set. `workspaceEpoch`, timestamps, and file
sizes are diagnostic inputs only. Before an enforceable applied mutation with a
valid receipt, an exclusive receipt lease is acquired and remains held through
the handler, typed-effect/manifest verification, and atomic revision update. An
observe/warn call allowed without a receipt has no receipt transition. Dry-run
never leases or advances a receipt.

## Discovery Policy Rollout

Descriptor requirement (`not_required`, `advisory_only`, or `enforceable`) is
independent from configured mode (`off`, `observe`, `warn`, or `deny`). Support
policy runs first. Mode is server/workspace configuration, not a call argument;
there is no per-call discovery bypass. Package default is `observe`, while
promotion requires both corpus quality and audited real observations.

## Privacy-Preserving Shadow Evidence

The guard emits a non-authoritative observation only after the operation
outcome is known. An OS-locked, schema-versioned JSONL journal stores stable
names, reason codes, policy predicates, revisions, sequence/timing metadata,
and digests. It must never contain task text or source text, raw mutation
arguments, absolute paths, or unhashed artifact names.

Journal errors never affect handler, receipt, or rollout decisions. Sanitized
comparison predicates support deterministic replay and maintainer-only audit;
neither capability is a public MCP tool.
