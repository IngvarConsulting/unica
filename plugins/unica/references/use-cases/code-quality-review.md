# Code Quality, Review, Refactoring, And Performance

## When to use

Use this when the user asks for code review, refactoring, error fixing,
performance optimization, or standards compliance in BSL code.
Use `api-design` for public API, service interface, overridable module,
versioning, or backward compatibility decisions.

Do not use this as a replacement for metadata or runtime tools. Use it together
with object-specific info tools, source search, syntax checks, and focused tests.

## Primary path

- Inspect metadata shape with `unica.*.info` tools before changing code that
  depends on objects, forms, roles, or reports.
- Use code search/analysis tools through MCP `unica` where available.
- Use `v8-runner` `operation=syntax` for syntax checks and `operation=test` for
  YaXUnit or Vanessa Automation validation.
- Report findings first for reviews, ordered by severity and grounded in file
  references.

## Standards to apply

- Business logic belongs in common modules unless the form lifecycle requires a
  form module.
- Avoid query-in-loop, unnecessary server round trips, hidden broad rights, and
  unbounded selections.
- Keep refactors incremental: map callers, change the smallest coherent unit,
  and run syntax/tests after each meaningful change.

## Related references

- `../platform/development-standards.md`
- `../platform/platform-solutions.md`
- `forms-ui.md`
- `rights-access.md`
