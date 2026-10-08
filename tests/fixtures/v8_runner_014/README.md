# Published runner envelopes

Captured 2026-10-08 from the published macOS aarch64 archive of
IngvarConsulting/v8-runner-rust v0.14.0 (`v8-runner-macos-aarch64.tar.gz`,
archive SHA-256 `600dc67b2566ae00d60bc151b0fb419ca0b5406427e799b590600d99bacfc4a5`),
source commit `d2944fe851541ce65bc3a202f401b92229386fc3`, binary SHA-256
`3d691c668296e088188454053a940e887334a63d935a9b9e7203d8d5d2f180b5`.
Platform: 1C 8.3.27.2074 (`tools.platform.path` in the local layer). Every
command used `--config <project>/v8project.yaml --json-message`, and each
infobase was an isolated file infobase created by the same runner
(`infobase create`). Machine-specific prefixes were replaced: the capture
directory by `/workspace`, the platform directory by `/platform`, the host name
by `dev-host`. These replacements touch prose and path fields alike; everything
else is captured bytes, re-indented.

These fixtures attest the runner boundary, not a portable guarantee that every
host has a working platform installation.

## Extensions

Project file: `providers: {extensions: ibcmd}` and one `CONFIGURATION` source
set; the infobase was declared in `v8project.local.yaml` as
`infobases.origin`. Each command ran first with `--dry-run`, then without:

- `extensions create --name UnicaRunnerProbe --name-prefix URP_ --purpose patch`
- `extensions info --name UnicaRunnerProbe`
- `extensions activate --name UnicaRunnerProbe --active no`
- `extensions activate --name UnicaRunnerProbe --active yes`
- `extensions delete --name UnicaRunnerProbe`
- `extensions list` (empty after deletion)

The shapes match the 0.13.0 captures they replace.

## Creating an infobase and the first pushes

The default providers of 0.14.0: `ibcmd` creates the file infobase, the
managed Designer agent pushes. One `CONFIGURATION` source set `main`; the
infobase, absent before the capture, was declared in `v8project.local.yaml` as
`infobases.origin`. Then:

- `infobase-create-preview.json`: `infobase create --dry-run` — the infobase
  step is `planned` with the main configuration of `main`, the EDT step
  `skipped`.
- `infobase-create-apply.json`: `infobase create` — `ibcmd` assembled the
  infobase with the main configuration. Afterwards `workPath` held
  `infobases/origin/hashes/main.redb` — the memory of the assembled set — and
  no `generation.json`.
- `infobase-create-receipt-preview.json`: `infobase create --dry-run` again —
  refused `invalid_argument` (kind `validation`) before the platform starts,
  the infobase step `failed`: the file infobase exists.
- `push-after-create-preview.json`, `push-after-create-apply.json`: `push
  --dry-run` and `push` — `main` is `skipped`, the platform is not started.
- `push-after-edit-preview.json`, `push-after-edit-apply.json`: after an edit
  of `Configuration.xml`, `push --dry-run` and `push` — `main` is loaded
  `full` by the agent, no `no_memory`; `generation.json` appeared after this
  load.

## Push against another working copy

Project `C` created the infobase and pushed an edit (above). Project `D`
declared the same infobase by its absolute path. Then:

- `push-no-memory-preview.json`: `push --dry-run` in `D` — `no_memory`, `D`
  has no memory of the base; `warnings` names the infobase of another working
  copy.
- `push-other-copy-force-preview.json`, `push-other-copy-force-apply.json`:
  `push --force --dry-run` and `push --force` in `D` — the full load over
  `C`'s configuration goes ahead with the warning about the infobase of
  another working copy; the owner marker still names `C`.
- `push-skipped-preview.json`, `push-skipped-apply.json`: `push --dry-run` and
  `push` in `C` — every set `skipped` by `C`'s memory; the platform was not
  started and the generation was not read, although the infobase held `D`'s
  configuration (issue #1240).
- `push-full-preview.json`, `push-non-fast-forward-apply.json`:
  `push --full --dry-run` and `push --full` in `C` — the preview plans a full
  load; the apply reads the generation and refuses `non_fast_forward` with both
  generations.
- `push-infobase-busy-apply.json`: `push --full` in `D` while `C` ran
  `pull --force` on the same infobase — `infobase_busy`.
- `other-copy-pull-preview.json`, `other-copy-pull-apply.json`:
  `pull main --force --dry-run` and `pull main --force` in `D`;
  `other-copy-extensions-deactivate-preview.json`,
  `other-copy-extensions-deactivate-apply.json`: `extensions activate --name
  UnicaOtherCopy --active no` in `D` after `C` created the extension;
  `other-copy-restore-preview.json`, `other-copy-restore-apply.json`:
  `infobase restore --input <C's dump>.dt --replace` in `D`. Each goes ahead
  with the same warning about the infobase of another working copy.
  Unica recognizes that warning by its wording, which has no code: recapture
  `push-other-copy-*` and `other-copy-*` whenever the pinned runner changes.
- `shared-key-refused.json`: `push --dry-run` in `D` with `shared: true` added
  to `infobases.origin` of its local layer — refused `invalid_argument`
  (unknown field): 0.14.0 removed the key.
