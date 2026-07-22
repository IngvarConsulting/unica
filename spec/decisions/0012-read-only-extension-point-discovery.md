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

## Consequences

The active request, result, bounds, evidence, snapshot, package acceptance, and
Slice B selection-gate contract is specified in
[`extension-point-discovery.md`](../architecture/extension-point-discovery.md).
The public boundary remains one server named `unica` with `unica.*` tools.
