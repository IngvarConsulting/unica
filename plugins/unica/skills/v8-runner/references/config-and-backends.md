# Config And Backends

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

For a fallback-capable runtime build, Unica binds the presence and exact bytes
of this sibling overlay together with the primary config. Do not create, remove,
or rewrite it between the normal attempt and the possible full retry: changing
it can select a different work directory, infobase, or executable.

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

Backend guidance:

- Designer format with Designer builder covers init/build/extensions/dump/syntax/tests/make/load.
- Designer format with IBCMD supports file infobases and server infobases when `infobase.dbms.kind/server/name` are configured.
- EDT format can build, run EDT syntax checks, synchronize extensions, and run configured tests, but `syntax edt` uses `projects` rather than Designer module flags.
- `convert` is a file workflow and accepts only `sourceSet` and `output` from the Unica wrapper.
- `make` requires a backend that can publish the requested artifact. For external processors/reports, `output` is a publish directory.
- `load` accepts `mode=load` or `mode=merge`; `mode=merge` requires `settings`. `mode=update` is rejected by v8-runner.
