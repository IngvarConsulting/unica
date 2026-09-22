# Autonomous Server And Web Client Debug

## When to use

Use this when the user needs a local isolated 1C debug contour for HTTP
services, web services, web-client checks, client MCP automation, or runtime
artifact analysis.

Do not use this for production deployment. Do not introduce a separate web
server deployment skill surface; runtime setup must stay behind MCP `unica`.

## Primary path

Runtime идёт через `unica.run`: вызов без `op` отдаёт словарь операций и
контракт каждой — `argsSchema`, `execution`, `previewRequired`,
`ifRevRequiredOnApply`. Контракт вызова бери оттуда, а не из этого текста;
при `implemented: true` используй опубликованную `argsSchema`; при
`support.state: limited` разрешено только подмножество `support.supportedArgs`.
При `support.state: unavailable` остановись; не выдумывай аргументов при
`argsSchema: null`. Превью исполнением не является. Не обходи контракт прямым runner-ом.

- `autonomous-server` prepares and analyzes the isolated runtime contour.
- `unica.run` prepares the contour step by step: `infobase.create`,
  `push`, then `launch`; creation and source push are unavailable with runner 0.11. Use an already prepared infobase; each supported previewApply step is previewed first
  and applied with its `ifRev`. Web publication and an MCP client mode are not
  on the v0.13 surface.
- A concrete web-client URL supplied independently by the user is the hand-off
  point for an external browser-testing tool. Preview cannot produce one.
- `log-analysis` analyzes journal registration and technological log evidence.

Report the unavailable debug URL and server state as a Unica MCP contract gap;
do not bypass the public boundary.

## Related references

- `../tooling/v8project.md`
- `../tooling/runtime-build.md`
- `../specs/web-spec.md`
