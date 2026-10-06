# Published runner extension envelopes

Captured 2026-10-06 from the published macOS aarch64 archive of
IngvarConsulting/v8-runner-rust v0.12.0 (`v8-runner-macos-aarch64.tar.gz`),
source commit `167e6cbda52452f00d31539367958b359e0853d1`, binary SHA-256
`4c0d3cee2a04e725651af4c68584a93331e6e6d82bfd14bad27a3916e2a5f6fe`.
Platform: 1C 8.3.27.2074, provider ibcmd, an isolated empty file infobase
created by the same runner (`infobase create`, provider ibcmd).
Only machine-specific workspace/platform prefixes in prose were replaced by
`/workspace` and `/platform`; structured fields and receipts are captured bytes.

The project file declared `providers.extensions: ibcmd` and the source set;
the infobase was declared in `v8project.local.yaml` as `infobases.origin`.
Every command used `--config <isolated>/v8project.yaml --json-message`.
Each ran first with `--dry-run`, then without:

- `extensions create --name UnicaRunnerProbe --name-prefix URP_ --purpose patch`
- `extensions info --name UnicaRunnerProbe`
- `extensions activate --name UnicaRunnerProbe --active no`
- `extensions activate --name UnicaRunnerProbe --active yes`
- `extensions delete --name UnicaRunnerProbe`
- `extensions list` (empty after deletion)

Separate `info` calls verified false/true activity after each change.
These fixtures attest the runner boundary, not a portable guarantee that every
host has a working platform installation.
