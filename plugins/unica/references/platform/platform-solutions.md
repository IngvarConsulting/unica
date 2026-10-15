# Platform Cases And Fix Templates

Use this as a compact checklist for common 1C platform risks.

## Query In Loop

Symptom: repeated query execution inside object or table iteration.

Fix: collect keys first, run one query with `IN (&Keys)`, then map results.

## Form Round Trips

Symptom: client handler calls the server for every row or every field change.

Fix: batch input data, use one server call, update client state once.

## Rights Drift

Symptom: code starts touching new metadata objects but roles are not updated.

Fix: inspect touched objects, add or update focused rights through `unica.apply`
(`right.add`, `right.set`), then check the role with `unica.check`.

## External Integration Timeouts

Symptom: HTTP or file exchange code can hang indefinitely.

Fix: set explicit connection/read timeouts and return actionable diagnostics.

## Unsupported Platform API

Symptom: method exists in one platform version but fails on the target baseline.

Fix: verify the project platform version and choose the supported API variant.
