# 10. Требования к качеству

## Token Efficiency

LLM-visible tool surface must avoid requiring the model to know which internal
engine owns which cache. Coordination belongs to `unica`.

## Safety

Mutating operations with an honest preview default to dry-run and return cache
impact before applied execution. No-preview platform mutations require a typed
prepare/apply authorization and verified postcondition.

Repository-cycle mutations additionally require stable operation IDs, durable
intent/effect/postcondition records, exact ownership, capability proof, and
fail-closed recovery. The original repository binding, no-XML boundary,
no-force merge/commit/unlock rule, exact capability-proven repository structural
update, and owned cleanup policy are non-negotiable.

## Maintainability

Domain, application, infrastructure, and MCP transport code remain separated so
adapter replacement does not rewrite public MCP handling.

## Observability

Operation result must include summary, warnings, errors, artifacts, and cache
impact. Future native adapters should add structured diagnostics without
changing the top-level result shape casually.

Every task-bound branched result requires `resultKind`, task/status,
evidence/data, and the ordinary result fields; `operationId` is required only
for mutation or when a read reports an existing operation. The closed
`completed`/`stopped`/`rejected` union requires `stopCode` only for an
evidence-bearing domain stop and forbids it for safety rejection. Raw
argv/stdout/stderr are omitted. Evidence is bounded, redacted, hash-addressed,
and sufficient to distinguish success, block, unknown effect, safe abandonment,
and completed cleanup.

## Recoverability

MCP disconnect, worker death, platform failure, and repository disconnect must
be testable at every dangerous journal boundary. Recovery never repeats an
unknown external effect and never treats an unavailable owner/lock check as
proof of release.

## Fidelity

Full-cycle completion includes modify, add, delete, form/attribute ownership,
references, multi-vendor support, relevant concurrency, one-version commit,
recovery, archive, and cleanup. Narrower milestones cannot redefine this scope.

## Packaging Reliability

Generated packages must verify checksums and must not depend on globally
installed tools when bundled equivalents exist.
