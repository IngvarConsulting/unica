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

Текущий runtime-контракт: `unica.runtime.execute` — preview-only и вызывается
только с `dryRun: true`; любой applied-режим возвращает fail-closed до
workspace discovery и process spawn. Preview не является runtime verification.
Не обходи этот отказ прямым runner-ом, через `unica.build.*` или fallback через `unica.runtime.job.*`.

- Run object-specific validation after metadata changes.
- Use `v8-runner` only to preview `unica.runtime.execute` syntax/test arguments
  with `dryRun: true`; retain an explicit residual risk because preview does not
  validate BSL in the runtime.
- For risky changes, inspect metadata shape before and after the edit.
