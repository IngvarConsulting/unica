# Rights And Access

## When to use

Use this when the user needs to inspect, create, validate, or audit roles,
object rights, RLS restrictions, templates, or least-privilege access for code
that touches metadata objects.

Do not use this for OS/user administration or infobase authentication recovery.
Use `db-auth-check` only for safe credential/license classification before
runtime operations.

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
