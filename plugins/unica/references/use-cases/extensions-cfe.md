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

Текущий runtime-контракт: `unica.runtime.execute` — preview-only и вызывается
только с `dryRun: true`; любой applied-режим возвращает fail-closed до
workspace discovery и process spawn. Preview не является runtime verification.
Не обходи этот отказ прямым runner-ом, через `unica.build.*` или fallback через `unica.runtime.job.*`.

## Related references

- `../specs/1c-extension-spec.md`
- `workspace-runtime.md`
