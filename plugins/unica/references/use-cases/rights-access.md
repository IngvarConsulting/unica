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
`ifRevRequiredOnApply`. Контракт вызова бери оттуда, а не из этого текста;
при `implemented: true` используй опубликованную `argsSchema`; при
`support.state: limited` разрешено только подмножество `support.supportedArgs`.
При `support.state: unavailable` остановись; не выдумывай аргументов при
`argsSchema: null`. Превью исполнением не является. Не обходи контракт прямым runner-ом.

## Primary path

Use native role tools through MCP `unica`:

- `unica.view` on the role node
- `unica.role.compile`
- `unica.check` on the role node (validator `role`)

When code changes require new rights, inspect the touched metadata objects and
compile focused role definitions rather than broad presets.

## Related references

- `../specs/1c-role-spec.md`
- `../specs/role-dsl-spec.md`
- `../platform/development-standards.md`
