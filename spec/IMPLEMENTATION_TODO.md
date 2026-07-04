# Active Tasks For `unica`

This file tracks open implementation work only.

## Current Tasks

- [ ] Keep parity-unproven XML/DSL tools on the transitional internal
  operation-file adapter until focused fixtures prove native Rust equivalence.
- [ ] Complete operation-specific native writers for mutating XML/DSL tools where
  `dryRun: false` still refuses execution until the writer is implemented.
- [ ] Expand fixture parity beyond the native generic XML parser for rich legacy
  `info`, `validate`, `compile`, `edit`, remove, and CFE/UI outputs.

## Rules

- Keep this file short and active-only.
- If a task changes a public or architectural contract, update the ADR and active
  docs layer before implementation.
- Promote only immediately executable work here.

## Done Criteria

- The behavior is covered by a focused test.
- The relevant ADR or invariant is updated if the public contract changes.
- `python3.12 -m unittest discover -s tests/ci` and
  `cargo test --package unica-coder` pass.
- `cargo run --quiet --bin unica -- --help` still reports the public server as
  `unica`, and generated package metadata starts `./bin/<target>/unica`
  directly with `cwd` set to the plugin root.
