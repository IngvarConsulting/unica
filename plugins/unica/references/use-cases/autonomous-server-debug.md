# Autonomous Server And Web Client Debug

## When to use

Use this when the user needs a local isolated 1C debug contour for HTTP
services, web services, web-client checks, client MCP automation, or runtime
artifact analysis.

Do not use this for production deployment. Do not introduce a separate web
server deployment skill surface; runtime setup must stay behind MCP `unica`.

## Primary path

По INV-MCP-RUNTIME-RECEIPT и ADR-0074: `unica.runtime.execute` с `dryRun: true`
показывает запланированную команду без побочных эффектов, а с `dryRun: false`
исполняет классифицированную операцию и отвечает её терминальным результатом в
том же вызове, приложив названную причину риска (`runtime_risk_*`)
предупреждением; неклассифицированная операция по-прежнему отказывает
`runtime_operation_unbounded` до обнаружения рабочего пространства. Preview
исполнением не является. Работу, которую вызов ждать не должен, запускай через
`unica.runtime.job.start`. Не обходи контракт прямым runner-ом или через
`unica.build.*`.

- `autonomous-server` prepares and analyzes the isolated runtime contour.
- `v8-runner` previews MCP `unica.runtime.execute` arguments for `config-init`,
  `init`, `build`, `syntax`, and `launch`; each preview explicitly keeps
  `dryRun: true` and does not prepare or launch the contour.
- A concrete web-client URL supplied independently by the user is the hand-off
  point for an external browser-testing tool. Preview cannot produce one.
- `log-analysis` analyzes journal registration and technological log evidence.

Report the unavailable debug URL and server state as a Unica MCP contract gap;
do not bypass the public boundary.

## Related references

- `../tooling/v8project.md`
- `../tooling/runtime-build.md`
- `../specs/web-spec.md`
