# Integrations

## When to use

Use this when the user needs HTTP services, REST clients, web services, file
exchange, message queues, webhooks, or OpenSpec-backed integration changes.

Do not start with transport code. First identify the business object, data
contract, error handling policy, authentication requirements, and where the
integration belongs in the 1C architecture.

## Primary path

Runtime идёт через `unica.run`: вызов без `op` отдаёт словарь операций и
контракт каждой — `argsSchema`, `execution`, `previewRequired`,
`ifRevRequiredOnApply`. Контракт вызова бери оттуда, а не из этого текста;
при `implemented: true` используй опубликованную `argsSchema`; при
`support.state: limited` разрешено только подмножество `support.supportedArgs`.
При `support.state: unavailable` остановись; не выдумывай аргументов при
`argsSchema: null`. Превью исполнением не является. Не обходи контракт прямым runner-ом.

- Use metadata tools to inspect or create HTTP services, common modules,
  constants, catalogs, documents, and registers needed by the integration.
- Use BSL source edits for modules and handlers.
- Check syntax with `unica.check`; test runs are outside the v0.13 surface,
  so report tests and integration runtime behavior as unverified without
  separate execution evidence.
- For OpenSpec work, keep proposal/spec artifacts in the project’s chosen spec
  workspace and link implementation tasks to those artifacts.

## Standards to apply

- Set connection and read timeouts explicitly.
- Normalize external payloads at the boundary.
- Keep secrets in local config or secure storage, not committed source files.
- Log enough context to diagnose failures without logging credentials or full
  personal data payloads.

## Related references

- `../platform/development-standards.md`
- `metadata-modeling.md`
- `code-quality-review.md`
