---
name: code-search
description: "Поиск и исследование BSL-кода и точек входа 1С. Используй когда нужно найти реализацию, вызовы, обработчики, модули, поток выполнения или быстро разобраться в механизме конфигурации."
---

# Code Search

## MCP routing

- Preferred path: use MCP `unica` tools `unica.code.search`, `unica.code.definition`, `unica.code.outline`, `unica.code.graph`, `unica.meta.info`, and `unica.project.map`.
- Use object-specific `unica.*.info` tools when code behavior depends on metadata, forms, DCS, roles, or HTTP service structure.
- Do not call internal code-index, analyzer, or package adapters directly. They are hidden behind MCP `unica`.
- `sourceSet` — это имя набора исходников из `v8project.yaml`, а не
  константа. Получите его через `unica.project.map`; `"main"` в примерах
  ниже — иллюстрация, а не значение по умолчанию.

## Tool choice

- MCP-first discipline: prefer the public `unica.*` project-index tools before
  shell search for 1C source. Use shell search only after the relevant
  `unica.code.*` / `unica.meta.*` attempts did not close the context gap, and
  report what was tried when that fallback matters to the answer.
- Use `unica.code.definition` for an exact procedure/function definition by name, especially exported methods.
- Use `unica.code.outline` before reading a large module; it gives regions, header context, and method ranges.
- Use `unica.code.search` for arbitrary text, XML, query fragments, string literals, captions, and non-method tokens. Read its role sections independently: `semantic`, `symbol`, then `lexical`; `provider` only reports the replaceable implementation that produced a section.
- `unica.code.search.limit` is the per-provider result cap: `1..50`, default `20`.
- Prefer the logical selector `sourceSet` from `unica.project.map`; add
  `metadataPath` to constrain the search to one logical object. The migration
  selector `sourceDir` is accepted only instead of `sourceSet`, never together
  with it, and cannot be combined with `metadataPath`.
- While a search is running, treat `notifications/progress` with typed
  `io.unica/searchProgress` metadata as proof of life. Wait for the terminal
  result from all three roles; do not poll by starting another search.
- Interpret `searchComplete`, `matches.relation`, `ranking`, and `ordering`
  together. `empty` proves exact zero; `limitReached` and `timedOut` preserve a
  lower-bound prefix and are not empty. The lexical role is deliberately
  unranked (`ranking: none`, `ordering: providerTraversal`).
- Inspect `termination` instead of parsing diagnostics: it is `null` for
  `ok`/`empty`, otherwise its provider-neutral `code` explains the terminal
  condition and `retryable` says whether repeating later can help. In
  particular, `dependencyPending` with `detailCode: buildingIndex` means the
  RLM deadline ended while the index was still building; keep results from the
  other roles and retry search later only if semantic evidence is still needed.
- When `dependencyPending` includes `retryAfterMs` or `state`, preserve that
  provider-supplied guidance. Do not invent a polling interval from prose.
- Reuse an `addressed` hit through its `sourceSet` and `metadataPath`.
  `unaddressable` is an observable source-relative location, not a logical
  target for a following mutation or subject reader.
- Use `unica.code.graph` for callers, callees, neighbors, graph overview, and impact analysis when a method or metadata node id is known or can be resolved.
- Use `unica.meta.info` for a compact metadata object profile: structure, modules, roles, event subscriptions, functional options, and predefined items.

## Workflow

1. Map the workspace with `unica.project.map` when the active source-set or source format is unclear.
2. For an exact metadata object name, call `unica.meta.info` before broad search to identify related modules, rights, subscriptions, and functional options.
3. Resolve exact method names with `unica.code.definition`; inspect large candidate modules with `unica.code.outline`.
4. For flow questions, resolve the node and ask `unica.code.graph` for callers, callees, or neighbors before treating lexical hits as execution flow.
5. Search exact identifiers next: object names, module names, event handlers, exported procedures, command names, URL templates.
6. Use `unica.code.search` for raw text fragments that are not BSL method names and inspect its role-local sections independently, including incomplete or unavailable roles.
7. Broaden only after exact search fails: synonyms, business terms, common module prefixes, form command captions.
8. Fall back to local `rg` only for repository files outside the public Unica index or after the MCP-first attempts above were insufficient.
9. For every result, separate declaration, caller, handler, graph edge, and dead-looking match. Do not infer flow from one hit.
10. Report concrete file paths and line anchors; include the query that produced each important hit when the search was non-obvious.

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
      "sourceSet": "main",
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
    "name": "unica.code.graph",
    "arguments": {
      "cwd": "<workspace>",
      "mode": "callers",
      "id": "method:CommonModule.Продажи.ОбработкаПроведения",
      "limit": 25
    }
  }
}
```

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.meta.info",
    "arguments": {
      "sourceSet": "main",
      "metadataPath": "Document.SalesOrder",
      "sections": ["roles", "subscriptions", "functionalOptions"],
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
    "name": "unica.code.search",
    "arguments": {
      "cwd": "<workspace>",
      "sourceSet": "main",
      "query": "ВЫБРАТЬ",
      "limit": 20
    }
  }
}
```
