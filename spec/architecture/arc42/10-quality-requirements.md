# 10. Требования к качеству

## Token Efficiency

LLM-visible tool surface must avoid requiring the model to know which internal
engine owns which cache. Coordination belongs to `unica`.

## Safety

Mutating operations default to dry-run and return cache impact before applied
execution.

Discovery never promotes lexical similarity or incomplete coverage to a
supported extension point. Every discovery receipt uses atomic target scope,
content fingerprints, and an exclusive receipt lease through handler execution
and post-mutation verification. Zero false receipt issue, stale acceptance,
scope expansion, or rejection of a valid in-scope receipt is tolerated.

## Maintainability

Domain, application, infrastructure, and MCP transport code remain separated so
adapter replacement does not rewrite public MCP handling.

## Observability

Operation result must include summary, warnings, errors, artifacts, and cache
impact. Future native adapters should add structured diagnostics without
changing the top-level result shape casually.

Discovery and guard decisions expose typed evidence/checks, stable reason codes,
and receipt eligibility. Shadow observations are non-authoritative and their
schema-versioned JSONL journal must remain auditable and deterministically
replayable. Journal and replay records must never contain task text or source
text.

## Determinism

For identical normalized input, analysis-contract version, provider versions
and typed outcomes, deterministic limits, and source fingerprints, analysis and
evidence identities, ordering, verdicts, receipt eligibility, and replayed guard
decisions are stable. Timestamps, `workspaceEpoch`, durations, and display-only
diagnostics do not participate in stable digests.

## Discovery Acceptance

The committed corpus contains 48 semantic scenarios across eight independent
mechanism families, plus 12 rolling receipt/guard scenarios and at least 20
deterministic metamorphic variants for each base case. Safety failures have zero
tolerance. Aggregate precision/recall and real shadow volumes gate rollout
promotion separately; synthetic corpus success cannot authorize `warn` or
`deny`.

Version 1 proves exchange/data-exchange only through a registered event
subscription and exact common-module handler. It proves report/data-processor
flows only through an owned registered form with an exact command/action binding
to a form handler. Other variants remain typed `unknown`.

## Packaging Reliability

Generated packages must verify checksums and must not depend on globally
installed tools when bundled equivalents exist.
