# Autonomous Server And Web Client Debug

## When to use

Use this when the user needs a local isolated 1C debug contour for HTTP
services, web services, web-client checks, client MCP automation, or runtime
artifact analysis.

Do not use this for production deployment. Do not introduce a separate web
server deployment skill surface; runtime setup must stay behind MCP `unica`.

## Primary path

- `autonomous-server` prepares and analyzes the isolated runtime contour.
- `v8-runner` calls MCP `unica.runtime.execute` for `config-init`, `init`,
  `build`, `syntax`, and `launch`.
- A concrete web-client URL is the hand-off point for an external browser-testing tool.
- `log-analysis` analyzes journal registration and technological log evidence.

If no public MCP `unica` operation can produce the required debug URL or server
state, report that as a Unica MCP contract gap instead of bypassing the public
boundary.

## Related references

- `../tooling/v8project.md`
- `../tooling/runtime-build.md`
- `../specs/web-spec.md`
