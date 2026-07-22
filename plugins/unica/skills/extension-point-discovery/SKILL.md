---
name: extension-point-discovery
description: Use when planning or implementing changes to existing typical or supported 1C configurations, CFE extensions, managed forms, documents, data processors, event handlers, or tabular sections.
---

## MCP routing

Use MCP `unica`. Make `unica.project.discover` the first inspection call. Call it before planning and before any mutation or manual XML/BSL edit, even when a likely method or object is already known.

Pass the original task. Add only object names already confirmed by the user or repository evidence; never invent metadata names. `cwd` is optional when the current workspace is already correct.

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "tools/call",
  "params": {
    "name": "unica.project.discover",
    "arguments": {
      "mode": "explore",
      "task": "При поступлении товаров контролировать остаточный срок годности серий",
      "cwd": "/workspace/project"
    }
  }
}
```

This task-only request is valid. Do not put guessed objects, search terms, or policy fields into the first call.

## Inspect the typed result

Inspect only `OperationResult.data.discovery`; presentation text is not discovery evidence. Read these fields before choosing an extension point:

- `schemaVersion`, `status`, `source`, and `concepts`;
- `providerOutcomes`, `warnings`, and `missingChecks`;
- `candidates`, `structuralEdges`, and `runtimeFlowEdges`;
- `evidence`, including every cited `evidenceIds` entry and its `location`;
- `analysisSnapshot.mappingFingerprint`, `analysisSnapshot.fingerprint`, and every `contributors` item with `relativePath`, `rawHash`, and `bytes`.

Resolve each candidate's `evidenceIds` against top-level `evidence[]`. Use the resulting locations when explaining why the candidate is selected or rejected. Do not infer runtime flow from a structural or lexical relationship.

The analysis snapshot is not mutation authorization, not a freshness guarantee, and not a mutation receipt. Run a new preflight whenever the evidence inputs may have changed.

## Close evidence gaps

Use only this public read-only allowlist to close a missing check:

- `unica.project.map`
- `unica.project.status`
- `unica.meta.info`
- `unica.meta.profile`
- `unica.form.info`
- `unica.code.search`
- `unica.code.definition`
- `unica.code.outline`
- `unica.code.grep`
- `unica.code.graph`
- `unica.cf.info`

No other tool is an allowed gap-closure path. Preserve the returned `providerOutcomes`, `warnings`, and `missingChecks` with the decision evidence.

`partial` is a review signal, not an automatic stop. Stop before planning or mutation when an unresolved material check, a blocking warning, or incomplete evidence can change the selected architecture. Continue only when remaining gaps are explicitly non-material to the choice.

## Record the selection

After inspecting the result and allowed gap evidence, record exactly this shape. Copy candidate fields from discovery, dereference evidence locations, state a concrete rejection reason, and retain the snapshot values verbatim. For support state, copy `supportState` verbatim when the candidate contains it; omit it when the candidate does not report it.

```json
{
  "selectedPoint": {
    "target": "<candidate-target>",
    "kind": "<candidate-kind>",
    "evidenceIds": ["<evidence-id>"],
    "evidenceLocations": [
      {"relativePath": "Documents/ПриобретениеТоваровУслуг.xml", "line": 2}
    ],
    "supportState": "<candidate-support-state>"
  },
  "rejectedAlternatives": [
    {
      "target": "<rejected-candidate-target>",
      "reason": "A blocking discovery warning shows insufficient coverage.",
      "evidenceIds": ["<evidence-id>"]
    }
  ],
  "unresolvedNonMaterialChecks": [
    {
      "provider": "<provider>",
      "code": "<code>",
      "message": "<message>"
    }
  ],
  "analysisSnapshot": {
    "fingerprint": "<fingerprint>",
    "contributors": [
      {"relativePath": "<relative-path>", "rawHash": "<raw-sha256>", "bytes": 0}
    ]
  }
}
```

## Do not bypass the preflight

| Temptation | Required response |
|---|---|
| A remembered method name seems sufficient | Call discovery first and require cited evidence. |
| Run discovery only when mapping is ambiguous | Run it for every triggered planning or implementation task. |
| Start from naming conventions | Treat names as search hints only after the task-only call. |
| Treat a project revision as current proof | A revision is not a freshness guard; use the reported analysis snapshot only as analysis evidence. |
| Stop because of missing concrete names | The task-only call is valid; run it before requesting or guessing names. |
