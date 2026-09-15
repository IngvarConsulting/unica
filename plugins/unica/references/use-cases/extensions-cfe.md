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
- `unica.check` on the extension root (`ext:Configuration`, validator `cfe`)
- `unica.diff` between the extension and configuration sets
- `unica.cfe.borrow`
- `unica.cfe.patch_method`

Runtime export or loading of `.cfe` artifacts goes through `unica.run`:
`cf.export` with `extension` writes the `.cfe` from the infobase, `cf.import`
with `extension` loads it, and `artifact.build` builds it from sources; each is
previewed first and applied with the `ifRev` the preview returned.

Runtime идёт через `unica.run`: вызов без `op` отдаёт словарь операций и
контракт каждой — `argsSchema`, `execution`, `previewRequired`,
`ifRevRequiredOnApply`. Контракт вызова бери оттуда, а не из этого текста;
выбирай только операцию с `implemented: true` и не выдумывай аргументов
записи с `argsSchema: null`; превью исполнением не является. Не обходи
контракт прямым runner-ом.

## Related references

- `../specs/1c-extension-spec.md`
- `workspace-runtime.md`
