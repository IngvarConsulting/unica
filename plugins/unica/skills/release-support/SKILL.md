---
name: release-support
description: "Поддержка поставки и обновлений 1С. Используй когда нужно проверить сравнение/объединение, поставку, поддержку, расширения, совместимость обновления, миграции данных и release readiness."
---

# Release Support

## MCP routing

- Preferred path: use MCP `unica` tools `unica.view {}`, `unica.search`, `unica.diff` between the extension and configuration sets, `unica.view` on the object node, `unica.check`, `unica.docs`, and `unica.run`.
- Runtime идёт через `unica.run`: вызов без `op` отдаёт словарь операций и
контракт каждой — `argsSchema`, `execution`, `previewRequired`,
`ifRevRequiredOnApply`. Контракт вызова бери оттуда, а не из этого текста;
при `implemented: true` используй опубликованную `argsSchema`; при
`support.state: limited` разрешено только подмножество `support.supportedArgs`.
При `support.state: unavailable` остановись; не выдумывай аргументов при
`argsSchema: null`. Превью исполнением не является. Не обходи контракт прямым runner-ом.
- Use `unica.view` on the role node, `unica.view` on the schema node, or form/meta tools when release risk is localized to rights, reports, forms, or metadata objects.
- Do not call internal package, metadata, analyzer, standards, or runtime adapters directly. They are hidden behind MCP `unica`.

Support-state checks come from `unica.view` on the configuration root (`support`) and on the object node, which read `Ext/ParentConfigurations.bin` through Unica. Treat `Поддержка: на замке` or read-only as a release decision: prefer CFE or an explicit support-state change plan before direct mutation.

## References

- Read `../../references/platform/compatibility-modes.md` when an upgrade, migration,
  configuration, or extension change depends on a compatibility mode.
- Read `../../references/platform/platform-mechanics.md` for platform behavior that affects compatibility and runtime risk.
- Read `../../references/platform/integration-contracts.md` when release changes public integration/API behavior.
- Read `../../references/use-cases/code-quality-review.md` for Findings first review output.

## Workflow

1. Identify release scope: vendor update, extension change, merge branch, support-state change, hotfix, migration, or integration contract change.
2. Map source-sets with `unica.view {}`; inspect the configuration root with `unica.view <set>:Configuration`, extensions with `unica.diff` between the extension and configuration sets, `unica.view` on the object node, and `unica.search`.
3. List compatibility risks: metadata rename/delete, changed roles, changed integration contracts, data migrations, scheduled jobs, query behavior, BSP hooks, and extension interceptors.
4. Run `unica.check` on the changed modules; build and update go through `unica.run` (`make`; `upload` then `apply`, or source `push` with `force:true`) with a preview and its `ifRev`; test runs are outside the v0.13 surface, so record them as unverified unless separate evidence is supplied.
5. Produce a release readiness note: blocking findings, migration steps, rollback boundary, manual checks, and Unica MCP contract gaps.

## Installed extensions

Исходники расширения и расширение, установленное в базе, — разные предметы.
Состав базы спрашивай через `unica.run` с `op: "extensions.list"`, `args: {}`,
`dryRun: true`, затем повтори с `dryRun: false` и полученным `ifRev`.
Для одного расширения выбери запись по имени из результата списка.
Превью платформу не запускает и состав базы не читает; исполнение открывает сеанс.
Поля inventory приходят от платформы, порядок не гарантирован. Префикс имени
читается из имеющихся исходников расширения, в inventory его нет.

`extensions.set` принимает `name` и boolean `active`; остальные свойства
адаптер 0.11 не поддерживает. Удаление — `push` с `args: {"delete": "Имя"}`;
оно удаляет и данные расширения, требует своего preview и `ifRev`.
Режим удаления нельзя совмещать с отправкой исходников. Выключение активности
не равно удалению. Отдельного публичного создания пустого расширения нет:
первая отправка `push` с `force:true` создаёт его из исходников.
`upload` загружает CF/CFE без обновления конфигурации БД; затем нужен
отдельный `apply`. `reset` с `force:true` отбрасывает неприменённое. Не заменяй эту операцию прямым запуском раннера.

## Review checklist

- Поставка и поддержка are explicit release decisions, not hidden in generated churn.
- Public APIs and exchange contracts remain backward compatible or have a migration note.
- Extension interceptors still bind to borrowed methods after update.
- Data migrations are idempotent and restartable.
- Tests cover changed business paths, integration paths, and update-only paths.

## Stop rules

- Do not mark release ready when syntax/tests/update checks were not run; say exactly what is missing.
- Do not hide compatibility risk behind a generic code review. Lead with blocking release findings.
