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

Runtime идёт через `unica.run`: вызов без `op` отдаёт словарь операций и
контракт каждой — `argsSchema`, `execution`, `previewRequired`,
`ifRevRequiredOnApply`. Контракт вызова бери оттуда, а не из этого текста;
выбирай только операцию с `implemented: true` и не выдумывай аргументов
записи с `argsSchema: null`; превью исполнением не является. Не обходи
контракт прямым runner-ом.

- Run object-specific validation after metadata changes.
- Check syntax with `unica.check`; test runs are outside the v0.13 surface,
  so retain an explicit residual risk: a static check does not validate BSL in
  the runtime.
- For risky changes, inspect metadata shape before and after the edit.
