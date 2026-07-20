---
name: code-patch
description: Точечно вставить BSL-код в существующий Module.bsl из XML-выгрузки конфигурации 1С. Используй для одной проверяемой вставки до или после метода либо якоря внутри метода
argument-hint: <path> <method|anchor> <content> [before|after]
allowed-tools:
  - Read
  - Glob
---

# /code-patch — безопасная точечная вставка BSL

## MCP routing

- Preferred path: use MCP `unica` tool `unica.code.patch`; `unica` validates the source set, supported-object state, selector, and writes atomically.
- Do not call internal MCP/CLI adapters directly. They are hidden behind `unica` and synchronized by the orchestrator.
- Always call `unica.code.patch` with `dryRun: true` first. Call it with `dryRun: false` only after the user explicitly asked to apply this exact insertion.

`unica.code.patch` v1 edits only an existing `Module.bsl` inside the selected platform-XML Configuration source set. It performs exactly one `insert`; it cannot create a module, batch-edit files, replace or delete text, edit EDT/external files, or synchronize source with an infobase.

## Parameters

| Parameter | Required | Description |
|---|:---:|---|
| `path` | yes | Existing `Module.bsl`, relative to the workspace |
| `operation` | yes | Always `insert` |
| `selector` | yes | Exactly one of `{ "method": "Name" }` or `{ "anchor": "text" }` |
| `content` | yes | Non-empty BSL text to insert |
| `position` | yes | `before` or `after` |

Method selectors match an entire procedure or function. Anchor selectors must match exactly once and be located inside one BSL method. Read the returned pre/post hashes, changed range, and diff before applying.

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
