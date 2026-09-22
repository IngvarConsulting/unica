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
`download` with `extension` writes the `.cfe` from the infobase, `upload`
with `extension` loads the working configuration without applying it to the database;
`apply` with that extension applies it, and `reset` with `force:true` discards
pending changes. `make` builds an artifact from sources; each operation is
previewed first and applied with the `ifRev` the preview returned.

Runtime идёт через `unica.run`: вызов без `op` отдаёт словарь операций и
контракт каждой — `argsSchema`, `execution`, `previewRequired`,
`ifRevRequiredOnApply`. Контракт вызова бери оттуда, а не из этого текста;
при `implemented: true` используй опубликованную `argsSchema`; при
`support.state: limited` разрешено только подмножество `support.supportedArgs`.
При `support.state: unavailable` остановись; не выдумывай аргументов при
`argsSchema: null`. Превью исполнением не является. Не обходи контракт прямым runner-ом.

## Related references

- `../specs/1c-extension-spec.md`
- `workspace-runtime.md`
