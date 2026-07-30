---
name: code-patch
description: Точечно вставить BSL-код в логически адресованный модуль XML-выгрузки Configuration или Extension 1С. Используй для одной проверяемой вставки до или после метода либо якоря внутри метода
argument-hint: <sourceSet> <metadataPath> <method|anchor> <content> [before|after]
allowed-tools:
  - Read
  - Glob
---

# /code-patch — безопасная точечная вставка BSL

## MCP routing

- Preferred path: use MCP `unica` tool `unica.code.patch`; `unica` validates the source set, supported-object state, selector, and exact in-memory BSL postimage before staging and atomic publication.
- Do not call internal MCP/CLI adapters directly. They are hidden behind `unica` and synchronized by the orchestrator.
- Always call `unica.code.patch` with `dryRun: true` first. Call it with `dryRun: false` only after the user explicitly asked to apply this exact insertion.

`unica.code.patch` edits only an existing module in a supported canonical layout, with its metadata descriptors present, inside the selected Platform XML Configuration or Extension source set. The physical `*Module.bsl` path is resolved privately from `sourceSet + metadataPath`; the removed `path` and `sourceDir` selector fields fail with `legacy_target_removed`. The tool performs exactly one `insert` or one `replace` — `insert` places `content` before or after the selected method or anchor, `replace` overwrites the selected span itself and does not accept `position`; it cannot create a module, batch-edit files, delete a whole module, edit EDT/external files, or synchronize source with an infobase.

If the requested BSL change cannot be expressed as one safe insertion and needs
a full existing-module replacement, stop this route and use the
`source-access` skill. It must explain why the specialized
`unica.code.patch` writer is insufficient, inspect the issued snapshot, and
preview `unica.source.apply` before any applying call.

## Parameters

| Parameter | Required | Description |
|---|:---:|---|
| `sourceSet` | yes | Exact configured name of a Platform XML Configuration or Extension source set |
| `metadataPath` | yes | Canonical logical module address, for example `CommonModule.Example.Module` |
| `operation` | yes | Always `insert` |
| `selector` | yes | Exactly one of `{ "method": "Name" }` or `{ "anchor": "text" }` |
| `content` | yes | Non-empty BSL text to insert |
| `position` | yes | `before` or `after` |

Method selectors match an entire procedure or function, including its annotations. Anchor selectors must match exactly once inside a BSL method; LF/CRLF differences in multiline anchors are normalized while returned ranges remain byte-exact. A request is rejected before writing if the resulting selector would become ambiguous and the next identical call could not be proven a no-op. In `OperationResult.data`, read the pre/post hashes, changed range, byte-exact diff, affected owner/module role, and terminal `validation.status` before applying. Preview, no-op, and failed validation do not publish a module-change event.

## MCP examples

### Dry run before a method

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.code.patch",
    "arguments": {
      "cwd": "<workspace>",
      "sourceSet": "main",
      "metadataPath": "CommonModule.Example.Module",
      "operation": "insert",
      "selector": { "method": "ПриСозданииНаСервере" },
      "content": "// TODO: добавить проверку",
      "position": "before",
      "dryRun": true
    }
  }
}
```

### Apply after an anchor

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.code.patch",
    "arguments": {
      "cwd": "<workspace>",
      "sourceSet": "myExtension",
      "metadataPath": "CommonModule.Example.Module",
      "operation": "insert",
      "selector": { "anchor": "Сообщить(\"Готово\");" },
      "content": "Лог.Информация(\"Операция завершена\");",
      "position": "after",
      "dryRun": false
    }
  }
}
```
