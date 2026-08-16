---
name: code-review
description: "Код-ревью BSL и изменений 1С. Используй когда пользователь явно просит review, ревью diff/PR/модуля/изменения, поиск дефектов, регрессий, рисков или недостающих тестов."
---

# Code Review

## MCP routing

- Preferred path: use MCP `unica` tools `unica.code.search`, `unica.code.definition`, `unica.code.outline`, `unica.code.graph`, `unica.code.diagnostics`, `unica.meta.info`, `unica.standards.explain`, `unica.standards.search`, `unica.project.map`, and `unica.runtime.execute`.
- По INV-MCP-RUNTIME-RECEIPT текущий runtime-контракт: `unica.runtime.execute` — preview-only и вызывается только с `dryRun: true`; любой applied-режим возвращает fail-closed до workspace discovery и process spawn. Preview не является runtime verification. Не обходи этот отказ прямым runner-ом, через `unica.build.*` или fallback через `unica.runtime.job.*`.
- Use `unica.*.info` tools before reviewing code that depends on metadata shape, form structure, rights, DCS, MXL, or interfaces.
- Do not call internal analyzer, standards, runtime, or package adapters directly. They are hidden behind MCP `unica`.

## Review stance

Lead with findings. Order them by severity and ground each finding in a file/line reference, reproducible path, or diagnostic output. Keep summaries secondary.

## Workflow

1. Identify the review scope: changed files, target source-set, affected metadata objects, public entry points.
2. Resolve changed exported methods and entry points with `unica.code.definition`; inspect large modules with `unica.code.outline`.
3. Use `unica.meta.info` for affected metadata objects to connect the review scope with modules, roles, subscriptions, functional options, and predefined items.
4. Use `unica.code.graph` for callers, callees, neighbors, and impact analysis when a changed method/node can be resolved. Use `unica.code.search` for handlers, literals, query fragments, and non-method tokens.
5. Inspect metadata with `unica.*.info` when code depends on object structure.
6. Run `unica.code.diagnostics` when the review includes BSL code. Select the exact `sourceSet`; use `action=findings` with each touched module's logical `metadataPath`, or `action=analyze` for a broad source-set review. Use `unica.source.locate` first when the diff supplies only a file path. Use `unica.standards.explain` for diagnostic codes or standards-sensitive claims.
7. Check high-risk 1C patterns: transaction boundaries, query-in-loop, server/client context, privileged mode, broad rights, background jobs, external calls, temporary files, and silent exception handling.
8. Use `unica.runtime.execute` only to preview typed syntax/test arguments; always state the exact unverified runtime risk unless separate execution evidence is supplied.

## Output

- Findings first: severity, path, issue, impact, suggested fix.
- Then open questions or assumptions.
- Then brief change/test summary only if useful.

Do not rewrite the code during a review unless the user explicitly asks for fixes after the review.
