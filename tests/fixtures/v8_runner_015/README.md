# Published runner envelopes: a restored module

Captured 2026-10-14 from the published macOS aarch64 archive of
IngvarConsulting/v8-runner-rust v0.15.1 (`v8-runner-macos-aarch64.tar.gz`,
archive SHA-256 `f28b045672ac89ff963824982efee94485f03abe892e75128cdde0791fd8f7c4`),
source commit `e3160e8e843c54c642ac82be80c79a0bf93e03ab`, binary SHA-256
`bda97aa5a5fa1d7e95145d1ab68a7cf4271dd1f0526a33189475f1016acd3ebb`.
Platform: 1C 8.3.27.2074 (`tools.platform.path` in the local layer). Every
command used `--config <project>/v8project.yaml --json-message`. The envelopes
carry no machine paths; they are captured bytes.

These fixtures attest the runner boundary, not a portable guarantee that every
host has a working platform installation.

## The scenario of issue #1272

Project file: `providers: {push: ibcmd}` and one `CONFIGURATION` source set
`main` with the configuration of OpenSubsystemsLibrary; the file infobase was
declared in `v8project.local.yaml` as `infobases.origin` and created by the same
runner (`infobase create`). Then:

1. A byte copy of `DataProcessors/HTTPResponse/Ext/ObjectModule.bsl` was saved
   with `cp -p`, keeping its modification time.
2. `push-temporary.json`: after a temporary line was appended to the module,
   `push` loaded `main` partially.
3. The saved copy was restored with `cp -p`: other bytes than the runner's
   memory, the old modification time.
4. `push-restored-preview.json`, `push-restored.json`: `push --dry-run` and
   `push` plan and load `main` partially, with `applied: true` after the load.
5. `push-again.json`: `push` once more — `main` is `skipped`, nothing changed.

`ibcmd infobase config export` of the infobase afterwards held the restored
module without the temporary line. The same scenario with the published
v0.14.0 archive answered `skipped` / `no changes` at step 4, and the exported
infobase still held the temporary line.
