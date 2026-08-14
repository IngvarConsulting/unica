# Integrations

## When to use

Use this when the user needs HTTP services, REST clients, web services, file
exchange, message queues, webhooks, or OpenSpec-backed integration changes.

Do not start with transport code. First identify the business object, data
contract, error handling policy, authentication requirements, and where the
integration belongs in the 1C architecture.

## Primary path

По INV-MCP-RUNTIME-RECEIPT текущий runtime-контракт: `unica.runtime.execute` — preview-only и вызывается
только с `dryRun: true`; любой applied-режим возвращает fail-closed до
workspace discovery и process spawn. Preview не является runtime verification.
Не обходи этот отказ прямым runner-ом, через `unica.build.*` или fallback через `unica.runtime.job.*`.

- Use metadata tools to inspect or create HTTP services, common modules,
  constants, catalogs, documents, and registers needed by the integration.
- Use BSL source edits for modules and handlers.
- Use `v8-runner` only to preview `unica.runtime.execute`
  `operation=syntax`/`operation=test` arguments with `dryRun: true`; report
  syntax, tests, and integration runtime behavior as unverified without
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
