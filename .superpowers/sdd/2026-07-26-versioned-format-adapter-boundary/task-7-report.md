# Task 7 report: Platform XML operational ports

Date: 2026-07-27
Branch: `codex/versioned-source-adapter-design`
Implementation commit: `ea68a83e` (`refactor: route platform xml guards through adapter`)

## RED

The Task 7 contract tests were written before production implementation.

| Command | RED result |
|---|---|
| `cargo test -p unica-format-core --test task7_operational_ports` | Exit 101: neutral operational contracts and ports were absent. |
| `cargo test -p unica-application --test task7_operational_policy` | Exit 101: format-neutral policy service and injectable operational ports were absent. |
| `cargo test -p unica-adapter-platform-xml --test task7_operational_ports` | Exit 101: compatibility, authorability, validation, publication ports and registration were absent. |
| `cargo test -p unica-coder format_guard::tests -- --nocapture` | Initial migration run: 51 passed, 18 failed; exposed message, malformed-root, validation-text, owner-kind, and versionless embedded-content regressions. |
| `cargo test -p unica-coder support_state_tests` | Intermediate run: 5 passed, 2 failed; exposed zero-vendor read-only and symlink canonicalization regressions. |

## GREEN

| Command | Result |
|---|---|
| `cargo test -p unica-format-core --test task7_operational_ports` | 4 passed. |
| `cargo test -p unica-application --test task7_operational_policy` | 3 passed, including an alternate fake adapter. |
| `cargo test -p unica-adapter-platform-xml --test task7_operational_ports` | 4 passed: supported/older/newer/malformed profiles, malformed support and validation, path privacy. |
| `cargo test -p unica-adapter-platform-xml factory::profile_contract_tests` | 2 passed: runtime registry and const profile identity retained. |
| `cargo test -p unica-adapter-platform-xml owner::tests` | 17 passed. |
| `cargo test -p unica-adapter-platform-xml certification::` | 9 passed. |
| `cargo test -p unica-adapter-platform-xml publication::tests -- --test-threads=1` | 36 passed: rollback, failure quarantine, races, no-downgrade, stage sealing, symlinks, platform attestation, and atomic visibility. |
| `cargo test -p unica-adapter-platform-xml versions::v2_20::support::tests` | 10 passed. |
| `cargo test -p unica-coder format_guard::tests` | 69 passed. |
| `cargo test -p unica-coder support_guard::tests` | 3 passed. |
| `cargo test -p unica-coder support_state_tests` | 7 passed. |
| `cargo test -p unica-coder tool_context::tests` | 5 passed. |
| `cargo test -p unica-coder application_ports::tests` | 1 passed: both synchronous full-dump routes use the verified adapter publication port. |
| `git diff --check` | Passed before the implementation commit. |
| Production boundary scan over migrated host files | No XML parser, namespace, native tag, platform/export literal, support-layout literal, or stale moved-module symbol remains in the migrated production files. |

## Responsibility map

| Previous host responsibility | New owner |
|---|---|
| Owner/profile compatibility classification, no-downgrade diagnostics, malformed owner roots, registered subsystem parsing | `unica-adapter-platform-xml/src/guards.rs` plus private `versions/v2_20` knowledge |
| Source-family compatibility interpretation | `SourceCompatibilityPort` in core, adapter implementation in `guards.rs`, neutral application policy call |
| Support-state parsing and authorability decisions | `AuthorabilityPort` in core, adapter `guards.rs`, private `versions/v2_20/support.rs` |
| Metadata validation context, owner/registration/language/registrar reads | `ValidationContextPort` in core and adapter `validation.rs` |
| Runtime-authoritative legacy metadata kind registry | Adapter `guards.rs`, backed by the private v2.20 runtime profile |
| Full-dump preflight, private stage, platform execution, validation, rollback and atomic publication | `PublicationPort` and adapter `publication.rs` |
| Process execution, bundled-tool resolution, cross-process publication gate | Neutral `PublicationHost` implemented by coder composition in `application_ports.rs` |
| Compatibility/authorability enforcement policy | Format-neutral `unica-application/src/commands.rs` |
| Cancellation and secret redaction shared by publication | `unica-format-core` operational contracts and `redaction.rs` |
| Legacy host parser/publication modules | Deleted `metadata_kinds.rs`; moved and removed host `platform/full_dump_publication.rs` |

## Files

- `Cargo.lock`
- `crates/unica-adapter-platform-xml/Cargo.toml`
- `crates/unica-adapter-platform-xml/src/factory.rs`
- `crates/unica-adapter-platform-xml/src/guards.rs`
- `crates/unica-adapter-platform-xml/src/lib.rs`
- `crates/unica-adapter-platform-xml/src/publication.rs`
- `crates/unica-adapter-platform-xml/src/validation.rs`
- `crates/unica-adapter-platform-xml/src/versions/v2_20/support.rs`
- `crates/unica-adapter-platform-xml/src/versions/v2_20/xml.rs`
- `crates/unica-adapter-platform-xml/tests/task7_operational_ports.rs`
- `crates/unica-application/src/commands.rs`
- `crates/unica-application/src/lib.rs`
- `crates/unica-application/tests/task7_operational_policy.rs`
- `crates/unica-coder/src/domain/cancellation.rs`
- `crates/unica-coder/src/infrastructure/application_ports.rs`
- `crates/unica-coder/src/infrastructure/format_guard.rs`
- `crates/unica-coder/src/infrastructure/internal_adapters.rs`
- `crates/unica-coder/src/infrastructure/metadata_kinds.rs` (deleted)
- `crates/unica-coder/src/infrastructure/mod.rs`
- `crates/unica-coder/src/infrastructure/native_operations/cf.rs`
- `crates/unica-coder/src/infrastructure/native_operations/code.rs`
- `crates/unica-coder/src/infrastructure/native_operations/compile_transaction.rs`
- `crates/unica-coder/src/infrastructure/native_operations/meta.rs`
- `crates/unica-coder/src/infrastructure/native_operations/meta_validation_context.rs`
- `crates/unica-coder/src/infrastructure/platform/full_dump_publication.rs` (moved/deleted from host)
- `crates/unica-coder/src/infrastructure/platform/mod.rs`
- `crates/unica-coder/src/infrastructure/platform_xml_owner.rs`
- `crates/unica-coder/src/infrastructure/project_sources.rs`
- `crates/unica-coder/src/infrastructure/support_guard.rs`
- `crates/unica-coder/src/infrastructure/tool_context.rs`
- `crates/unica-format-core/src/lib.rs`
- `crates/unica-format-core/src/ports.rs`
- `crates/unica-format-core/src/redaction.rs`
- `crates/unica-format-core/tests/task7_operational_ports.rs`

## Residual risks and scope notes

- The request's absolute statement that all of `unica-coder` must stop matching source filesystem layout conflicts with actual Plan Task 8/9. Writer implementation and generic source discovery remain in the host by plan; `project_sources.rs` still contains two pre-existing `Configuration.xml` joins. Migrated Task 7 production files and all `unica-application` production code are clean, and filesystem paths remain command/session-private and absent from navigation JSON.
- Writer implementations were intentionally not moved. Task 8 owns that migration.
- An unscoped full `unica-adapter-platform-xml` run exposed failures in unchanged decoder/probe/projector tests and stalled in an existing navigation-limit fixture; it was terminated. The targeted Task 5/6 invariants and all Task 7 acceptance suites listed above are green. No unchanged decoder/probe/projector code was committed by Task 7.
- Existing dead-code warnings in the prior source-adapter registry remain; Task 7 did not broaden scope to remove Task 5/6 scaffolding.
- No full-workspace or Task 8 writer suite was run, per the instruction to use only Task 7 scoped validation.
