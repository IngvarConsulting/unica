# Task 1 Report: Source Adapter Domain Contracts

## Changed Files

- `crates/unica-coder/src/domain/mod.rs`: exported the crate-private `source_adapters` domain module.
- `crates/unica-coder/src/domain/source_adapters.rs`: added format-version/range, source descriptor, snapshot, manifest, identity, and structured error contracts with inline tests.
- `.superpowers/sdd/2026-07-26-platform-xml-source-adapter-core/task-1-report.md`: recorded this Task 1 implementation report.

## Decisions

- Format versions use ordered numeric components, not semantic-version inference. A range includes only versions between its explicit inclusive bounds.
- Invalid numeric format text and all-zero versions return `SourceAdapterErrorKind::FormatUnsupported` with the `format_unsupported` code.
- `SourceId` and `SourceRevision` reject empty values and control characters, and serialize as their raw string values without physical paths.
- Domain structs and enums use camelCase serialized field and variant names. All accepted source-adapter error kinds expose their specified snake_case error code.
- The public MCP boundary remains one `unica` server; Task 1 adds no MCP server or tool registration.

## Self-Check

- The initial test-first run failed as expected because `SourceSnapshot`, `FormatRange`, `FormatVersion`, `SourceAdapterErrorKind`, `SourceId`, `SourceRevision`, and `SnapshotConsistency` did not yet exist.
- The implementation provides every contract and error kind named in the Task 1 brief.
- `plugins/unica/.mcp.json` still declares only the `unica` MCP server.

## Tests

Command:

```bash
cargo test -p unica-coder domain::source_adapters::tests -- --nocapture
```

Initial TDD output:

```text
error[E0422]: cannot find struct, variant or union type `SourceSnapshot` in this scope
error[E0433]: cannot find type `FormatRange` in this scope
error[E0433]: cannot find type `FormatVersion` in this scope
error[E0433]: cannot find type `SourceAdapterErrorKind` in this scope
error[E0433]: cannot find type `SourceId` in this scope
error[E0433]: cannot find type `SourceRevision` in this scope
error[E0433]: cannot find type `SnapshotConsistency` in this scope
error: could not compile `unica-coder` (lib test) due to 11 previous errors
```

Final output:

```text
running 3 tests
test domain::source_adapters::tests::invalid_format_versions_are_structured_failures ... ok
test domain::source_adapters::tests::format_ranges_are_explicit_and_do_not_select_nearest_versions ... ok
test domain::source_adapters::tests::snapshot_serialization_does_not_expose_physical_locations ... ok

test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 723 filtered out; finished in 0.00s
```

The final command exited with status 0. It emitted expected `dead_code` warnings because these crate-private contracts are intentionally introduced before their future adapters consume them.
