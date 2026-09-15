# Code Quality, Review, Refactoring, And Performance

## When to use

Use this when the user asks for code review, refactoring, error fixing,
performance optimization, or standards compliance in BSL code.
Use `api-design` for public API, service interface, overridable module,
versioning, or backward compatibility decisions.

Do not use this as a replacement for metadata or runtime tools. Use it together
with object-specific info tools, source search, syntax checks, and focused tests.

## Primary path

Runtime идёт через `unica.run`: вызов без `op` отдаёт словарь операций и
контракт каждой — `argsSchema`, `execution`, `previewRequired`,
`ifRevRequiredOnApply`. Контракт вызова бери оттуда, а не из этого текста;
выбирай только операцию с `implemented: true` и не выдумывай аргументов
записи с `argsSchema: null`; превью исполнением не является. Не обходи
контракт прямым runner-ом.

- Inspect metadata shape with `unica.*.info` tools before changing code that
  depends on objects, forms, roles, or reports.
- Use code search/analysis tools through MCP `unica` where available.
- Check syntax with `unica.check`; test runs are outside the v0.13 surface,
  so never claim YaXUnit or Vanessa Automation results without separate
  execution evidence.
- Report findings first for reviews, ordered by severity and grounded in file
  references.

## Standards to apply

- Business logic belongs in common modules unless the form lifecycle requires a
  form module.
- Avoid query-in-loop, unnecessary server round trips, hidden broad rights, and
  unbounded selections.
- Keep refactors test-first: write a reproducing test and confirm that it fails
  for the defect before changing the code. Then map callers, make the smallest
  coherent fix, check syntax with `unica.check` as a separate static check,
  and retain explicit residual runtime risk.

## Related references

- `../platform/development-standards.md`
- `../platform/platform-solutions.md`
- `forms-ui.md`
- `rights-access.md`
