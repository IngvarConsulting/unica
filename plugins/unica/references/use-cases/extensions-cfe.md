# Extensions CFE

## When to use

Use this when the user needs to create a configuration extension, validate it,
borrow configuration objects into it, inspect its differences, or generate a
method interceptor.

Do not use this for ordinary metadata object edits in the base configuration.
Use metadata-modeling references and `unica.meta.*` for that.

## Primary path

Use native CFE tools through MCP `unica`:

- `unica.cfe.init`
- `unica.cfe.validate`
- `unica.cfe.diff`
- `unica.cfe.borrow`
- `unica.cfe.patch_method`

Runtime export or loading of `.cfe` artifacts is handled by `v8-runner`
only as a preview of `unica.runtime.execute` with `operation=make` or
`operation=load` and explicit `dryRun: true`.

По INV-MCP-RUNTIME-RECEIPT и ADR-0074: `unica.runtime.execute` с `dryRun: true`
показывает запланированную команду без побочных эффектов, а с `dryRun: false`
исполняет операцию и отвечает её терминальным результатом в том же вызове,
приложив названную причину риска (`runtime_risk_*`) предупреждением. Preview
исполнением не является. Работу, которую вызов ждать не должен, запускай через
`unica.runtime.job.start`. Не обходи контракт прямым runner-ом или через
`unica.build.*`.

## Related references

- `../specs/1c-extension-spec.md`
- `workspace-runtime.md`
