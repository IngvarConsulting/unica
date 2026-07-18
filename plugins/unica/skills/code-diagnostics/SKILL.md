---
name: code-diagnostics
description: "Диагностика BSL и объяснение отключений диагностик в коде. Используй когда нужно запустить или разобрать диагностики, объяснить коды АПК, EDT, BSL LS, inline/range disable markers, suppression-комментарии или стандарт v8std за диагностикой."
---

# Code Diagnostics

## MCP routing

- Preferred path: use MCP `unica` tools `unica.code.diagnostics`, `unica.code.graph`, `unica.code.definition`, `unica.code.outline`, `unica.code.grep`, `unica.code.search`, `unica.standards.explain`, `unica.standards.search`, and `unica.runtime.execute`.
- Use `unica.code.diagnostics` with `mode=analyze` or no `mode` for the classic analyzer run; large workspaces may set `timeoutSeconds` from 30 to 3600 (default 120). Use `mode=status|catalog|file|workspace` for typed diagnostic catalog and scoped diagnostic reads; those modes do not accept `timeoutSeconds`.
- Use `unica.code.graph` only for diagnostic impact context: containing node, callers, callees, neighbors, or workspace graph status.
- v8std access goes only through public `unica.standards.*` tools.
- Do not call internal analyzer, standards, or package adapters directly. They are hidden behind MCP `unica`.

## Workflow

1. Run `unica.code.diagnostics` for the selected source-set or module. Start with `mode=status` when the analyzer workspace model may still be loading, and use `mode=catalog` when diagnostic codes need classification.
2. Group diagnostics by file, diagnostic id/code, and root cause. Do not fix duplicate reports independently when one source issue explains them.
3. For one file or range, use `unica.code.diagnostics` with `mode=file`; then use `unica.code.outline`, `unica.code.definition`, or `unica.code.grep` for exact context.
4. When diagnostic output includes a graph id or the fix may affect callers/callees, inspect impact with `unica.code.graph` before proposing a change.
5. Search nearby code with `unica.code.search` only when exact context tools do not identify the root cause.
6. For each diagnostic id/code, call `unica.standards.explain` with `codes` when the code is explicit; otherwise search `unica.standards.search` by diagnostic name, APK/EDT/BSL LS token, or nearby snippet.
7. Report fixes in cause-first order: source defect, impacted diagnostics, graph impact if relevant, standard reference, verification command.

## Verification gate

- The verification gate is part of the delivery contract, not an optional final
  polish step.
- Run diagnostics after syntax-sensitive edits and treat new `error` or `critical`
  findings as blocking.
- Run impact analysis with `unica.code.graph` when an exported method, metadata
  handler, public API, query path, or shared module contract changes.
- If public MCP `unica` cannot expose the required syntax, diagnostic, or impact
  evidence, report that as a Unica MCP contract gap instead of claiming the
  change is fully verified.

## Suppression and range-disable comments

When comments disable diagnostics over a line or range, treat the exact marker as evidence, not as decoration.

- Extract literal codes or ids from the comment: АПК, EDT, BSL LS, analyzer rule names, numeric or mnemonic ids.
- Use `unica.standards.explain` with all extracted codes. If v8std does not resolve a code, search with `unica.standards.search` using the code plus nearby diagnostic text.
- Explain why the отключение exists only when the code, surrounding range, and standard support the reason. If the reason is absent, say that the suppression is not justified in the source.
- Prefer narrowing the disabled range or fixing the code. Keep suppression only when the standard or platform limitation makes the diagnostic intentionally false-positive.

## MCP examples

```jsonc
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.code.diagnostics",
    "arguments": {
      "cwd": "<workspace>",
      "sourceDir": "src",
      "mode": "file",
      "path": "CommonModules/Продажи/Ext/Module.bsl",
      "limit": 100
    }
  }
}
```

```jsonc
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.code.diagnostics",
    "arguments": {
      "cwd": "<workspace>",
      "mode": "catalog",
      "codes": ["UnusedLocalVariable", "DataExchangeLoading"]
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
