# Code Quality, Review, Refactoring, And Performance

## When to use

Use this when the user asks for code review, refactoring, error fixing,
performance optimization, or standards compliance in BSL code.
Use `api-design` for public API, service interface, overridable module,
versioning, or backward compatibility decisions.

Do not use this as a replacement for metadata or runtime tools. Use it together
with object-specific info tools, source search, syntax checks, and focused tests.

## Primary path

По INV-MCP-RUNTIME-RECEIPT и ADR-0074: `unica.runtime.execute` с `dryRun: true`
показывает запланированную команду без побочных эффектов, а с `dryRun: false`
исполняет классифицированную операцию и отвечает её терминальным результатом в
том же вызове, приложив названную причину риска (`runtime_risk_*`)
предупреждением; неклассифицированная операция по-прежнему отказывает
`runtime_operation_unbounded` до обнаружения рабочего пространства. Preview
исполнением не является. Работу, которую вызов ждать не должен, запускай через
`unica.runtime.job.start`. Не обходи контракт прямым runner-ом или через
`unica.build.*`.

- Inspect metadata shape with `unica.*.info` tools before changing code that
  depends on objects, forms, roles, or reports.
- Use code search/analysis tools through MCP `unica` where available.
- Use `v8-runner` with `dryRun: true` to preview `unica.runtime.execute`
  `operation=syntax`/`operation=test` arguments and with `dryRun: false` to run
  them; never claim YaXUnit, Vanessa Automation, or syntax validation from a
  preview alone.
- Report findings first for reviews, ordered by severity and grounded in file
  references.

## Standards to apply

- Business logic belongs in common modules unless the form lifecycle requires a
  form module.
- Avoid query-in-loop, unnecessary server round trips, hidden broad rights, and
  unbounded selections.
- Keep refactors test-first: write a reproducing test and confirm that it fails
  for the defect before changing the code. Then map callers, make the smallest
  coherent fix, preview intended syntax/test arguments as a separate typed-
  argument check, and retain explicit residual runtime risk.

## Related references

- `../platform/development-standards.md`
- `../platform/platform-solutions.md`
- `forms-ui.md`
- `rights-access.md`
