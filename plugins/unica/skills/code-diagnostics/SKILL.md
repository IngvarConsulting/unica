---
name: code-diagnostics
description: "Use when требуется запустить или разобрать диагностики BSL, АПК, EDT, BSL LS, отключения правил или стандарт v8std за диагностикой."
---

# Code Diagnostics

## MCP routing

- Preferred path: use MCP `unica` tools `unica.code.diagnostics`, `unica.source.locate`, `unica.project.map`, `unica.code.graph`, `unica.code.definition`, `unica.code.outline`, `unica.code.search`, `unica.standards.explain`, `unica.standards.search`, and `unica.runtime.execute`.
- Every diagnostics call names an exact `sourceSet`; `cwd` selects the workspace only and never identifies the target. Use `unica.project.map` when the source-set name is unknown.
- Use `action=status` before resident `findings` when readiness is uncertain. A completed status request is not proof that a provider is ready: read each `providers[].readiness.state` and call `findings` only for `ready`; `building`, `notStarted`, and `stale` are not clean results.
- Use `action=findings` with a logical `metadataPath` for one module or metadata object, `action=analyze` for a complete one-shot source-set scan, and `action=catalog` to discover provider-qualified rules. The current live provider `bsl-analyzer` supports module findings only. Until a metadata provider is registered, a metadata-object request returns provider `status=unsupported` with `error.code=target_not_supported`; this means no applicable provider, not clean metadata and not a bad logical address.
- `action=analyze` may set `timeoutSeconds` from 30 to 3600. Without it the call uses `operational.code_diagnostics.analyze_timeout_seconds` from `<workspaceRoot>/unica.local.toml`, then `unica.toml`, then the compiled 120-second fallback. `findings`, `status`, and `catalog` do not read this operational config and do not accept `timeoutSeconds`.
- Put severity and exact case-sensitive codes under `filter`. Every code is `{provider, code}`; obtain provider ids and codes from `catalog`. `limit` is global across providers after normalization and deterministic ordering.
- Read `location` as the logical navigation target and `focus` as the position inside it. `location.kind=addressed` carries `sourceSet`, optional `metadataPath`, and derived `targetKind`; `focus.kind` is `target`, `sourceRange`, or `metadata`.
- Treat `state=partial` as useful but incomplete. A `resourceFailure` is one provider's failure for one logical resource. `location.kind=unaddressable` means the observation is safe to report but cannot be navigated as its own logical target. Neither means clean code.
- For actions that return `items`, only `state=completed`, `complete=true`, `truncated=false`, and provider sections without failures prove an exhaustive answer. Check `itemsTotal` and `itemsReturned` before treating `items` as complete. `status` has no `truncated` field; its evidence is each provider's readiness.
- `unica.code.definition` returns `index_pending:` only while an RLM index is building and `index_unavailable:` for missing, stale, failed, or unavailable indexes. Neither means “no definitions”.
- Use `unica.code.graph` only for diagnostic impact context. v8std access goes only through public `unica.standards.*` tools. Do not call internal analyzer, standards, or package adapters directly.

## Workflow

1. Resolve the exact `sourceSet`. If the starting point is a physical file, use `unica.source.locate` to obtain its logical `metadataPath`.
2. Call `status` when resident readiness matters and `catalog` when rule ids need classification. Use `findings` for one logical target or `analyze` for the whole source set.
3. Group diagnostics by logical `location`, provider-qualified code, and root cause. Follow `focus` for the exact source range or metadata element.
4. Inspect the target with `unica.code.outline`, `unica.code.definition`, or `unica.code.search`. Use `unica.code.graph` before changing shared or exported behavior.
5. Call `unica.standards.explain` with explicit codes; otherwise use `unica.standards.search` by diagnostic name, АПК/EDT/BSL LS token, or nearby snippet.
6. Report source cause, impacted diagnostics, logical target and focus, standard evidence, and verification result.

## Verification gate

This verification gate is mandatory:

- Run diagnostics after syntax-sensitive edits and treat new `error` findings as blocking.
- Run impact analysis when an exported method, metadata handler, public API, query path, or shared module contract changes.
- If public MCP `unica` cannot expose required evidence, report a contract gap instead of claiming full verification.

## Suppression and range-disable comments

When comments отключают diagnostics over a line or range, treat the exact marker as evidence.

- Extract literal rule codes from АПК, EDT, BSL LS, analyzer, or suppression comments.
- Explain an отключение only when the code, surrounding range, and standard support the reason.
- Prefer fixing the cause or narrowing the disabled range. Keep suppression only with standards, platform-help, or runtime evidence of an intentional false positive.

## MCP examples

One logical module, narrowed to one rule. Remove `filter.codes` to include all codes at the selected severity threshold; set `filter.minSeverity=hint` for every severity and still inspect `truncated`:

```jsonc
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.code.diagnostics",
    "arguments": {
      "cwd": "<workspace>",
      "action": "findings",
      "sourceSet": "main",
      "metadataPath": "CommonModule.Продажи.Module",
      "filter": {
        "minSeverity": "warning",
        "codes": [
          {"provider": "bsl-analyzer", "code": "UnusedLocalVariable"}
        ]
      },
      "limit": 100
    }
  }
}
```

Complete source-set scan:

```jsonc
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.code.diagnostics",
    "arguments": {
      "cwd": "<workspace>",
      "action": "analyze",
      "sourceSet": "main",
      "timeoutSeconds": 900
    }
  }
}
```

```jsonc
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.standards.explain",
    "arguments": {
      "codes": ["АПК:142", "LineLength"]
    }
  }
}
```

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.runtime.execute",
    "arguments": {
      "cwd": "<workspace>",
      "operation": "syntax",
      "mode": "designer-modules",
      "dryRun": false
    }
  }
}
```
