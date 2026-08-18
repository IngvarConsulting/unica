# 1C Development Standards

Use these standards during BSL implementation, review, and refactoring.

## Architecture

- Put reusable business logic in common modules.
- Keep form modules focused on UI lifecycle, event handlers, and client/server
  orchestration.
- Keep integration boundary code separate from domain logic.
- Prefer small exported procedures/functions with explicit input contracts.

## Forms

- Avoid unnecessary client/server round trips.
- Add event hooks in both `Form.xml` and the module procedure/function.
- Keep form commands and attributes aligned with the form XML.
- Do not use modal UI calls unless the target client mode explicitly supports
  them.

## Naming And Comments

- See `metadata-conventions.md` for object naming, synonym, representation, and
  fill-check conventions.
- Use project-local naming conventions when present.
- Add comments for non-obvious platform constraints and integration decisions,
  not for trivial assignments.
- Keep modification comments consistent with the project baseline.

## Validation

По INV-MCP-RUNTIME-RECEIPT и ADR-0074: `unica.runtime.execute` с `dryRun: true`
показывает запланированную команду без побочных эффектов, а с `dryRun: false`
исполняет классифицированную операцию и отвечает её терминальным результатом в
том же вызове, приложив названную причину риска (`runtime_risk_*`)
предупреждением; неклассифицированная операция по-прежнему отказывает
`runtime_operation_unbounded` до обнаружения рабочего пространства. Preview
исполнением не является. Работу, которую вызов ждать не должен, запускай через
`unica.runtime.job.start`. Не обходи контракт прямым runner-ом или через
`unica.build.*`.

- Run object-specific validation after metadata changes.
- Use `v8-runner` with `dryRun: true` to preview `unica.runtime.execute`
  syntax/test arguments and with `dryRun: false` to run them; after a preview
  alone, retain an explicit residual risk because preview does not validate BSL
  in the runtime.
- For risky changes, inspect metadata shape before and after the edit.
