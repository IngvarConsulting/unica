# Published runner envelopes

Captured 2026-10-07 from the published macOS aarch64 archive of
IngvarConsulting/v8-runner-rust v0.13.0 (`v8-runner-macos-aarch64.tar.gz`,
archive SHA-256 `70972d94a16cdfc23063e6f46aabf6296f7f5ff031cf936e3b0b39b6b3b16a2b`),
source commit `e0b93d4114f2c5f7f5ea3b63ad08e3deaadd824e`, binary SHA-256
`d586d843e4c58160ce28af6f969e2ae3cd094dd9777a22d863ffc500497b8d1f`.
Platform: 1C 8.3.27.2074. Every command used
`--config <project>/v8project.yaml --json-message`, and each infobase was an
isolated file infobase created by the same runner (`infobase create`).
Machine-specific prefixes were replaced: the capture directory by `/workspace`,
the platform directory by `/platform`, the host name by `dev-host`. These
replacements touch prose and path fields alike; everything else is captured
bytes, re-indented.

These fixtures attest the runner boundary, not a portable guarantee that every
host has a working platform installation.

## Extensions

Project file: `providers: {extensions: ibcmd, infobase.create: ibcmd}` and one
`CONFIGURATION` source set; the infobase was declared in
`v8project.local.yaml` as `infobases.origin`. Each command ran first with
`--dry-run`, then without:

- `extensions create --name UnicaRunnerProbe --name-prefix URP_ --purpose patch`
- `extensions info --name UnicaRunnerProbe`
- `extensions activate --name UnicaRunnerProbe --active no`
- `extensions activate --name UnicaRunnerProbe --active yes`
- `extensions delete --name UnicaRunnerProbe`
- `extensions list` (empty after deletion)

Separate `info` calls verified false/true activity after each change. The
shapes match the 0.12.0 captures they replace.

## Push against another working copy

Provider designer. Project `A` created the infobase, pulled it (`pull --force`)
and pushed a change. Project `B` declared the same infobase by its absolute
path; both declared it with `shared: true`. Then:

- `push-infobase-held-preview.json`: `push --dry-run` in `B` before
  `shared: true` was set — `infobase_held`, the base is held by `A`.
- `push-no-memory-preview.json`: `push --dry-run` in `B` after both copies
  shared the base — `no_memory`, `B` has no memory of the base.
- `B` ran `push --force` (full load into the shared infobase).
- `push-skipped-preview.json`, `push-skipped-apply.json`: `push --dry-run` and
  `push` in `A` — every set `skipped` by `A`'s memory; the platform was not
  started and the generation was not read, although the infobase held `B`'s
  configuration (issue #1240).
- `push-full-preview.json`, `push-non-fast-forward-apply.json`:
  `push --full --dry-run` and `push --full` in `A` — the preview plans a full
  load; the apply reads the generation and refuses `non_fast_forward` with both
  generations.
- `push-infobase-busy-apply.json`: `push --full` in `B` while `A` ran
  `pull --force` on the same infobase — `infobase_busy`.
