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

## Fix Round 1

### Base and commits

- Review base: `d9ad7f7a0a60f75d2b726b54afe44218ae623ed3`.
- Primary implementation: `885b4a243ec5aef79bf5c4f450d070f8dda58914` (`fix: close Task 7 operational adapter boundary`).
- Source-discovery correction: `f57871a187bf08b4931b9720d17c765d2d2696ce` (`fix: keep source discovery adapter-owned`).
- Public-profile cleanup: `5a0a9c4f21e83972c66bb8ba7530c3d60663b3ff` (`fix: remove public native format profile`).
- The controller-owned `progress.md` change was never staged or modified by this work.

### RED

- `cargo test -p unica-adapter-platform-xml --test task7_fix_round1_architecture`
  initially failed all four initial adversarial checks: operational `PathBuf` leakage, public native registry/query leakage, host support-reader retention, and publication state inferred from message text.
- `cargo test -p unica-coder infrastructure::format_guard::tests --lib`
  reached an intermediate RED of 59 passed / 10 failed while owner selection, versionless artifacts, detached aggregate content, registrar references, empty prospective roots, and canonical identities were moved.
- After adding the fifth dependency/call-path assertion,
  `cargo test -p unica-adapter-platform-xml --test task7_fix_round1_architecture task7_moved_flows_have_no_native_layout_and_remaining_joins_are_writer_locations`
  failed because `project_sources.rs` still joined `Configuration.xml` and parsed reserved external descriptors in the host.
- The first source-discovery GREEN transition compiled but produced 9 passed / 8 failed in
  `cargo test -p unica-coder infrastructure::project_sources::tests --lib`;
  the failures exposed path-bearing evidence expectations, an unnormalized authorized-root alias, and the former permissive symlink behavior.
- A final focused RED,
  `cargo test -p unica-adapter-platform-xml --test task7_fix_round1_architecture task7_native_registry_and_queries_are_adapter_private`,
  failed on the unused public `AdapterFormatProfile` raw-string escape hatch.

### GREEN

The final Task 7 scoped matrix passed:

- `cargo test -q -p unica-format-core --test task7_operational_ports`: 7 passed.
- `cargo test -q -p unica-application --test task7_operational_policy`: 5 passed.
- `cargo test -q -p unica-adapter-platform-xml --test task7_fix_round1_architecture`: 5 passed.
- `cargo test -q -p unica-adapter-platform-xml --test task7_operational_ports`: 7 passed.
- `cargo test -q -p unica-adapter-platform-xml operational_capture --lib`: 7 passed.
- `cargo test -q -p unica-adapter-platform-xml publication --lib`: 38 passed.
- `cargo test -q -p unica-adapter-platform-xml coverage --lib`: 1 passed.
- `cargo test -q -p unica-adapter-platform-xml source_sets::tests --lib`: 4 passed.
- `cargo test -q -p unica-adapter-platform-xml --test legacy_parity coverage_manifest_is_runtime_checked_and_rejects_every_authority_mutation`: 1 passed.
- `cargo test -q -p unica-coder infrastructure::format_guard::tests --lib`: 71 passed.
- `cargo test -q -p unica-coder infrastructure::support_guard::tests --lib`: 4 passed.
- `cargo test -q -p unica-coder infrastructure::native_operations::common::support_state_tests --lib`: 7 passed.
- `cargo test -q -p unica-coder meta_validation_context --lib`: 2 passed.
- `cargo test -q -p unica-coder infrastructure::application_ports::task7_fix_round1_publication_tests --lib`: 2 passed.
- `cargo test -q -p unica-coder infrastructure::project_sources::tests --lib`: 17 passed.
- `cargo test -q -p unica-coder config_dump_info --lib`: 19 passed.
- `git diff --check`: passed.

The matrix above is 197 focused passing tests. After removing `AdapterFormatProfile`, the affected slices were rerun: architecture 5 passed, core operational contracts 7 passed, application Task 7 policy 5 passed, and application navigation 32 passed.

### Moved responsibility map

| Responsibility | Host before fix | Owner after fix |
| --- | --- | --- |
| Compatibility/version inspection and no-downgrade decisions | `format_guard.rs`, native helper queries, raw profile access | `unica-format-core` closed compatibility contracts plus adapter `guards.rs` and private `v2_20::operations` |
| Support-state inspection and write refusal | `support_guard.rs`, `native_operations/common.rs`, host support parser helpers | adapter `guards.rs`, private `v2_20::operations` and private support mapping; unreadable evidence is a typed fail-closed state |
| Validation context, registrations, languages, registrars and method references | `meta_validation_context.rs` and branches in `native_operations/meta.rs` | adapter `validation.rs` and private `v2_20::operations` over one immutable captured snapshot |
| Full-dump publication lifecycle | host process/result inference and free-form text | adapter `publication.rs` plus closed core cancellation, rollback, cleanup and recovery states |
| Operational source identity | path-bearing core/application DTOs | opaque `OperationalSourceSession`; paths exist only while command/composition code captures the adapter-private session |
| Platform source-set discovery and reserved descriptor classification | `project_sources.rs`, domain XML parser and Git blob classifier | private `v2_20::source_sets` over the safe provider snapshot, returning only closed neutral match/artifact kinds |
| Native metadata capability registry | duplicated guard table plus public re-exports | one private `v2_20` registry shared by decoding, coverage and guards; initialization/coverage tests enforce consistency |
| Public diagnostics/results | arbitrary adapter maps, paths and free text | allowlisted closed codes/details and typed publication lifecycle mapping |

### Files

Core/application contracts and tests:

- `crates/unica-format-core/src/ports.rs`
- `crates/unica-format-core/tests/task7_operational_ports.rs`
- `crates/unica-application/src/commands.rs`
- `crates/unica-application/src/navigation.rs`
- `crates/unica-application/tests/task7_operational_policy.rs`

Platform XML adapter:

- `crates/unica-adapter-platform-xml/src/factory.rs`
- `crates/unica-adapter-platform-xml/src/guards.rs`
- `crates/unica-adapter-platform-xml/src/lib.rs`
- `crates/unica-adapter-platform-xml/src/owner.rs`
- `crates/unica-adapter-platform-xml/src/publication.rs`
- `crates/unica-adapter-platform-xml/src/validation.rs`
- `crates/unica-adapter-platform-xml/src/versions/v2_20/mod.rs`
- `crates/unica-adapter-platform-xml/src/versions/v2_20/operations.rs`
- `crates/unica-adapter-platform-xml/src/versions/v2_20/probe.rs`
- `crates/unica-adapter-platform-xml/src/versions/v2_20/provider.rs`
- `crates/unica-adapter-platform-xml/src/versions/v2_20/source_sets.rs`
- `crates/unica-adapter-platform-xml/tests/legacy_parity.rs`
- `crates/unica-adapter-platform-xml/tests/task7_fix_round1_architecture.rs`
- `crates/unica-adapter-platform-xml/tests/task7_operational_ports.rs`

Coder composition and writer-boundary callers:

- `crates/unica-coder/src/domain/format_profile.rs`
- `crates/unica-coder/src/domain/project_sources.rs`
- `crates/unica-coder/src/infrastructure/application_ports.rs`
- `crates/unica-coder/src/infrastructure/format_guard.rs`
- `crates/unica-coder/src/infrastructure/internal_adapters.rs`
- `crates/unica-coder/src/infrastructure/metadata_kinds.rs`
- `crates/unica-coder/src/infrastructure/mod.rs`
- `crates/unica-coder/src/infrastructure/native_operations/cf.rs`
- `crates/unica-coder/src/infrastructure/native_operations/code.rs`
- `crates/unica-coder/src/infrastructure/native_operations/common.rs`
- `crates/unica-coder/src/infrastructure/native_operations/dcs.rs`
- `crates/unica-coder/src/infrastructure/native_operations/form.rs`
- `crates/unica-coder/src/infrastructure/native_operations/meta.rs`
- `crates/unica-coder/src/infrastructure/native_operations/meta_validation_context.rs`
- `crates/unica-coder/src/infrastructure/native_operations/mxl.rs`
- `crates/unica-coder/src/infrastructure/native_operations/role.rs`
- `crates/unica-coder/src/infrastructure/native_operations/subsystem.rs`
- `crates/unica-coder/src/infrastructure/native_operations/support.rs`
- `crates/unica-coder/src/infrastructure/platform_xml_owner.rs`
- `crates/unica-coder/src/infrastructure/project_sources.rs`
- `crates/unica-coder/src/infrastructure/support_guard.rs`
- `crates/unica-coder/src/infrastructure/tool_context.rs`

### Remaining native host paths and justification

- `crates/unica-coder/src/infrastructure/format_guard.rs` retains exactly three production `Configuration.xml` joins, in `add_meta_remove_format_dependencies`, `add_subsystem_compile_format_dependencies`, and `add_role_compile_format_dependencies`. They calculate source locations for Task 8 read-modify-write serializers; the architecture test fails if another join appears.
- `crates/unica-coder/src/infrastructure/native_operations/{cf,cfe,code,common,form,interface,meta,role,subsystem,support}.rs` retains native filenames inside existing writer serialization, registration updates, mutation provenance and support-write implementation. Moving those implementations now would violate the explicit Task 8 boundary.
- `crates/unica-coder/src/application/mod.rs` and `application/tool_contracts.rs` retain native writer argument/help prose required by the current public tool contract. They do not perform Task 7 reads or compatibility/support/validation decisions.
- `crates/unica-coder/src/infrastructure/internal_adapters.rs` retains the `ConfigDumpInfo.xml` Git pathspec and user-facing hygiene warning. It does not read workspace Platform XML; staged blob bytes are classified through the adapter-owned neutral `ReservedSourceArtifactKind` contract.
- `crates/unica-coder/src/infrastructure/workspace.rs` retains legacy workspace marker names. Platform XML source-set detection in `project_sources.rs` no longer uses them; changing general workspace discovery is outside Task 7.
- `SourceAdapterRegistration.manifest` remains the generic Task 2 navigation adapter capability manifest. The unused public `AdapterFormatProfile` containing raw platform/export strings was deleted; Task 7 operational compatibility does not expose or consume the manifest range.

### Residual risks and validation notes

- The requested scoped validation is green; the full workspace suite was not rerun.
- An exploratory full `unica-format-core` run still reaches the pre-existing `EmptyReference` property-registry expectation mismatch. The relevant Task 7 core test is green and the affected registry/navigation files are unchanged from the review base.
- An exploratory full `legacy_parity` run still has three pre-existing projection/oracle failures; the runtime coverage mutation test relevant to this fix is green and projection implementation files are unchanged from the review base.
- `cargo fmt --all -- --check` reports broad repository formatting drift, including already committed Task 4/5/6 and Task 7 files. It was not used as a Task 7 gate, and no unrelated formatting-only rewrite was introduced.
- Safe source-set capture now rejects a symlink anywhere in the captured aggregate rather than silently classifying a reserved symlink as absent. This is an intentional fail-closed correction required by the authorization/evidence binding finding.

## Fix Round 2

Base: `b3c934a4137ceb1296c477abacf4d1f25403a3ae`

Implementation commit: `3543a49451c41f969a53f904ffc2df5f1938cf13`

The controller-owned `.superpowers/sdd/2026-07-26-versioned-format-adapter-boundary/progress.md` remained unstaged and was not modified by this fix round.

### RED

- `cargo test -p unica-format-core --test task7_fix_round2_contracts`: failed because authorability still admitted optional/contradictory states and validation/publication lifecycle contracts were not closed.
- `cargo test -p unica-adapter-platform-xml --test task7_fix_round2_architecture`: 4/4 failed against the whole-root provider, host metadata registry, public native registry helpers, and free-form operational DTOs.
- `cargo test -p unica-coder validate_meta_ --lib`: 12 failed, 6 passed; standalone targets and language-context validation were still routed through host/project-map assumptions.
- `cargo test -p unica-coder infrastructure::project_sources::tests --lib`: 14 failed, 3 passed during canonical-alias RED; requested and opened source identities were not normalized to the same authorized descriptor.
- `cargo test -p unica-adapter-platform-xml publication::tests --lib`: 1 failed, 38 passed; an entity-obfuscated raw export version was accepted after XML entity expansion.

### GREEN responsibility map

| Responsibility | Fix Round 2 owner |
| --- | --- |
| Authorability and support refusal | Closed `Allowed(evidence) / Denied(reason)` core contract; adapter inspection errors necessarily produce `supportStateUnreadable`; application exhaustively matches. |
| Source sessions | Adapter-owned authorized-root capability with descriptor-relative no-follow reads, exact opened-handle identity/digest evidence, lazy selected-artifact traversal, and typed per-artifact/page/operation limits. |
| Navigation capture | `v2_20/provider.rs` no longer snapshots the whole root or owns 512-file/64 MiB aggregate limits; it captures descriptor/configuration/support evidence and lazily materializes only the selected companion scope. |
| Validation and guards | Private `v2_20/validation.rs`, `operations.rs`, and adapter guard ports parse native XML/support evidence; host policy receives closed neutral findings only. |
| Metadata mapping | Private checked `v2_20/coverage.json` plus `semantic_map.rs` is the sole native class/directory mapping used by decoding, coverage, discovery, guards, validation, and semantic artifact location. |
| Writer preimages | Core semantic artifact leases and adapter artifact access replace host guard/preimage layout discovery; native serialization itself remains for Task 8. |
| Publication | Closed lifecycle enum preserves cancellation phase, rollback, cleanup, and recovery states; staged XML is validated from snapshot-bound bytes and exact raw version tokens; no message parsing. |
| Public privacy | Closed codes/details and opaque semantic/session IDs replace path-bearing DTOs; public command, batch, navigation, tool-context, and publication mappings are denylist-tested recursively. |
| Host cleanup | Deleted `metadata_kinds.rs`, `meta_validation_context.rs`, hidden native registry/query exports, and stale format/support parser call paths. |

### GREEN validation

```text
cargo test -q -p unica-format-core --test task7_operational_ports --test task7_fix_round2_contracts
  7 passed; 7 passed

cargo test -q -p unica-application --test task7_operational_policy --test task7_fix_round2_policy
  2 passed; 5 passed

cargo test -q -p unica-adapter-platform-xml   --test task7_operational_ports   --test task7_fix_round1_architecture   --test task7_fix_round2_architecture   --test task7_fix_round2_lazy_source
  9 passed; 5 passed; 4 passed; 3 passed

cargo test -q -p unica-adapter-platform-xml provider::tests --lib
  5 passed

cargo test -q -p unica-adapter-platform-xml safe_root::tests --lib
  2 passed

cargo test -q -p unica-adapter-platform-xml authorization_order_tests --lib
  2 passed

cargo test -q -p unica-adapter-platform-xml publication::tests --lib
  39 passed

cargo test -q -p unica-adapter-platform-xml registry_authority_tests --lib
  1 passed

cargo test -q -p unica-adapter-platform-xml --test legacy_parity --   --skip fix_round6_rights_target_crosswalk_equals_runtime_supported_top_level_registry   --skip rights_mixed_content_remains_typed_where_known_and_opaque_where_unknown   --skip fix_round5_static_new_only_contract_is_exact_and_mutation_sensitive
  23 passed; 3 base-confirmed failures skipped

cargo test -q -p unica-coder validate_meta_ --lib
  20 passed
cargo test -q -p unica-coder meta_validation_batch_json_is_recursively_path_and_native_vocabulary_free --lib
  1 passed
cargo test -q -p unica-coder infrastructure::support_guard::tests --lib
  4 passed
cargo test -q -p unica-coder infrastructure::format_guard::tests --lib
  1 passed
cargo test -q -p unica-coder infrastructure::tool_context::tests --lib
  6 passed
cargo test -q -p unica-coder task7_fix_round1_publication_tests --lib
  2 passed
cargo test -q -p unica-coder semantic_guard_is_rechecked_before_any_publication --lib
  1 passed
cargo test -q -p unica-coder exact_family_guard --lib
  3 passed
cargo test -q -p unica-coder infrastructure::project_sources::tests --lib
  17 passed

git diff --check
  clean
```

The large-source fixtures include more than 512 unrelated files and a 96 MiB sparse unrelated artifact; discovery, guard, validation, and selected navigation continue without whole-root loading. Deterministic no-follow tests cover authorization ordering, alias/canonical identity, symlink/reparse rejection, and swap-open-swap-back behavior.

The three skipped `legacy_parity` tests fail identically at the requested base commit and current implementation. They are pre-existing rights-oracle expectation defects, not Round 2 regressions.

### Files in implementation commit

- `crates/unica-adapter-platform-xml/src/artifact_access.rs`
- `crates/unica-adapter-platform-xml/src/factory.rs`
- `crates/unica-adapter-platform-xml/src/guards.rs`
- `crates/unica-adapter-platform-xml/src/lib.rs`
- `crates/unica-adapter-platform-xml/src/publication.rs`
- `crates/unica-adapter-platform-xml/src/safe_root.rs`
- `crates/unica-adapter-platform-xml/src/validation.rs`
- `crates/unica-adapter-platform-xml/src/versions/v2_20/coverage.json`
- `crates/unica-adapter-platform-xml/src/versions/v2_20/decoder.rs`
- `crates/unica-adapter-platform-xml/src/versions/v2_20/inspection.rs`
- `crates/unica-adapter-platform-xml/src/versions/v2_20/mod.rs`
- `crates/unica-adapter-platform-xml/src/versions/v2_20/operations.rs`
- `crates/unica-adapter-platform-xml/src/versions/v2_20/projector.rs`
- `crates/unica-adapter-platform-xml/src/versions/v2_20/provider.rs`
- `crates/unica-adapter-platform-xml/src/versions/v2_20/semantic_map.rs`
- `crates/unica-adapter-platform-xml/src/versions/v2_20/source_sets.rs`
- `crates/unica-adapter-platform-xml/src/versions/v2_20/validation.rs`
- `crates/unica-adapter-platform-xml/src/versions/v2_20/xml.rs`
- `crates/unica-adapter-platform-xml/tests/legacy_parity.rs`
- `crates/unica-adapter-platform-xml/tests/task7_fix_round1_architecture.rs`
- `crates/unica-adapter-platform-xml/tests/task7_fix_round2_architecture.rs`
- `crates/unica-adapter-platform-xml/tests/task7_fix_round2_lazy_source.rs`
- `crates/unica-adapter-platform-xml/tests/task7_operational_ports.rs`
- `crates/unica-application/src/commands.rs`
- `crates/unica-application/tests/task7_fix_round2_policy.rs`
- `crates/unica-application/tests/task7_operational_policy.rs`
- `crates/unica-coder/src/application/ports.rs`
- `crates/unica-coder/src/infrastructure/application_ports.rs`
- `crates/unica-coder/src/infrastructure/format_guard.rs`
- `crates/unica-coder/src/infrastructure/internal_adapters.rs`
- `crates/unica-coder/src/infrastructure/metadata_kinds.rs`
- `crates/unica-coder/src/infrastructure/mod.rs`
- `crates/unica-coder/src/infrastructure/native_operations.rs`
- `crates/unica-coder/src/infrastructure/native_operations/cf.rs`
- `crates/unica-coder/src/infrastructure/native_operations/code.rs`
- `crates/unica-coder/src/infrastructure/native_operations/common.rs`
- `crates/unica-coder/src/infrastructure/native_operations/compile_transaction.rs`
- `crates/unica-coder/src/infrastructure/native_operations/external.rs`
- `crates/unica-coder/src/infrastructure/native_operations/form.rs`
- `crates/unica-coder/src/infrastructure/native_operations/help.rs`
- `crates/unica-coder/src/infrastructure/native_operations/interface.rs`
- `crates/unica-coder/src/infrastructure/native_operations/meta.rs`
- `crates/unica-coder/src/infrastructure/native_operations/meta_validation_context.rs`
- `crates/unica-coder/src/infrastructure/native_operations/template.rs`
- `crates/unica-coder/src/infrastructure/platform_xml_owner.rs`
- `crates/unica-coder/src/infrastructure/project_sources.rs`
- `crates/unica-coder/src/infrastructure/support_guard.rs`
- `crates/unica-coder/src/infrastructure/tool_context.rs`
- `crates/unica-format-core/src/navigation.rs`
- `crates/unica-format-core/src/ports.rs`
- `crates/unica-format-core/tests/task7_fix_round2_contracts.rs`
- `crates/unica-format-core/tests/task7_operational_ports.rs`

### Remaining host-native code and Task 8 boundary

- `native_operations/meta.rs`, `form.rs`, `template.rs`, `help.rs`, `role.rs`, `subsystem.rs`, and `interface.rs` retain native XML generation/mutation and destination topology needed by existing writer implementations. Moving those serializers is Task 8.
- `native_operations/cf.rs`, `cfe.rs`, `mxl.rs`, `dcs.rs`, and `support.rs` retain legacy command-specific native parsers/writers outside the Task 7 authorability, guard, validation-context, and publication policy call paths. They are not claimed as neutral or writer-only; completing a universal native-command boundary remains a later migration risk.
- `native_operations/common.rs::resolve_cf_edit_config_path`, `resolve_cf_read_config_path`, `resolve_cfe_validate_config_path`, and `resolve_existing_path` remain command/composition path setup for legacy handlers. Task 7 policy ports do not call them, but they should move when those handlers are adapterized.
- Test fixtures continue to construct native filenames and XML intentionally; production Task 7 policy modules do not.

### Residual risks

- Windows reparse-point behavior is implemented with handle-relative no-follow semantics but was not runtime-executed on this macOS validation host.
- Full workspace validation was intentionally not run; validation was limited to Task 7 scope as requested.
- The three base-confirmed rights-oracle failures listed above remain outside this fix round.
