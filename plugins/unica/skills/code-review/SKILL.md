---
name: code-review
description: "Код-ревью BSL и изменений 1С. Используй когда пользователь явно просит review, ревью diff/PR/модуля/изменения, поиск дефектов, регрессий, рисков или недостающих тестов."
---

# Code Review

## MCP routing

- Preferred path: use MCP `unica` tools `unica.search`, `unica.check`, `unica.view` on the object node, `unica.docs`, `unica.view {}`, and `unica.run`.
- Runtime идёт через `unica.run`: вызов без `op` отдаёт словарь операций и
контракт каждой — `argsSchema`, `execution`, `previewRequired`,
`ifRevRequiredOnApply`. Контракт вызова бери оттуда, а не из этого текста;
выбирай только операцию с `implemented: true` и не выдумывай аргументов
записи с `argsSchema: null`; превью исполнением не является. Не обходи
контракт прямым runner-ом.
- Use `unica.view` at the object's logical address before reviewing code that depends on metadata shape, form structure, rights, DCS, or interfaces; a spreadsheet template is read by `unica.view` at its template address.
- Do not call internal analyzer, standards, runtime, or package adapters directly. They are hidden behind MCP `unica`.

## Review stance

Lead with findings. Order them by severity and ground each finding in a file/line reference, reproducible path, or diagnostic output. Keep summaries secondary.

## Workflow

1. Identify the review scope: changed files, target source-set, affected metadata objects, public entry points.
2. Find changed exported methods and entry points with `unica.search`; inspect large modules with `unica.view` on the module node (its `Method` branch lists the methods).
3. Use `unica.view` on the object node for affected metadata objects to connect the review scope with modules, roles, subscriptions, functional options, and predefined items.
4. Find callers and impact with `unica.search` by the method name (a call graph is not on the v0.13 surface); the same search covers handlers, literals, query fragments, and non-method tokens.
5. Inspect metadata with `unica.view` on the object node when code depends on object structure.
6. Run `unica.check {at}` on each touched module when the review includes BSL code; a whole-source-set analysis is not on the v0.13 surface, so check module by module. Use `unica.resolve` first when the diff supplies only a file path. Use `unica.docs` with `source: "development-standard"` for diagnostic codes or standards-sensitive claims.
7. Check high-risk 1C patterns: transaction boundaries, query-in-loop, server/client context, privileged mode, broad rights, background jobs, external calls, temporary files, and silent exception handling.
8. Check syntax with `unica.check` (test runs are outside the v0.13 surface); always state the exact unverified runtime risk unless separate execution evidence is supplied.

## Output

- Findings first: severity, path, issue, impact, suggested fix.
- Then open questions or assumptions.
- Then brief change/test summary only if useful.

Do not rewrite the code during a review unless the user explicitly asks for fixes after the review.
