---
name: release-support
description: "Поддержка поставки и обновлений 1С. Используй когда нужно проверить сравнение/объединение, поставку, поддержку, расширения, совместимость обновления, миграции данных и release readiness."
---

# Release Support

## MCP routing

- Preferred path: use MCP `unica` tools `unica.view {}`, `unica.code.search`, `unica.diff` between the extension and configuration sets, `unica.meta.info`, `unica.code.diagnostics`, `unica.docs`, and `unica.runtime.execute`.
- По INV-MCP-RUNTIME-RECEIPT и ADR-0074: `unica.runtime.execute` с `dryRun: true`
показывает запланированную команду без побочных эффектов, а с `dryRun: false`
исполняет классифицированную операцию и отвечает её терминальным результатом в
том же вызове, приложив названную причину риска (`runtime_risk_*`)
предупреждением; неклассифицированная операция по-прежнему отказывает
`runtime_operation_unbounded` до обнаружения рабочего пространства. Preview
исполнением не является. Работу, которую вызов ждать не должен, запускай через
`unica.runtime.job.start`. Не обходи контракт прямым runner-ом или через
`unica.build.*`.
- Use `unica.view` on the role node, `unica.view` on the schema node, or form/meta tools when release risk is localized to rights, reports, forms, or metadata objects.
- Do not call internal package, metadata, analyzer, standards, or runtime adapters directly. They are hidden behind MCP `unica`.

Support-state checks come from `unica.view` on the configuration root (`support`) and object-level `unica.meta.info`, `unica.mxl.info`, and `unica.view` on the object node, which read `Ext/ParentConfigurations.bin` through Unica. Treat `Поддержка: на замке` or read-only as a release decision: prefer CFE or an explicit support-state change plan before direct mutation.

## References

- Read `../../references/platform/compatibility-modes.md` when an upgrade, migration,
  configuration, or extension change depends on a compatibility mode.
- Read `../../references/platform/platform-mechanics.md` for platform behavior that affects compatibility and runtime risk.
- Read `../../references/platform/integration-contracts.md` when release changes public integration/API behavior.
- Read `../../references/use-cases/code-quality-review.md` for Findings first review output.

## Workflow

1. Identify release scope: vendor update, extension change, merge branch, support-state change, hotfix, migration, or integration contract change.
2. Map source-sets with `unica.view {}`; inspect the configuration root with `unica.view <set>:Configuration`, extensions with `unica.diff` between the extension and configuration sets, `unica.meta.info`, and `unica.code.search`.
3. List compatibility risks: metadata rename/delete, changed roles, changed integration contracts, data migrations, scheduled jobs, query behavior, BSP hooks, and extension interceptors.
4. Run `unica.code.diagnostics`; then use `unica.runtime.execute` only to preview typed syntax/test/build/update arguments and record all runtime checks as unverified unless separate evidence is supplied.
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
