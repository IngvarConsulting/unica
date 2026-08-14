# Config And Backends

- Текущий runtime-контракт: `unica.runtime.execute` — preview-only и вызывается только с `dryRun: true`; любой applied-режим возвращает fail-closed до workspace discovery и process spawn. Preview не является runtime verification. Не обходи этот отказ прямым runner-ом, через `unica.build.*` или fallback через `unica.runtime.job.*`.

Important `v8project.yaml` concepts:

- `format`: `designer` or `edt`.
- `builder`: `DESIGNER` or `IBCMD`.
- `execution_timeout`: runner operation budget in milliseconds; default `300000`, valid range `1..=86400000`.
- `source-set`: ordered configuration and extension source entries.
- `basePath` is not supported; relative paths are resolved from the primary config directory.
- `infobase.connection`: runtime connection string.
- `infobase.dbms`: required for IBCMD server infobases and invalid for file infobases.
- `tools.client_mcp.extension`: optional generated tool extension prepared by `build`.
- external source-set types: `EXTERNAL_DATA_PROCESSORS` publishes `.epf`, `EXTERNAL_REPORTS` publishes `.erf`.

Use `v8project.local.yaml` for local `workPath`, `infobase`, `tools`, `tests`, and `mcp` values. Do not pass it as the MCP `config` argument. Do not put shared `source-set`, `format`, `builder`, or `execution_timeout` there.

Do not use legacy top-level `connection`; the current schema stores the connection under `infobase.connection`.

For an opt-in fail-closed platform binding, put the machine-specific path in
`v8project.local.yaml` when appropriate:

```yaml
tools:
  platform:
    version: "8.3.27.1859"
    path: "C:\\Program Files\\1cv8\\8.3.27.1859\\bin"
    strict: true
```

The key is `tools.platform.strict`. A configured `path` is always an
explicit-only boundary. With `strict: true`, the runner enforces the configured
version, rejects missing utilities and unknown/incompatible versions, and pins
`1cv8`, `1cv8c`, and `ibcmd` to one canonical installation root. With
`strict: false` or no `strict`, a configured `path` remains the boundary but
its `version` is ignored. When `path` is absent, omitted/false `strict`
preserves normal root/`PATH` discovery; `strict: true` alone adds no boundary.
This is project configuration, not a new `unica.runtime.execute` argument.

Backend argument guidance for previews; these capabilities do not make the
current public applied modes executable:

- Designer format with Designer builder defines arguments for init/build/extensions/dump/syntax/tests/make/load previews.
- Designer format with IBCMD models file infobases and server infobases when `infobase.dbms.kind/server/name` are configured.
- EDT format models build, EDT syntax, extension, and configured-test previews; `syntax edt` uses `projects` rather than Designer module flags.
- `convert` is a file workflow and accepts only `sourceSet` and `output` from the Unica wrapper.
- A future applied `make` requires a backend that can publish the requested artifact. In its preview for external processors/reports, `output` is a publish directory.
- `load` accepts `mode=load` or `mode=merge`; `mode=merge` requires `settings`. `mode=update` is rejected by v8-runner.
