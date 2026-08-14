# Rights And Access

## When to use

Use this when the user needs to inspect, create, validate, or audit roles,
object rights, RLS restrictions, templates, or least-privilege access for code
that touches metadata objects.

Do not use this for OS/user administration or infobase authentication recovery.
Use `db-auth-check` only to classify already supplied credential/license
evidence; it does not probe the infobase.

- По INV-MCP-RUNTIME-RECEIPT текущий runtime-контракт: `unica.runtime.execute` — preview-only и вызывается только с `dryRun: true`; любой applied-режим возвращает fail-closed до workspace discovery и process spawn. Preview не является runtime verification. Не обходи этот отказ прямым runner-ом, через `unica.build.*` или fallback через `unica.runtime.job.*`.

## Primary path

Use native role tools through MCP `unica`:

- `unica.role.info`
- `unica.role.compile`
- `unica.role.validate`

When code changes require new rights, inspect the touched metadata objects and
compile focused role definitions rather than broad presets.

## Related references

- `../specs/1c-role-spec.md`
- `../specs/role-dsl-spec.md`
- `../platform/development-standards.md`
