---
name: code-review
description: "Код-ревью BSL и изменений 1С. Используй когда пользователь явно просит review, ревью diff/PR/модуля/изменения, поиск дефектов, регрессий, рисков или недостающих тестов."
---

# Code Review

## MCP routing

- Preferred path: use MCP `unica` tools `unica.code.search`, `unica.code.definition`, `unica.code.outline`, `unica.code.grep`, `unica.code.diagnostics`, `unica.standards.explain`, `unica.standards.search`, `unica.project.map`, and `unica.runtime.execute`.
- Use `unica.*.info` tools before reviewing code that depends on metadata shape, form structure, rights, SKD, MXL, or interfaces.
- Do not call internal analyzer, standards, runtime, or package adapters directly. They are hidden behind MCP `unica`.

## Review stance

Lead with findings. Order them by severity and ground each finding in a file/line reference, reproducible path, or diagnostic output. Keep summaries secondary.

## Workflow

1. Identify the review scope: changed files, target source-set, affected metadata objects, public entry points.
2. Resolve changed exported methods and entry points with `unica.code.definition`; inspect large modules with `unica.code.outline`.
3. Map callers, handlers, literals, and query fragments with `unica.code.search` and `unica.code.grep`; inspect metadata with `unica.*.info` when code depends on object structure.
4. Run `unica.code.diagnostics` when the review includes BSL code. Use `unica.standards.explain` for diagnostic codes or standards-sensitive claims.
5. Check high-risk 1C patterns: transaction boundaries, query-in-loop, server/client context, privileged mode, broad rights, background jobs, external calls, temporary files, and silent exception handling.
6. Verify with `unica.runtime.execute` syntax/tests when feasible; otherwise state the exact unverified risk.

## Output

- Findings first: severity, path, issue, impact, suggested fix.
- Then open questions or assumptions.
- Then brief change/test summary only if useful.

Do not rewrite the code during a review unless the user explicitly asks for fixes after the review.
