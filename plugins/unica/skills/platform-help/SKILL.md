---
name: platform-help
description: "Справка платформы 1С и объектной модели BSL. Используй когда нужно уточнить метод, свойство, конструктор, поведение API, версию платформы, совместимость или стандартное решение задачи."
---

# Platform Help

## MCP routing

- Preferred path: use MCP `unica` tools `unica.standards.search`, `unica.standards.explain`, `unica.code.search`, `unica.project.map`, and `unica.runtime.execute`.
- Use object-specific `unica.*.info` tools when the API question depends on metadata structure.
- Do not call internal standards, runtime, or package adapters directly. They are hidden behind MCP `unica`.

## Workflow

1. State the exact platform/API question: object, method/property, platform version, infobase mode, client/server context, managed/ordinary mode, and whether code runs in UI, server, background job, or external integration.
2. Search standards and platform guidance through `unica.standards.search`; for code fragments use `unica.standards.explain` with `snippet`.
3. Validate against local project context with `unica.project.map` and targeted `unica.code.search` if the answer depends on project conventions.
4. If behavior is version-sensitive, ask for or read the configured platform version before giving a hard answer.
5. For code examples, run `unica.runtime.execute` with `operation=syntax` when feasible.

## Platform context

- Read `references/platform/compatibility-modes.md` for every question about a
  compatibility mode or version-sensitive behavior. Resolve the runtime
  platform, literal mode, effective compatibility version, and
  feature-specific boundary separately.
- Read `references/platform/platform-mechanics.md` when the answer depends on runtime context, auth, temporary storage, data separation, background jobs, or client/server boundaries.
- Read `references/platform/runtime-diagnostics.md` when a platform question is really about a startup/runtime failure and needs evidence before an answer.
- Do not give a platform answer from memory when version, mode, or context can change the behavior. Resolve that first, then answer.

## Stop rules

- Do not invent exact method signatures when `unica.standards.*` cannot confirm them.
- If the requested platform-help source is not available through public MCP `unica`, report it as a Unica MCP contract gap instead of bypassing the public boundary.

## MCP examples

```jsonc
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.standards.search",
    "arguments": {
      "query": "Соответствие Вставить Получить платформа 1С",
      "limit": 5
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
      "snippet": "Результат = Новый Соответствие; Результат.Вставить(\"Ключ\", Значение);",
      "language": "bsl",
      "limit": 5
    }
  }
}
```
