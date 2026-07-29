---
name: data-exchange
description: "Обмен данными 1С. Используй когда нужно реализовать или диагностировать планы обмена, РИБ, регистрацию изменений, загрузку/выгрузку сообщений, конфликт данных или контракт обмена."
---

# Data Exchange

## MCP routing

- Preferred path: use MCP `unica` tools `unica.project.map`, `unica.code.search`, `unica.meta.info`, `unica.code.diagnostics`, `unica.standards.search`, `unica.standards.explain`, and `unica.runtime.execute`.
- Use `unica.role.info` when exchange behavior depends on rights, privileged mode, or separated data.
- Do not call internal metadata, runtime, analyzer, standards, or package adapters directly. They are hidden behind MCP `unica`.

## References

- Read `../../references/platform/integration-contracts.md` for message contracts, file exchange, idempotency, versioning, and error semantics.
- Read `../../references/platform/platform-mechanics.md` for exchange registration, tenant boundaries, background workers, and temporary files.
- Read `../../references/platform/runtime-diagnostics.md` for ЖР/ТЖ timeline when exchange fails at runtime.

## Workflow

1. Identify the exchange model: plan exchange, РИБ, file batch, queue, direct API, or hybrid integration.
2. Inspect metadata and modules with `unica.meta.info` and `unica.code.search`: plans, nodes, registration rules, message numbers, loading handlers, and conflict resolution.
3. Define the exchange contract: node identity, external ids, schema version, ordering, idempotency, retry behavior, duplicate detection, and compatibility rules.
4. Check change registration deliberately: keep регистрация изменений explicit, state which objects are registered, when registration is suppressed, how deletes are represented, and how retries avoid double writes.
5. Verify syntax/tests with `unica.runtime.execute`; for live failures, collect ЖР/ТЖ and correlate message id, node, user/session, and object ids.

## Review checklist

- Message processing is restartable and idempotent.
- Errors distinguish validation, duplicate, incompatible version, missing reference, and temporary transport failure.
- Conflict resolution is explicit and logged.
- Exchange does not bypass rights or tenant boundaries accidentally.
- File names, encoding, schema version, and partial-write behavior are validated before loading.

## Contract gaps

If public MCP `unica` cannot expose exchange runtime state, message queue state, or registration details required for the task, report a Unica MCP contract gap.
