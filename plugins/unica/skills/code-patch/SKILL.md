---
name: code-patch
description: Точечно вставить BSL-код в существующий *Module.bsl из XML-выгрузки конфигурации 1С. Используй для одной проверяемой вставки до или после метода либо якоря внутри метода
argument-hint: <path> <method|anchor> <content> [before|after]
allowed-tools:
  - Read
  - Glob
---

# /code-patch — безопасная точечная вставка BSL

## MCP routing

- Preferred path: use MCP `unica` tool `unica.code.patch`; `unica` validates the source set, supported-object state, selector, and exact in-memory BSL postimage before staging and atomic publication.
- Do not call internal MCP/CLI adapters directly. They are hidden behind `unica` and synchronized by the orchestrator.
- Always call `unica.code.patch` with `dryRun: true` first. Call it with `dryRun: false` only after the user explicitly asked to apply this exact insertion.

`unica.code.patch` v1 edits only an existing regular `*Module.bsl` in a supported canonical layout, with its metadata descriptors present, inside the selected platform-XML Configuration source set. It performs exactly one `insert`; it cannot create a module, batch-edit files, replace or delete text, edit EDT/external files, or synchronize source with an infobase.

## Parameters

| Parameter | Required | Description |
|---|:---:|---|
| `path` | yes | Existing `*Module.bsl`, relative to the workspace |
| `operation` | yes | Always `insert` |
| `selector` | yes | Exactly one of `{ "method": "Name" }` or `{ "anchor": "text" }` |
| `content` | yes | Non-empty BSL text to insert |
| `position` | yes | `before` or `after` |
| `sourceDir` | no | Configured Configuration source set; required when the workspace is ambiguous |

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
      "path": "src/cf/CommonModules/Example/Ext/Module.bsl",
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
      "path": "src/cf/CommonModules/Example/Ext/Module.bsl",
      "operation": "insert",
      "selector": { "anchor": "Сообщить(\"Готово\");" },
      "content": "Лог.Информация(\"Операция завершена\");",
      "position": "after",
      "dryRun": false
    }
  }
}
```
