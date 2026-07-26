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

- Run object-specific validation after metadata changes.
- Run `v8-runner` syntax/tests for BSL changes.
- For risky changes, inspect metadata shape before and after the edit.
