# Rights And Access

## When to use

Use this when the user needs to inspect, create, validate, or audit roles,
object rights, RLS restrictions, templates, or least-privilege access for code
that touches metadata objects.

Do not use this for OS/user administration or infobase authentication recovery.
Use `db-auth-check` only to classify already supplied credential/license
evidence; it does not probe the infobase.

- По INV-MCP-RUNTIME-RECEIPT и ADR-0074: `unica.runtime.execute` с `dryRun: true`
показывает запланированную команду без побочных эффектов, а с `dryRun: false`
исполняет классифицированную операцию и отвечает её терминальным результатом в
том же вызове, приложив названную причину риска (`runtime_risk_*`)
предупреждением; неклассифицированная операция по-прежнему отказывает
`runtime_operation_unbounded` до обнаружения рабочего пространства. Preview
исполнением не является. Работу, которую вызов ждать не должен, запускай через
`unica.runtime.job.start`. Не обходи контракт прямым runner-ом или через
`unica.build.*`.

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
