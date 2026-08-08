---
name: code-patch
description: Точечно вставить или заменить BSL-код в логически адресованном модуле XML-выгрузки Configuration или Extension 1С. Используй для одной проверяемой операции insert или replace; insert без селектора дописывает содержимое в конец модуля
argument-hint: <sourceSet> <metadataPath> <insert|replace> <content> [method|anchor] [before|after]
allowed-tools:
  - Read
  - Glob
---

# /code-patch — безопасная вставка и замена BSL

## MCP routing

- Preferred path: use MCP `unica` tool `unica.code.patch`; `unica` validates the source set, supported-object state, selector, and exact in-memory BSL postimage before staging and atomic publication.
- Do not call internal MCP/CLI adapters directly. They are hidden behind `unica` and synchronized by the orchestrator.
- Always call `unica.code.patch` with `dryRun: true` first. Call it with `dryRun: false` only after the user explicitly asked to apply this exact change.

`unica.code.patch` edits only a module of an existing metadata object in a supported canonical layout, with its descriptors present, inside the selected Platform XML Configuration or Extension source set. The physical `*Module.bsl` path is resolved privately from `sourceSet + metadataPath`; the removed `path` and `sourceDir` selector fields fail with `legacy_target_removed`. The tool performs exactly one `insert` or one `replace` — `insert` places `content` before or after the selected method or anchor, and `replace` overwrites the selected span itself and does not accept `position`. `selector` is optional for `insert`: without it the content goes to the end of the module, `position` is refused, and a module that holds no method yet is served by that same path. A module file the platform never exported is created on apply, never on preview, and only when the role is one the metadata kind owns and the owner descriptor is proven. The tool cannot create a metadata object, batch-edit files, delete a whole module, edit EDT/external files, or synchronize source with an infobase.

If the requested BSL change cannot be expressed as one safe insertion and needs
a full existing-module replacement, stop this route and use the
`source-access` skill to inspect the target through the read-only
`unica.source.resources` and `unica.source.read`, then come back with a
narrower `insert` or `replace`.

## Parameters

| Parameter | Required | Description |
|---|:---:|---|
| `sourceSet` | yes | Exact configured name of a Platform XML Configuration or Extension source set |
| `metadataPath` | yes | Canonical logical module address, for example `CommonModule.Example.Module` |
| `operation` | yes | `insert` or `replace` |
| `selector` | replace | Exactly one of `{ "method": "Name" }` or `{ "anchor": "text" }`; optional for `insert`, and omitting it appends to the end of the module |
| `content` | yes | Non-empty BSL text to write |
| `position` | insert+selector | `before` or `after`; refused when `insert` names no selector |

Method selectors match an entire procedure or function, including its annotations. Anchor selectors must match exactly once inside a BSL method; LF/CRLF differences in multiline anchors are normalized while returned ranges remain byte-exact. A request is rejected before writing if the resulting selector would become ambiguous and the next identical call could not be proven a no-op. In `OperationResult.data`, read the pre/post hashes, changed range, byte-exact diff, affected owner/module role, and terminal `validation.status` before applying. Preview, no-op, and failed validation do not publish a module-change event.

### Append to the end of a module

Omit `selector` when the content belongs at the end — including the first body of a module that holds no method yet. Preview first; the preview returns the exact diff, pre/post hashes, and BSL parse status without writing.

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
      "content": "Procedure Run()\nEndProcedure",
      "dryRun": true
    }
  }
}
```

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
