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

Do not use legacy top-level `connection`; the current schema stores the connection under `infobase.connection`.

Backend guidance:

- Designer format with Designer builder covers init/build/extensions/dump/syntax/tests/make/load.
- Designer format with IBCMD supports file infobases and server infobases when `infobase.dbms.kind/server/name` are configured.
- EDT format can build, run EDT syntax checks, synchronize extensions, and run configured tests, but `syntax edt` uses `projects` rather than Designer module flags.
- `convert` is a file workflow and accepts only `sourceSet` and `output` from the Unica wrapper.
- `make` requires a backend that can publish the requested artifact. For external processors/reports, `output` is a publish directory.
- `load` accepts `mode=load` or `mode=merge`; `mode=merge` requires `settings`. `mode=update` is rejected by v8-runner.
