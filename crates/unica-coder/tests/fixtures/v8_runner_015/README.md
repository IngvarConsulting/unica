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

## Creating an infobase, pushes and another working copy

Recaptured with the same runner, platform and configuration in place of the
0.14.0 captures of `tests/fixtures/v8_runner_014`, with `providers: {push:
ibcmd}`: on the capture machine the Designer had no licence and the managed
agent refused the empty infobase user, so the default agent provider of the
0.14.0 captures could not be repeated. Project `E` declared its own file
infobase, project `F` declared `E`'s infobase by its absolute path. Machine
prefixes were replaced: the capture directory by `/workspace`, the platform
directory by `/platform`; everything else is captured bytes.

- `infobase-create-preview.json`, `infobase-create-apply.json`,
  `infobase-create-receipt-preview.json`: `infobase create --dry-run`,
  `infobase create` (ibcmd assembles the infobase with the main configuration
  of `main`), and `infobase create --dry-run` again — refused
  `invalid_argument`: the file infobase exists.
- `push-after-create-preview.json`, `push-after-create-apply.json`: `push
  --dry-run` and `push` — `main` is `skipped`, the platform is not started.
- `push-after-edit-preview.json`, `push-after-edit-apply.json`: after an edit
  of one module, `main` is loaded `partial` with `applied: true`.
- `push-no-memory-preview.json`: `push --dry-run` in `F` — `no_memory`, with
  the warning about the infobase of another working copy.
- `push-other-copy-force-preview.json`, `push-other-copy-force-apply.json`:
  `push --force --dry-run` and `push --force` in `F` — a full load over `E`'s
  configuration with the same warning.
- `push-skipped-preview.json`, `push-skipped-apply.json`: `push --dry-run` and
  `push` in `E` — `skipped` by `E`'s memory although the infobase holds `F`'s
  load (issue #1240).
- `push-full-preview.json`, `push-non-fast-forward-apply.json`: `push --full
  --dry-run` and `push --full` in `E` — the apply refuses `non_fast_forward`
  with both generations.

## A load left unapplied

After `push --force` in `E` and an edit:

- `push-no-apply.json`: `push --no-apply` — `main` loaded `partial`,
  `applied: false`.
- `push-unapplied-preview.json`: `push --dry-run` — `skipped`; the step message
  names the pending apply.
- `push-unapplied-applied.json`: `push` — `skipped` with `applied: true`, the
  platform dispatched: the runner applied the earlier load.

After another edit, `push --no-apply` in `E` and `push --force` in `F`:

- `push-unapplied-moved.json`: `push` in `E` — `skipped`, `applied: false`,
  `provider_dispatched: true`: the runner read the generation, found that the
  infobase moved away from the record of the unapplied load and did not apply
  it; the step message says so.
