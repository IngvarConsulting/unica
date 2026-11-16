# Runner 0.17 envelopes: the refusal without memory of the infobase

Captured 2026-11-10 on Windows from a build of IngvarConsulting/v8-runner-rust
master `f094d9a7` (0.17 candidate with #588, envelope 5; the binary still
reports `0.16.0`), binary SHA-256
`af241b049af1cc32816caff0adf3503e25302fa381b5997d6ff03d182ce3c5b5`. No
platform was started. The working-copy path in the message is replaced by
`/workspace/F` (forward slashes); every other byte is as captured.

- `push-untracked-base-preview.json`: `push --dry-run` with `--config
  <F>/v8project.yaml --json-message` in a working copy `F` with `format:
  DESIGNER`, `providers: {push: ibcmd}`, one `CONFIGURATION` set `main`, the
  file infobase `File=ib` in `v8project.local.yaml` and no memory of it —
  `untracked_base` (formerly `no_memory`), exit 3.
