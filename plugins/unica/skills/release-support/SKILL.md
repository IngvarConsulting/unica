---
name: release-support
description: "Поддержка поставки и обновлений 1С. Используй когда нужно проверить сравнение/объединение, поставку, поддержку, расширения, совместимость обновления, миграции данных и release readiness."
---

# Release Support

## MCP routing

- Preferred path: use MCP `unica` tools `unica.project.map`, `unica.code.search`, `unica.cf.info`, `unica.cfe.diff`, `unica.meta.info`, `unica.code.diagnostics`, `unica.standards.search`, `unica.standards.explain`, and `unica.runtime.execute`.
- Use `unica.role.info`, `unica.dcs.info`, or form/meta tools when release risk is localized to rights, reports, forms, or metadata objects.
- Do not call internal package, metadata, analyzer, standards, or runtime adapters directly. They are hidden behind MCP `unica`.

Support-state checks come from `unica.cf.info` and object-level `unica.meta.info`/`unica.form.info`/`unica.dcs.info`/`unica.mxl.info`/`unica.role.info`/`unica.subsystem.info`, which read `Ext/ParentConfigurations.bin` through Unica. Treat `Поддержка: на замке` or read-only as a release decision: prefer CFE or an explicit support-state change plan before direct mutation.

## References

- Read `references/platform/compatibility-modes.md` when an upgrade, migration,
  configuration, or extension change depends on a compatibility mode.
- Read `references/platform/platform-mechanics.md` for platform behavior that affects compatibility and runtime risk.
- Read `references/platform/integration-contracts.md` when release changes public integration/API behavior.
- Read `references/use-cases/code-quality-review.md` for Findings first review output.

## Workflow

1. Identify release scope: vendor update, extension change, merge branch, support-state change, hotfix, migration, or integration contract change.
2. Map source-sets with `unica.project.map`; inspect configuration and extensions with `unica.cf.info`, `unica.cfe.diff`, `unica.meta.info`, and `unica.code.search`.
3. List compatibility risks: metadata rename/delete, changed roles, changed integration contracts, data migrations, scheduled jobs, query behavior, BSP hooks, and extension interceptors.
4. Run `unica.code.diagnostics`; then use `unica.runtime.execute` for syntax/tests/build/update verification as the project allows.
5. Produce a release readiness note: blocking findings, migration steps, rollback boundary, manual checks, and Unica MCP contract gaps.

## Review checklist

- Поставка и поддержка are explicit release decisions, not hidden in generated churn.
- Public APIs and exchange contracts remain backward compatible or have a migration note.
- Extension interceptors still bind to borrowed methods after update.
- Data migrations are idempotent and restartable.
- Tests cover changed business paths, integration paths, and update-only paths.

## Stop rules

- Do not mark release ready when syntax/tests/update checks were not run; say exactly what is missing.
- Do not hide compatibility risk behind a generic code review. Lead with blocking release findings.
