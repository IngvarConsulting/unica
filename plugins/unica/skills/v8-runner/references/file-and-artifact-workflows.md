# File And Artifact Workflows

Use `dump` to bring database changes into Git-visible files. Check the worktree before dump and review the diff after dump.

For an applied `dump`, use:

- `mode=full` for first workspace fill or explicit full export.
- `sourceSet` or `extension` for scoped export.

`mode=incremental` and `mode=partial` are temporarily available only as
read-only previews with `dryRun=true`. Applied execution is fail-closed until
v8-runner publishes through shadow/staging with exact path/hash receipts.
Partial preview also requires `object` or `objects`.

Use `convert` for Designer/EDT source conversion. It is repository-aware and does not require an infobase.

Use `make` for `.cf`, `.cfe`, `.epf`, or `.erf` artifacts. Provide `output`; add `sourceSet` or `extension` when the target is not the default source. For external processors/reports, `output` is a publish directory, not a single `.epf`/`.erf` filename.

Use `load` for applying `.cf` or `.cfe` artifacts. Supported modes are `load` and `merge`; `merge` requires `settings`, and `update` is not a supported load mode. v8-runner rejects `.epf` and `.erf` for `load`; external processors/reports are handled through external source-sets with `build`, `dump`, and `make`.
