# Rights And Access

## When to use

Use this when the user needs to inspect, create, validate, or audit roles,
object rights, RLS restrictions, templates, or least-privilege access for code
that touches metadata objects.

Do not use this for OS/user administration or infobase authentication recovery.
Use `db-auth-check` only to classify already supplied credential/license
evidence; it does not probe the infobase.

- Runtime идёт через `unica.run`: вызов без `op` отдаёт словарь операций и
контракт каждой — `argsSchema`, `execution`, `previewRequired`,
`dryRunRequired`. Контракт вызова бери оттуда, а не из этого текста;
при `implemented: true` используй опубликованную `argsSchema`; при
`support.state: limited` разрешено только подмножество `support.supportedArgs`.
При `support.state: unavailable` остановись; не выдумывай аргументов при
`argsSchema: null`. Для плановой операции сначала проверь результат `dryRun: true`,
затем исполняй запрос с `dryRun: false`. Preview не фиксирует входы между
вызовами. Не обходи контракт прямым runner-ом.

## Primary path

Use the canonical role operations through MCP `unica` (skill `role-edit`):

- `unica.view` on the role node and its `Right` branch
- `unica.apply` with `role.create` at `<set>:Configuration`, `right.add` on the
  role and `right.set` on a role entry; preview first, then execute the saved
  plan by `executionToken`
- `unica.check` on the role node (validator `role`)

When code changes require new rights, inspect the touched metadata objects and
grant focused rights per object rather than broad typical sets.

## Related references

- `../specs/1c-role-spec.md`
- `../platform/development-standards.md`
