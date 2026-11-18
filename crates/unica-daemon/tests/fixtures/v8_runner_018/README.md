# Runner 0.18.0 envelopes: sessions that hold a cluster infobase

Captured 2026-11-19 on Windows from the published release binary of v8-runner
`v0.18.0` (IngvarConsulting/v8-runner-rust `98bd362e`):
`v8-runner-windows-x86_64.zip`, archive SHA-256
`33ad05c9028ae37b57a8dbe3c88fd5a77d03d1a03d3ac73021cd8649f42b82a0`, `v8-runner.exe`
SHA-256 `9c9cfa20b26332ffca5ce751f2dce6485095598c75ab863c007d7be91822facc`. The stand
directory is replaced by `C:\workspace` and the host name by `workstation`;
every other byte is as captured.

The working copy has `format: DESIGNER` and one `CONFIGURATION` set `main` in
`src/cf`. The infobase `origin` is the runner's managed cluster of platform
8.3.27.2342 (`cluster.managed`, `dbms.managed`), created empty by
`infobase create` and dumped by `pull main --force`. A thin client opened by
`launch thin` holds it while the commands run.

- `push-sessions-refused.json`: `push main --full` after adding a constant —
  `infobase_sessions`, `terminable: true`, exit 3. The set is loaded into the
  main configuration but not applied.
- `apply-sessions-refused.json`: `apply main` right after — the same refusal of
  `apply`, exit 3.
- `apply-sessions-force.json`: `apply main --sessions force` — the session is
  ended and named in `data.sessions.terminated`, exit 0.
- `push-sessions-force.json`: `push main --sessions force` after removing the
  constant and opening a new thin client — the session is ended, exit 0.

Each session carries the 0.18 shape: `number` is the platform number, `id` the
session UUID when the server reports it (`null` here), and the new
`from_this_machine` and `excluded`.
