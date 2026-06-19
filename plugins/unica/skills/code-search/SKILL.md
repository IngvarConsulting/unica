---
name: code-search
description: "Поиск и исследование BSL-кода и точек входа 1С. Используй когда нужно найти реализацию, вызовы, обработчики, модули, поток выполнения или быстро разобраться в механизме конфигурации."
---

# Code Search

## MCP routing

- Preferred path: use MCP `unica` tools `unica.code.search`, `unica.code.definition`, `unica.code.outline`, `unica.code.grep`, and `unica.project.map`.
- Use object-specific `unica.*.info` tools when code behavior depends on metadata, forms, SKD, roles, or HTTP service structure.
- Do not call internal code-index, analyzer, or package adapters directly. They are hidden behind MCP `unica`.

## Tool choice

- Use `unica.code.definition` for an exact procedure/function definition by name, especially exported methods.
- Use `unica.code.outline` before reading a large module; it gives regions, header context, and method ranges.
- Use `unica.code.grep` for arbitrary text, XML, query fragments, string literals, captions, and non-method tokens.
- Use `unica.code.search` for broad BSL search and mixed analyzer/index results.

## Workflow

1. Map the workspace with `unica.project.map` when the active source-set or source format is unclear.
2. Resolve exact method names with `unica.code.definition`; inspect large candidate modules with `unica.code.outline`.
3. Search exact identifiers next: object names, module names, event handlers, exported procedures, command names, URL templates.
4. Use `unica.code.grep` for raw text fragments that are not BSL method names.
5. Broaden only after exact search fails: synonyms, business terms, common module prefixes, form command captions.
6. For every result, separate declaration, caller, handler, and dead-looking match. Do not infer flow from one hit.
7. Report concrete file paths and line anchors; include the query that produced each important hit when the search was non-obvious.

## Common searches

- Object lifecycle: `ОбработкаПроведения`, `ПередЗаписью`, `ПриЗаписи`, `ПриСозданииНаСервере`.
- Managed form flow: command handler, server procedure, client wrapper, form attribute name.
- Integrations: HTTP service root URL, method name, header name, endpoint path, exchange plan node.
- BSP entry points: exported common-module procedure plus surrounding callers.

## MCP examples

```jsonc
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.project.map",
    "arguments": {
      "cwd": "<workspace>"
    }
  }
}
```

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.code.search",
    "arguments": {
      "cwd": "<workspace>",
      "query": "ОбработкаПроведения",
      "limit": 20
    }
  }
}
```

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.code.definition",
    "arguments": {
      "cwd": "<workspace>",
      "name": "ОбработкаПроведения",
      "limit": 10
    }
  }
}
```

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.code.grep",
    "arguments": {
      "cwd": "<workspace>",
      "query": "ВЫБРАТЬ",
      "fileTypes": "bsl",
      "limit": 20
    }
  }
}
```
