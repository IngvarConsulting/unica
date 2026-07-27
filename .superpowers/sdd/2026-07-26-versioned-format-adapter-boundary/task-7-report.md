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

## Fix Round 3

Base: `88f4a7653593318c3c8bd9aa223eea6bf51b2b92`

Implementation commit: `9c53e058af1ab3b777c22161b979a83b6f7ff1a7`

The controller-owned `.superpowers/sdd/2026-07-26-versioned-format-adapter-boundary/progress.md` remained unstaged and was not modified by this fix round.

### RED

- `cargo check -p unica-adapter-platform-xml --target x86_64-pc-windows-msvc --lib --tests` failed with nine errors: unstable Windows `MetadataExt` APIs, invalid width assumptions, and default/sentinel identity fallback.
- `cargo test -p unica-format-core --test task7_fix_round3_evidence` initially failed to compile because read-derived operational results had no mandatory evidence revision.
- `cargo test -p unica-adapter-platform-xml --test task7_fix_round3_lazy_revision` exposed two root defects: changing a reachable form companion did not change the navigation revision, and an unrelated malformed registered descriptor changed target-local validation.
- The new adapter read-log probe showed global descriptor-index construction opening unrelated registrations before validating the selected semantic target.
- The existing lexical architecture checks could be bypassed by moving or renaming forbidden native read/layout logic behind another helper module; the new AST call-graph negative fixtures reproduced both bypasses.
- Unskipped `cargo test -p unica-adapter-platform-xml --test legacy_parity` reproduced all three reported failures: rights registry readiness drift, mixed-content availability drift, and static new-only rights contract drift.

### GREEN responsibility map

| Responsibility | Fix Round 3 owner |
| --- | --- |
| Windows opened-handle identity | Private `platform_handle.rs` uses stable `windows-sys::GetFileInformationByHandle`, explicit width conversion, and typed query failure. `safe_root.rs` and publication consume that same opened-handle identity with no pathname/default fallback. |
| Target-local validation | Private `v2_20/operations.rs` resolves the selected semantic descriptor directly through the checked registry and opens only required dependent artifacts. Global descriptor-index traversal was removed. |
| Navigation revision | `v2_20/provider.rs` captures the complete reachable descriptor/companion read set before finalizing an immutable deterministic revision, then serves only captured bytes. |
| Operational evidence | Core requires a validated opaque `OperationalEvidenceRevision` on every read-derived compatibility, authorability, validation-context, and validation result. Adapter operation forks finalize evidence over exactly the artifacts they read. |
| Architecture boundary | `task7_fix_round3_architecture.rs` builds a Rust AST/module call graph from the three Task 7 host entrypoints. Reachable host parser imports/calls, native layout joins, native wire literals, and parser crates fail; neutral trait-object port calls terminate the graph. |
| Legacy parity | Private projection/semantic mapping once again distinguishes resolved external rights targets, duplicate unknown occurrences, and known unresolved targets without leaking native identity. All 26 parity cases and affected Task 5/6 relation suites run unconditionally. |
| Cross-platform CI | The macOS Rust CI job installs `x86_64-pc-windows-msvc` and runs the adapter `--lib --tests` cross-target check, so every `cfg(windows)` adapter module is compiled on non-Windows CI. |

### GREEN validation

```text
cargo test -p unica-format-core \
  --test task7_operational_ports \
  --test task7_fix_round2_contracts \
  --test task7_fix_round3_evidence \
  --test public_json_contract
  public_json_contract: 5 passed
  task7_fix_round2_contracts: 7 passed
  task7_fix_round3_evidence: 3 passed
  task7_operational_ports: 7 passed

cargo test -p unica-application \
  --test task7_operational_policy \
  --test task7_fix_round2_policy
  task7_fix_round2_policy: 2 passed
  task7_operational_policy: 5 passed

cargo test -p unica-adapter-platform-xml \
  --test task7_fix_round1_architecture \
  --test task7_fix_round2_architecture \
  --test task7_fix_round2_lazy_source \
  --test task7_fix_round3_architecture \
  --test task7_fix_round3_lazy_revision \
  --test task7_operational_ports \
  --test legacy_parity \
  --test specialized_relations \
  --test unmapped_fact
  legacy_parity: 26 passed
  specialized_relations: 7 passed
  task7_fix_round1_architecture: 5 passed
  task7_fix_round2_architecture: 2 passed
  task7_fix_round2_lazy_source: 3 passed
  task7_fix_round3_architecture: 3 passed
  task7_fix_round3_lazy_revision: 3 passed
  task7_operational_ports: 9 passed
  unmapped_fact: 8 passed

cargo test -p unica-adapter-platform-xml --lib platform_handle::tests
  3 passed
cargo test -p unica-adapter-platform-xml --lib safe_root::tests
  2 passed
cargo test -p unica-adapter-platform-xml --lib versions::v2_20::provider::tests
  6 passed
cargo test -p unica-adapter-platform-xml --lib publication::tests
  39 passed

cargo check -p unica-format-core -p unica-application -p unica-coder \
  -p unica-adapter-platform-xml --tests
  passed; unica-coder emitted 21 existing dead-code warnings

cargo check -p unica-adapter-platform-xml \
  --target x86_64-pc-windows-msvc --lib --tests
  passed

cargo fmt --all -- --check
  passed
git diff --check
  passed
```

The target-local validation fixture covers readable, malformed, oversized, and symlinked unrelated registered descriptors and records zero opens for every unrelated descriptor. Revision fixtures cover form, template, object-module, rights, and other companion changes, plus unchanged unrelated-file stability. Opened-handle tests cover swap-open-swap-back, canonical aliases, missing Windows identity data, explicit width conversion, and failure propagation.

### Files in implementation commit

- `.github/workflows/unica-plugin-release.yml`
- `Cargo.lock`
- `crates/unica-adapter-platform-xml/Cargo.toml`
- `crates/unica-adapter-platform-xml/src/factory.rs`
- `crates/unica-adapter-platform-xml/src/guards.rs`
- `crates/unica-adapter-platform-xml/src/lib.rs`
- `crates/unica-adapter-platform-xml/src/platform_handle.rs`
- `crates/unica-adapter-platform-xml/src/publication.rs`
- `crates/unica-adapter-platform-xml/src/safe_root.rs`
- `crates/unica-adapter-platform-xml/src/versions/v2_20/decoder.rs`
- `crates/unica-adapter-platform-xml/src/versions/v2_20/inspection.rs`
- `crates/unica-adapter-platform-xml/src/versions/v2_20/operations.rs`
- `crates/unica-adapter-platform-xml/src/versions/v2_20/probe.rs`
- `crates/unica-adapter-platform-xml/src/versions/v2_20/projector.rs`
- `crates/unica-adapter-platform-xml/src/versions/v2_20/provider.rs`
- `crates/unica-adapter-platform-xml/src/versions/v2_20/schema.rs`
- `crates/unica-adapter-platform-xml/src/versions/v2_20/semantic_map.rs`
- `crates/unica-adapter-platform-xml/src/versions/v2_20/source_sets.rs`
- `crates/unica-adapter-platform-xml/src/versions/v2_20/validation.rs`
- `crates/unica-adapter-platform-xml/tests/legacy_parity.rs`
- `crates/unica-adapter-platform-xml/tests/specialized_relations.rs`
- `crates/unica-adapter-platform-xml/tests/task7_fix_round1_architecture.rs`
- `crates/unica-adapter-platform-xml/tests/task7_fix_round2_architecture.rs`
- `crates/unica-adapter-platform-xml/tests/task7_fix_round2_lazy_source.rs`
- `crates/unica-adapter-platform-xml/tests/task7_fix_round3_architecture.rs`
- `crates/unica-adapter-platform-xml/tests/task7_fix_round3_lazy_revision.rs`
- `crates/unica-adapter-platform-xml/tests/task7_operational_ports.rs`
- `crates/unica-adapter-platform-xml/tests/unmapped_fact.rs`
- `crates/unica-application/src/commands.rs`
- `crates/unica-application/src/navigation.rs`
- `crates/unica-application/tests/task7_fix_round2_policy.rs`
- `crates/unica-application/tests/task7_operational_policy.rs`
- `crates/unica-coder/src/application/mod.rs`
- `crates/unica-coder/src/application/tool_contracts.rs`
- `crates/unica-coder/src/infrastructure/application_ports.rs`
- `crates/unica-coder/src/infrastructure/format_guard.rs`
- `crates/unica-coder/src/infrastructure/internal_adapters.rs`
- `crates/unica-coder/src/infrastructure/native_operations/cf.rs`
- `crates/unica-coder/src/infrastructure/native_operations/code.rs`
- `crates/unica-coder/src/infrastructure/native_operations/common.rs`
- `crates/unica-coder/src/infrastructure/native_operations/compile_transaction.rs`
- `crates/unica-coder/src/infrastructure/native_operations/meta.rs`
- `crates/unica-coder/src/infrastructure/native_operations/support.rs`
- `crates/unica-coder/src/infrastructure/platform_xml_owner.rs`
- `crates/unica-coder/src/infrastructure/project_sources.rs`
- `crates/unica-coder/src/infrastructure/support_guard.rs`
- `crates/unica-coder/src/infrastructure/tool_context.rs`
- `crates/unica-format-core/src/navigation.rs`
- `crates/unica-format-core/src/ports.rs`
- `crates/unica-format-core/src/value.rs`
- `crates/unica-format-core/tests/public_json_contract.rs`
- `crates/unica-format-core/tests/task7_fix_round2_contracts.rs`
- `crates/unica-format-core/tests/task7_fix_round3_evidence.rs`
- `crates/unica-format-core/tests/task7_operational_ports.rs`

### Remaining native host paths and Task 8 justification

- The mechanical call graph proves that `evaluate_format_guard`, `evaluate_support_guard`, and `validate_meta` cannot reach host native XML readers, parser crates, native layout joins, or native wire literals.
- `native_operations/common.rs` still contains legacy command/composition path setup (`resolve_cf_edit_config_path`, `resolve_cf_read_config_path`, `resolve_cfe_validate_config_path`, and `resolve_existing_path`) and native writer helpers. These are outside the Task 7 roots and move with their Task 8 serializers.
- `native_operations/meta.rs`, `form.rs`, `template.rs`, `help.rs`, `role.rs`, `subsystem.rs`, `interface.rs`, `cf.rs`, `cfe.rs`, `mxl.rs`, `dcs.rs`, and `support.rs` retain native serialization, mutation topology, destination paths, and command-specific legacy writer behavior. Moving serialization now would violate the explicit Task 8 boundary.
- Native strings under tests are adversarial fixtures and recursive public-leak denylist probes, not production policy decisions.

### Residual risks and non-gates

- Windows opened-handle behavior compiled for `x86_64-pc-windows-msvc` and its platform-neutral identity logic is unit tested, but the Windows filesystem race tests were not runtime-executed on this macOS host.
- Full workspace validation was intentionally not run; validation remained within Task 7 and affected Task 5/6 parity scope.
- The repository-wide `scripts/ci/check-rust-platform-boundary.py` is not a scoped gate: it reports a broad existing set of OS-specific adapter, writer, and test modules outside its accepted facade list. The new CI cross-target command is the requested compiling invariant.
- An exploratory stricter workspace clippy command reported existing non-Task-7 lints in `unica-format-core`; it was not used as a scoped gate. The native compile checks above are green.
- No Task 7 scoped test, parity test, formatting check, native compile check, or Windows cross-target compile remains failing.

## Fix Round 4

Base: `b31ee77b6493f239d48e2ed66244c95af36a30a2`

Implementation commits:

- `9946925ad36b53c372d5bfaa33718069ffc5f782` (`fix(task7): close remaining adapter evidence gaps`)
- `d6d09feea2599dc9b36a89ce5da7f3b4327cfb94` (`fix(task7): bind legacy support inspection target`)

Tree hashes:

- base: `94618acd11b280324c810a1bac0c89226b3a24fc`
- primary implementation: `e41d4030675b06971938c11b6c9930aace99d8ae`
- final implementation: `accef9007bf834e578f7354697725fd7b3ff071b`

The controller-owned `progress.md` remained modified, unstaged, and untouched.

### RED

- Platform-neutral Windows identity tests did not compile because no complete
  `FileIdInfo` constructor existed; the old implementation used the narrower
  `BY_HANDLE_FILE_INFORMATION` identity and admitted zero values.
- Injected evidence-finalization tests did not compile because there were no
  operation/publication finalization hooks, and authorability had no closed
  evidence-free unreadable denial.
- The direct multi-register fixture returned `Some(false)` instead of
  `Some(true)` and exposed the global registrar scan.
- Three method-hidden architecture fixtures passed incorrectly: renamed
  inherent method syntax, trait/UFCS dispatch, and nested multi-layer helper
  dispatch were outside the function-only graph.
- `task7_fix_round3_evidence` failed to compile with `E0277`/`E0599` because
  authorability evidence was mandatory and `source_unreadable` did not exist.
- The scoped certification probe failed with
  `unwrap_err()` on `Ok(SupportEvidence { source: Absent, ... })` for an
  out-of-root target. The legacy support port authorized only the root and then
  substituted a fixed relative artifact. After binding the target, the first
  GREEN attempt also showed that a legitimately missing bound artifact was
  incorrectly mapped to unreadable. Both distinctions are now explicit.

### Responsibility closure map

| Concern | Final owner and invariant |
| --- | --- |
| Windows opened-file identity | `platform_handle.rs` uses stable `windows-sys` `GetFileInformationByHandleEx(FileIdInfo)` for volume serial plus the complete 128-bit file ID, and `FileStandardInfo` for link count. Missing, partial, all-zero, or query-failed identity is unavailable. |
| Safe-root/publication identity | `SafeSourceRoot` and publication compare identities derived from opened handles. No zero identity, pathname fallback, or default identity can enter comparison. Unix remains descriptor-relative and no-follow. |
| Operational evidence | Compatibility and validation return typed source-unavailable errors when finalization fails. Authorability can return evidence-free only through the closed unreadable denial; every allowed or ordinary denied result requires real finalized evidence. |
| Publication evidence | Staged-tree evidence is finalized and rechecked atomically before visibility. A finalization failure leaves the prior target visible and creates no cacheable success. |
| Registrar resolution | Compatibility and validation share target-local reference resolution from the selected descriptor and explicit reference graph. Unrelated registered `Document` descriptors are never indexed or opened. |
| Architecture reachability | The Task 7 AST graph includes nested modules, inherent and trait impl methods, default trait methods, closures, associated/UFCS calls, and conservative receiver-method edges. Unknown neutral trait-object calls terminate at the port boundary. |
| Legacy support inspection | The adapter binds and reads the exact requested support artifact under the authorized root. Proven missing-at-bind is `Absent`; containment, identity, race, and read failures remain fail closed. |

### GREEN validation on the final implementation tree

```text
cargo test -p unica-format-core \
  --test public_json_contract \
  --test task7_fix_round2_contracts \
  --test task7_fix_round3_evidence \
  --test task7_operational_ports
  22 passed; 0 failed; 0 ignored

cargo test -p unica-application \
  --test task7_fix_round2_policy \
  --test task7_operational_policy
  7 passed; 0 failed; 0 ignored

cargo test -p unica-adapter-platform-xml \
  --test legacy_parity \
  --test specialized_relations \
  --test task7_fix_round1_architecture \
  --test task7_fix_round2_architecture \
  --test task7_fix_round2_lazy_source \
  --test task7_fix_round3_architecture \
  --test task7_fix_round3_lazy_revision \
  --test task7_operational_ports \
  --test unmapped_fact
  71 passed; 0 failed; 0 ignored
  legacy_parity: 26 passed with source/oracle hash checks enabled
  task7_fix_round3_architecture: 7 passed
  task7_fix_round3_lazy_revision: 4 passed

cargo test -p unica-adapter-platform-xml certification:: --lib
  9 passed; 0 failed; 0 ignored
cargo test -p unica-adapter-platform-xml platform_handle::tests:: --lib
  4 passed; 0 failed; 0 ignored
cargo test -p unica-adapter-platform-xml safe_root::tests:: --lib
  2 passed; 0 failed; 0 ignored
cargo test -p unica-adapter-platform-xml versions::v2_20::provider::tests:: --lib
  6 passed; 0 failed; 0 ignored
cargo test -p unica-adapter-platform-xml \
  versions::v2_20::operations::fix_round3_tests:: --lib
  3 passed; 0 failed; 0 ignored
cargo test -p unica-adapter-platform-xml publication::tests:: --lib
  40 passed; 0 failed; 0 ignored

Scoped total: 164 passed; 0 failed; 0 ignored

cargo check -p unica-format-core -p unica-application -p unica-coder \
  -p unica-adapter-platform-xml --tests
  passed; unica-coder emitted 21 existing dead-code warnings

cargo check -p unica-adapter-platform-xml \
  --target x86_64-pc-windows-msvc --lib --tests
  passed

cargo fmt --all -- --check
  passed
git diff --check
  passed

rg 'failure_evidence|GetFileInformationByHandle\(|identity.*unwrap_or_default'
  platform_handle.rs publication.rs safe_root.rs operations.rs validation.rs
  no matches
```

### Adversarial coverage added

- Windows identity preserves differing high 64 bits and rejects missing volume,
  missing file ID, all-zero file ID, zero volume, zero link count, and query
  failure.
- Authorability, compatibility, validation context, validation, and publication
  inject evidence-finalization failure and cannot return a cacheable successful
  result.
- Malformed, unreadable/symlinked, and oversized unrelated registered
  `Document` descriptors record zero opens; direct and multi-register references
  resolve positively.
- Publication finalization failure preserves the old visible tree and leaves no
  stage, backup, or recovery artifact.
- Architecture negative fixtures hide forbidden logic behind renamed inherent
  methods, trait methods, UFCS, receiver syntax, nested modules, and multiple
  helper layers; the neutral trait-object port fixture remains accepted.
- Legacy support inspection rejects an out-of-root target and proves that an
  unrelated fixed-layout support file cannot substitute for the requested
  artifact.

### Files

Primary implementation commit:

- `crates/unica-adapter-platform-xml/src/guards.rs`
- `crates/unica-adapter-platform-xml/src/platform_handle.rs`
- `crates/unica-adapter-platform-xml/src/publication.rs`
- `crates/unica-adapter-platform-xml/src/safe_root.rs`
- `crates/unica-adapter-platform-xml/src/validation.rs`
- `crates/unica-adapter-platform-xml/src/versions/v2_20/operations.rs`
- `crates/unica-adapter-platform-xml/src/versions/v2_20/validation.rs`
- `crates/unica-adapter-platform-xml/tests/task7_fix_round3_architecture.rs`
- `crates/unica-adapter-platform-xml/tests/task7_fix_round3_lazy_revision.rs`
- `crates/unica-format-core/src/ports.rs`
- `crates/unica-format-core/tests/task7_fix_round3_evidence.rs`

Exact-target follow-up commit:

- `crates/unica-adapter-platform-xml/src/certification.rs`
- `crates/unica-adapter-platform-xml/src/versions/v2_20/inspection.rs`

### Remaining native host paths and Task 8 justification

- The method-complete call graph proves that the Task 7 host entrypoints cannot
  reach host XML readers, parser crates, native layout joins, or native wire
  literals.
- `native_operations/common.rs` retains command/composition source-location
  setup and writer helpers outside the Task 7 policy roots.
- `native_operations/meta.rs`, `form.rs`, `template.rs`, `help.rs`, `role.rs`,
  `subsystem.rs`, `interface.rs`, `cf.rs`, `cfe.rs`, `mxl.rs`, `dcs.rs`, and
  `support.rs` retain only native serialization, mutation topology, destination
  paths, and command-specific writer behavior. Those paths are required by the
  existing writers and move with Task 8; moving them here would move native
  serialization early.
- Native strings remaining in tests are fixtures and recursive leakage probes,
  not application policy or public output.

### Residual risks and non-gates

- The Windows branch is cross-compiled and its identity normalization is tested
  platform-neutrally, but Windows filesystem race behavior was not
  runtime-executed on this macOS host.
- Full workspace validation was intentionally not run.
- An exploratory unfiltered adapter `--lib` run was stopped after it crossed
  Task 7 scope and entered a long-running decoder boundary fixture. It exposed
  unrelated decoder/projector/probe failures whose base provenance was not
  established. Its one Task 7-adjacent certification failure was isolated,
  fixed, and the complete nine-test certification module is GREEN. A
  crate-wide adapter-unit GREEN claim is therefore not made.
- No scoped Task 7 test, affected Task 5/6 parity test, native compile check,
  Windows cross-target check, formatting check, or diff check remains failing.

## Fix Round 5

Base: `56b2fbdd51bf5a23933a5bc9abb8d8af68c245d8`

Implementation commit:
`ad411cf4902bcfe80269ccd6f3823e9eb84ffb2c`
(`fix(task7): correct registrar coverage and reachability`)

Tree hashes:

- base: `6b072e7b04584669b248e3a052036d9a5a7e0411`
- implementation: `d43c8fe6706d721943b7fe3e09a32d80fcf76dc5`

The controller-owned `progress.md` remained modified, unstaged, and untouched.

### RED

- Real Platform XML evidence in `meta.rs` and fixtures showed that
  `Document.Properties.RegisterRecords` is the forward relation. No genuine
  register-side `Recorders` or `Registrars` vocabulary was found.
- The old resolver parsed those invented reverse fields and converted their
  absence on a register descriptor into `Some(false)`, which allowed
  target-local validation to emit a false `RegistrarMissing`.
- The new neutral contract test failed to compile with `E0432`/`E0599` because
  `ValidationCoverage`, `ValidationRelationCoverage`,
  `registrar_coverage`, and `ValidationReport::new_with_coverage` did not exist.
- The first architecture RED run had six expected failures:
  `Self::helper`, block-local alias, block-local glob, trait-default dispatch,
  nested async/alias traversal, and the same-name neutral-port false positive.
- Conservative receiver expansion then exposed unresolved external constructor
  calls. The cause was mechanical: grouped `use ...::{self, ...}` was recorded
  under the alias `self`, local re-exports were treated as owned local types,
  and test-only modules entered the production graph.

### Responsibility closure map

| Concern | Final owner and invariant |
| --- | --- |
| Forward registrar relation | Private 2.20 adapter parsing reads only `Document.Properties.RegisterRecords` and resolves each explicitly named register descriptor. |
| Register-target reverse coverage | Target-local validation does not scan documents and returns `ValidationRelationCoverage::NotEvaluated`; unrelated documents cannot create either success or failure. |
| Complete reverse proof | Only the closed `CompleteMissing` state can produce `RegistrarMissing`. The current target-local adapter never constructs it because it has no complete authorized reverse-reference index. |
| Public validation completeness | `ValidationReport` carries closed `Complete` or `Partial` coverage. A report with partial coverage cannot contain `RegistrarMissing`; constructors and Serde preserve the invariant. |
| Local validation facts | Register descriptor structure, values, ownership, registration, languages, methods, and other local checks still execute. Only the unobserved reverse-relation check is omitted from the check count. |
| Lexical call resolution | The architecture graph snapshots module/block imports, aliases, and globs by lexical scope; nested blocks, closures, and async blocks retain the correct scope. |
| Method reachability | `Self` resolves through the enclosing impl/trait owner; inherent methods, trait impls, default methods, UFCS, receiver methods, nested modules, and helper chains are traversed. |
| Fail-closed graph behavior | Unresolved local calls are violations. Unknown receiver calls connect to every production method with the same name unless the receiver has explicit `dyn *Port` static type evidence. |

### Real-shape registrar tests

- A valid document with two `RegisterRecords` opens both explicitly referenced
  register descriptors.
- A recorder-subordinate register with no invented local recorder fields is
  valid with `NotEvaluated` relation coverage and a `Partial` report.
- A document naming a missing register produces `ReferenceMissing`, not
  `RegistrarMissing`.
- A register referenced by multiple documents does not require opening either
  document during target-local register validation.
- Malformed, oversized, and symlinked/unreadable unrelated documents record
  zero opens and cannot cause a false registrar error.

### GREEN validation

```text
cargo test -p unica-format-core \
  --test public_json_contract \
  --test task7_fix_round2_contracts \
  --test task7_fix_round3_evidence \
  --test task7_operational_ports
  23 passed; 0 failed; 0 ignored

cargo test -p unica-application \
  --test task7_fix_round2_policy \
  --test task7_operational_policy
  7 passed; 0 failed; 0 ignored

cargo test -p unica-adapter-platform-xml \
  versions::v2_20::operations::fix_round3_tests:: --lib
  5 passed; 0 failed; 0 ignored

cargo test -p unica-adapter-platform-xml \
  --test legacy_parity \
  --test specialized_relations \
  --test task7_fix_round1_architecture \
  --test task7_fix_round2_architecture \
  --test task7_fix_round2_lazy_source \
  --test task7_fix_round3_architecture \
  --test task7_fix_round3_lazy_revision \
  --test task7_operational_ports \
  --test unmapped_fact
  79 passed; 0 failed; 0 ignored
  legacy_parity: all 26 passed with source/oracle hash checks enabled
  task7_fix_round3_architecture: 15 passed
  task7_fix_round3_lazy_revision: 4 passed

Scoped total: 114 passed; 0 failed; 0 ignored

cargo check -p unica-format-core -p unica-application -p unica-coder \
  -p unica-adapter-platform-xml --tests
  passed; unica-coder emitted 21 dead-code warnings

cargo check -p unica-adapter-platform-xml \
  --target x86_64-pc-windows-msvc --lib --tests
  passed

cargo fmt --all -- --check
  passed
git diff --check
  passed

rg 'registrar_references|registrar_present|<Recorders>|<Registrars>|\
child\(properties, "Recorders"\)|child\(properties, "Registrars"\)' \
  crates/unica-adapter-platform-xml/src/versions/v2_20 \
  crates/unica-format-core/src
  no matches
```

### Files

- `crates/unica-adapter-platform-xml/src/versions/v2_20/operations.rs`
- `crates/unica-adapter-platform-xml/src/versions/v2_20/validation.rs`
- `crates/unica-adapter-platform-xml/tests/task7_fix_round3_architecture.rs`
- `crates/unica-adapter-platform-xml/tests/task7_fix_round3_lazy_revision.rs`
- `crates/unica-format-core/src/ports.rs`
- `crates/unica-format-core/tests/task7_fix_round3_evidence.rs`
- `crates/unica-format-core/tests/task7_operational_ports.rs`

### Remaining host-native paths and Task 8 justification

- No host source changed in this round. The strengthened production-only call
  graph remains GREEN from `evaluate_format_guard`, `evaluate_support_guard`,
  and `validate_meta`.
- `native_operations/common.rs` still owns command/composition source-location
  setup and writer helpers.
- `native_operations/meta.rs`, `form.rs`, `template.rs`, `help.rs`, `role.rs`,
  `subsystem.rs`, `interface.rs`, `cf.rs`, `cfe.rs`, `mxl.rs`, `dcs.rs`, and
  `support.rs` retain native serialization, mutation topology, destination
  paths, and command-specific writer behavior. These are Task 8 writer
  responsibilities, not Task 7 reads or policy.

### Residual risks and non-gates

- A complete reverse-reference index is intentionally not built by
  target-local validation. Register reports are explicitly partial until a
  separately authorized complete index is introduced.
- The architecture test is a conservative Rust AST graph, not rustc type
  inference. It fails closed for unresolved local calls and exempts only
  explicit neutral trait-object port receivers.
- Windows code was cross-compiled but not runtime-executed on this macOS host.
- Full workspace validation was intentionally not run.
- No scoped Task 7 test, affected Task 5/6 parity test, native compile check,
  Windows cross-target check, formatting check, or diff check remains failing.

## Fix Round 6

Base: `423fda322f5ffad6c0b9412dab197529f2109d54`

Implementation commits:

- `03a1b6000a18678b798b4b8c2f2d623ab4a6e519`
  (`fix(task7): preserve validation completeness and graph provenance`)
- `c9e11d40a92cee9df8e5f6fc0c4c20b984469b96`
  (`fix(task7): close reachability inference gaps`)

Tree hashes:

- base: `2420c986f2a8966dedd145808a95a97da69df52c`
- validation/contracts implementation:
  `08b62437319df745e0f3ba46df2eaf61d73ab02b`
- final implementation: `286bd05de8c77ce3ef1d6effb94bda17fa7cefd3`

The controller-owned `progress.md` remained modified, unstaged, and untouched.
Neither implementation commit was amended.

### RED

- The new core contract test failed to compile with `E0599` because
  `ValidationStatus::Partial` and closed registrar-coverage diagnostic codes
  did not exist. The old public shape could serialize a report with partial
  coverage as `valid`.
- The first AI-facing coder test failed to compile with `E0425` because
  `meta.validate` had no typed-data path capable of carrying validation status,
  coverage, reports, and unavailable diagnostics through the command gateway.
- Initial architecture negatives proved four bypasses: non-path impl self types
  were skipped, a local port-name suffix was accepted, a spoofed port trait was
  accepted, and a receiver call with no candidate silently terminated.
- A stronger post-implementation negative then proved that
  `make().hidden_native_reader()` could still terminate when the local helper's
  return provenance was unknown. Its first run reported zero violations.
- Production-positive graph execution exposed receiver-provenance loss through
  constants, glob-imported helper returns, branch expressions, match payloads,
  ranges, and chained standard-library methods. These were graph-model defects,
  not reasons to allowlist method names.
- The migrated scalar decoder initially duplicated element text by collecting
  both an element's aggregate `text()` and its text child. A real registrar
  fixture therefore produced a false semantic-value finding. Restricting the
  decoder to text nodes restored the approved value semantics.
- One parallel coder validation run had 19/20 passing and lost the expected
  command-presentation warning. The case passed alone and on rerun. Root cause
  was a test fixture directory keyed only by a shared label plus wall-clock
  nanoseconds; an atomic process-local sequence now makes those directories
  collision-free.

### Responsibility closure map

| Concern | Final owner and invariant |
| --- | --- |
| Validation decision | `unica-format-core` exposes the closed `Valid`, `Partial`, and `Invalid` report states. Coverage is closed and cannot contradict status or findings. |
| Operational unavailability | The host public command result uses a separate closed `Unavailable` state with unavailable coverage and a neutral diagnostic; it cannot be represented as valid. |
| Partial registrar coverage | The private 2.20 adapter emits a neutral warning for the semantic registrar-coverage area when reverse coverage is `NotEvaluated` or `Partial`. No path or native vocabulary is exposed. |
| Invalid precedence | Error findings make the result `Invalid` without erasing partial coverage evidence. Complete-valid, partial, invalid, and unavailable remain distinguishable. |
| Constructor and wire invariants | `ValidationReport` construction derives status from closed findings and coverage. Custom deserialization reconstructs and compares the state, rejecting contradictory status, coverage, severity, and registrar combinations. |
| Single/batch command mapping | `meta.validate` maps every core report into one typed public result. Batch aggregation uses the same status/coverage rules and preserves each subject report exactly. |
| Arbitrary impl owners | The architecture graph canonicalizes every `syn::Type`, including references, generic/wrapped paths, tuples, arrays, slices, pointers, trait impls, and nested combinations. No `ItemImpl` is skipped because its self type is not `Type::Path`. |
| Method reachability | Receiver calls enqueue every same-name inherent, trait-impl, and default-trait method. `Self`, UFCS, lexical aliases/globs, nested modules, closures, async blocks, branches, destructuring, and helper returns are tracked. Unknown local/no-candidate receivers fail closed. |
| Neutral port termination | A call terminates only when imports resolve the receiver's trait to an approved trait declaration in `unica_format_core` or the approved `unica_application` port contract and the method belongs to that trait. Names and suffixes are irrelevant. |
| Chained value provenance | Same-name host method bodies are traversed before a chained result is treated as opaque. This avoids borrowing an unrelated method's return type while retaining reachability of every possible local implementation. |
| Test isolation | Parallel `meta.validate` fixtures use a monotonic process-local suffix in addition to time, so one case cannot overwrite or delete another case's authorized source. |

### Public validation contract

- Complete, error-free reports serialize with `status: "valid"` and
  `coverage: "complete"`.
- Target-local register reports whose reverse registrar relation was not
  evaluated serialize with `status: "partial"` and `coverage: "partial"`,
  plus `registrarCoverageNotEvaluated`.
- Reports containing errors serialize with `status: "invalid"`; partial
  coverage remains explicit when applicable.
- Source capture/read failures serialize with `status: "unavailable"` and
  `coverage: "unavailable"`, never `valid`.
- Single-object and one-item batch responses contain identical typed report
  state.
- Recursive public JSON tests reject POSIX paths, Windows paths,
  `Configuration.xml`, namespaces, native tags, and native keys in all four
  outcomes.

### Architecture adversarial coverage

- Negative fixtures cover `impl Trait for &Boundary`, generic wrappers,
  tuples, arrays, slices, `Self::helper`, block-local alias and glob imports,
  inherent methods, trait impl methods, default trait methods, nested
  multi-layer and async helper chains, unknown receivers, and no-candidate
  receivers.
- Spoof fixtures cover local `HostPort`, same-name shadow traits, alias
  spoofing, and re-export spoofing.
- Positive fixtures import actual neutral port traits by exact path, including
  aliases and globs. Explicit neutral trait-object calls terminate without
  suppressing same-named local methods reachable elsewhere.
- No production Task 7 root reaches a host XML reader, parser crate, native
  layout helper, or native wire literal.

### GREEN validation

```text
cargo test -p unica-format-core \
  --test public_json_contract \
  --test task7_fix_round2_contracts \
  --test task7_fix_round3_evidence \
  --test task7_operational_ports \
  --test task7_fix_round6_validation
  27 passed; 0 failed; 0 ignored

cargo test -p unica-application \
  --test task7_fix_round2_policy \
  --test task7_operational_policy
  7 passed; 0 failed; 0 ignored

cargo test -p unica-adapter-platform-xml \
  versions::v2_20::operations::fix_round3_tests:: --lib
  5 passed; 0 failed; 0 ignored

cargo test -p unica-adapter-platform-xml \
  --test legacy_parity \
  --test specialized_relations \
  --test task7_fix_round1_architecture \
  --test task7_fix_round2_architecture \
  --test task7_fix_round2_lazy_source \
  --test task7_fix_round3_architecture \
  --test task7_fix_round3_lazy_revision \
  --test task7_operational_ports \
  --test unmapped_fact
  83 passed; 0 failed; 0 ignored
  legacy_parity: all 26 passed
  task7_fix_round3_architecture: 19 passed
  task7_fix_round3_lazy_revision: 4 passed

cargo test -p unica-coder meta_validation_ --lib
  3 passed; 0 failed; 0 ignored
cargo test -p unica-coder validate_meta_ --lib
  20 passed; 0 failed; 0 ignored
cargo test -p unica-coder \
  meta_validate_typed_gateway_exposes_closed_validation_json --lib
  1 passed; 0 failed; 0 ignored

Scoped total: 146 passed; 0 failed; 0 ignored

cargo check -p unica-format-core -p unica-application -p unica-coder \
  -p unica-adapter-platform-xml --tests
  passed; unica-coder emitted 21 existing dead-code warnings

cargo check -p unica-adapter-platform-xml \
  --target x86_64-pc-windows-msvc --lib --tests
  passed

cargo fmt --all -- --check
  passed
git diff --check
  passed
```

### Files

- `Cargo.lock`
- `crates/unica-adapter-platform-xml/Cargo.toml`
- `crates/unica-adapter-platform-xml/src/versions/v2_20/operations.rs`
- `crates/unica-adapter-platform-xml/src/versions/v2_20/validation.rs`
- `crates/unica-adapter-platform-xml/tests/task7_fix_round3_architecture.rs`
- `crates/unica-coder/src/infrastructure/native_operations/meta.rs`
- `crates/unica-coder/src/infrastructure/native_operations/typed_result.rs`
- `crates/unica-format-core/src/ports.rs`
- `crates/unica-format-core/tests/task7_fix_round6_validation.rs`
- `crates/unica-format-core/tests/task7_operational_ports.rs`
- `.superpowers/sdd/2026-07-26-versioned-format-adapter-boundary/task-7-report.md`

### Remaining host-native paths and Task 8 justification

- Task 7 policy roots are mechanically GREEN: `evaluate_format_guard`,
  `evaluate_support_guard`, and `validate_meta` cannot reach host XML parsing,
  native layout joins, or native wire matching.
- `native_operations/common.rs` retains command/composition source-location
  setup and writer helpers only. No moved guard or validation read traverses
  those helpers.
- `native_operations/meta.rs`, `form.rs`, `template.rs`, `help.rs`, `role.rs`,
  `subsystem.rs`, `interface.rs`, `cf.rs`, `cfe.rs`, `mxl.rs`, `dcs.rs`, and
  `support.rs` retain native serialization, mutation topology, destination
  paths, and command-specific writer behavior. These are the existing writer
  implementations and move in Task 8; moving them in Task 7 would violate the
  approved sequencing.
- `Configuration.xml` joins that remain in host code are writer
  source/destination-location paths outside all moved read/guard/validation
  call paths. Architecture tests traverse callers rather than allowlisting
  helper names.
- Native strings in tests are fixtures and recursive leakage denylist probes,
  not application policy or public output.

### Residual risks and non-gates

- The architecture test is a conservative Rust AST/module call graph rather
  than rustc type inference. It compensates by traversing every same-name local
  method and failing closed on unresolved local/unknown calls; only
  provenance-resolved neutral trait methods terminate.
- Windows code cross-compiles, but Windows filesystem behavior was not
  runtime-executed on this macOS host.
- Full workspace validation was intentionally not run.
- No scoped Task 7 test, affected Task 5/6 parity test, host compile check,
  Windows cross-target check, formatting check, or diff check remains failing.

## Fix Round 7

Base: `b6c7aaa1606303ac33e83b30237f2369a6bf5a88`

Implementation commit:
`fc06bd91fea7238663deb32486f3d22f3134ad25`
(`fix(task7): preserve validation evidence and provenance`)

Tree hashes:

- base: `343cd1f7397d50226a9af13cc7e121b943d7ee02`
- implementation: `0a68c566e99322cc0a2d73298d7d5e48ff690141`

The controller-owned `progress.md` remained modified, unstaged, and untouched.
The implementation commit was not amended.

### RED

- The new core test failed with `E0432` because
  `ValidationErrorTruncation` did not exist and with `E0599` because
  `ValidationOptions::max_errors` and
  `ValidationReport::new_with_coverage_and_truncation` did not exist.
- The coder tests failed with `E0432`/`E0425` because there was no closed batch
  aggregate or report normalizer. The old loop imperatively overwrote status
  and coverage according to input order.
- The architecture suite ran 22 tests with 19 passing and three intentional
  RED groups failing: parameter shadowing inherited approval by name, a local
  module named `unica_format_core` was accepted as external, and a real
  multi-hop external alias chain was not resolved.
- The first integration fixture made the object's semantic name empty and
  therefore correctly produced `Unavailable` through failed owner resolution.
  The fixture was corrected to use a duplicate child identity as its second
  local error, preserving an available validation context.
- After dependency-backed resolution was introduced, the positive alias tests
  exposed that workspace dependency keys were parsed as
  `unica_format_core.workspace`. Parsing the package key before the Cargo
  dotted subkey fixed the root without adding a name allowlist.

### Responsibility closure map

| Concern | Final owner and invariant |
| --- | --- |
| Error limit contract | `unica-format-core::ValidationOptions` names `max_errors`, accepts `0..=1000`, and no longer describes a generic finding limit. |
| Truncation evidence | `ValidationReport` carries closed `ValidationErrorTruncation::{Complete, Truncated}`. `Truncated` necessarily derives `Invalid`, including when zero error findings are retained. |
| Mandatory completeness evidence | Adapter validation limits only truncatable error findings. Registrar coverage warnings and `SourceUnreadable` remain present regardless of `maxErrors`. |
| Partial plus errors | Partial coverage retains exactly one neutral registrar coverage finding. Retained or omitted errors produce `Invalid` while coverage remains `Partial`; truncation never changes the result to `Unavailable`. |
| Finding order | The core constructor canonicalizes mandatory coverage evidence first, source-unreadable evidence second, retained errors third, and warnings last, with closed code ordering inside each class. |
| Wire validation | Existing constructors default to complete error evidence. The explicit constructor and custom Serde reject status, coverage, severity, and truncation contradictions. |
| Batch algebra | The host uses the five legal lattice elements `Valid`, `Partial`, `InvalidComplete`, `InvalidPartial`, and `Unavailable`. Join is commutative and associative; `Unavailable` is top, then invalid status while partial coverage is preserved, then partial, then valid. |
| Batch normalization | Reports are canonically ordered before rendering. Diagnostics are sorted and deduplicated by semantic subject plus closed finding, so input permutations are structurally identical without losing per-item identity. |
| Lexical binding identity | Architecture analysis maps each parameter, local, closure, loop, match, and conditional binding to a scope-local binding ID. Neutral methods attach only to the typed parameter's ID; any shadow creates a new unapproved ID. |
| External trait provenance | Approved neutral traits must resolve recursively to actual normal dependencies declared by `unica-coder/Cargo.toml`. Local namespaces take precedence over extern-prelude names. |
| Alias safety | Import aliases and globs resolve recursively with cycle detection. Any local module/re-export hop, unresolved root, local crate-name spoof, or alias cycle is unapproved. |

### Validation truncation and batch coverage

- `maxErrors=0` retains no truncatable errors, retains the mandatory partial
  coverage finding, serializes `errorTruncation: "truncated"`, and remains
  `status: "invalid", coverage: "partial"`.
- `maxErrors=1` retains one of two deterministic local errors, retains the
  coverage finding, and reports truncated errors.
- A larger limit retains both errors and reports complete error evidence.
- Complete, partial, invalid-complete, invalid-partial, and unavailable lattice
  elements are checked across every pair and triple for commutativity and
  associativity.
- Every permutation of all five lattice elements folds to the same
  `Unavailable` result. Invalid-complete joined with partial yields
  invalid-partial.
- Reordered batches, including duplicate reports, produce equal normalized
  structures. Duplicate diagnostics for the same semantic subject are removed;
  the same diagnostic on different subjects remains distinct.

### Architecture adversarial coverage

- Parameter and nested-scope shadowing cannot inherit a neutral port
  exemption.
- A local module named `unica_format_core`, one-hop and two-hop local aliases,
  local glob/re-export facades, and an alias cycle all remain reachable and
  expose their hidden native logic.
- Existing local `HostPort`, shadow-trait, alias, and re-export spoof fixtures
  remain rejected.
- Direct actual external traits, external aliases, external globs, and a
  three-hop actual dependency alias chain terminate only at methods declared
  by the resolved approved trait.
- The production Task 7 roots remain unable to reach host XML readers, parser
  crates, native layout helpers, or native wire literals.

### GREEN validation

```text
cargo test -p unica-format-core \
  --test public_json_contract \
  --test task7_fix_round2_contracts \
  --test task7_fix_round3_evidence \
  --test task7_operational_ports \
  --test task7_fix_round6_validation \
  --test task7_fix_round7_validation
  32 passed; 0 failed; 0 ignored

cargo test -p unica-application \
  --test task7_fix_round2_policy \
  --test task7_operational_policy
  7 passed; 0 failed; 0 ignored

cargo test -p unica-adapter-platform-xml \
  versions::v2_20::operations::fix_round3_tests:: --lib
  5 passed; 0 failed; 0 ignored

cargo test -p unica-adapter-platform-xml \
  --test legacy_parity \
  --test specialized_relations \
  --test task7_fix_round1_architecture \
  --test task7_fix_round2_architecture \
  --test task7_fix_round2_lazy_source \
  --test task7_fix_round3_architecture \
  --test task7_fix_round3_lazy_revision \
  --test task7_operational_ports \
  --test unmapped_fact
  86 passed; 0 failed; 0 ignored
  legacy_parity: all 26 passed
  task7_fix_round3_architecture: 22 passed
  task7_fix_round3_lazy_revision: 4 passed

cargo test -p unica-coder meta_validation_ --lib
  3 passed; 0 failed; 0 ignored
cargo test -p unica-coder validate_meta_ --lib
  20 passed; 0 failed; 0 ignored
cargo test -p unica-coder \
  max_errors_limits_only_errors_and_preserves_partial_coverage --lib
  1 passed; 0 failed; 0 ignored
cargo test -p unica-coder \
  validation_batch_lattice_is_commutative_associative_and_permutation_stable \
  --lib
  1 passed; 0 failed; 0 ignored
cargo test -p unica-coder \
  normalized_batch_is_order_stable_and_dedupes_diagnostics_by_subject --lib
  1 passed; 0 failed; 0 ignored
cargo test -p unica-coder \
  meta_validate_typed_gateway_exposes_closed_validation_json --lib
  1 passed; 0 failed; 0 ignored

Scoped total: 157 passed; 0 failed; 0 ignored

cargo check -p unica-format-core -p unica-application -p unica-coder \
  -p unica-adapter-platform-xml --tests
  passed; unica-coder emitted 21 existing dead-code warnings

cargo check -p unica-adapter-platform-xml \
  --target x86_64-pc-windows-msvc --lib --tests
  passed

cargo fmt --all -- --check
  passed
git diff --check
  passed
```

### Files

- `crates/unica-format-core/src/ports.rs`
- `crates/unica-format-core/tests/task7_fix_round7_validation.rs`
- `crates/unica-adapter-platform-xml/src/versions/v2_20/validation.rs`
- `crates/unica-adapter-platform-xml/tests/task7_fix_round3_architecture.rs`
- `crates/unica-coder/src/infrastructure/native_operations/meta.rs`
- `.superpowers/sdd/2026-07-26-versioned-format-adapter-boundary/task-7-report.md`

### Remaining host-native paths and Task 8 justification

- Task 7 policy roots remain mechanically GREEN: `evaluate_format_guard`,
  `evaluate_support_guard`, and `validate_meta` cannot reach host XML parsing,
  native layout joins, or native wire matching.
- `native_operations/common.rs` retains command/composition source-location
  setup and writer helpers only. No moved guard or validation read traverses
  those helpers.
- `native_operations/meta.rs`, `form.rs`, `template.rs`, `help.rs`, `role.rs`,
  `subsystem.rs`, `interface.rs`, `cf.rs`, `cfe.rs`, `mxl.rs`, `dcs.rs`, and
  `support.rs` retain native serialization, mutation topology, destination
  paths, and command-specific writer behavior. These are Task 8 writer
  responsibilities.
- Remaining `Configuration.xml` joins are writer source/destination-location
  paths outside all moved read/guard/validation call paths.
- Native strings in tests are fixtures and recursive leakage probes, not
  application policy or public output.

### Residual risks and non-gates

- The architecture test remains a conservative AST/module graph rather than
  rustc type inference. It fails closed on unresolved local/unknown calls and
  now derives approved extern roots from the host manifest rather than names.
- Windows code cross-compiles, but Windows filesystem behavior was not
  runtime-executed on this macOS host.
- Full workspace validation was intentionally not run.
- No scoped Task 7 test, affected Task 5/6 parity test, host compile check,
  Windows cross-target check, formatting check, or diff check remains failing.
