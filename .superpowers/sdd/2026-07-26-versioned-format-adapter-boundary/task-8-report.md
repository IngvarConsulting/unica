# Task 8 Report: Platform XML Writer Migration

Date: 2026-07-28

Branch: `codex/versioned-source-adapter-design`

Task 7 base: `a8b2286ea2b47cb211d76593c20c543d954b7041`

## Outcome

Task 8 is implemented.

Every pre-existing Platform XML writer and read-modify-write implementation is
owned by `unica-adapter-platform-xml` under the private
`versions/v2_20/writers` tree. The host no longer owns a Platform XML parser,
serializer, native layout registry, platform-version constant, or source and
destination topology rule for these mutations.

The write boundary now has three distinct layers:

1. `unica-coder` converts public MCP arguments and workspace context into
   format-neutral semantic commands, source roles, and opaque adapter inputs.
2. `unica-format-core` owns closed immutable command, result, and port types.
3. `unica-adapter-platform-xml` resolves source roles to private paths, converts
   semantic arguments into the existing native invocation shape, enforces
   format and filesystem policy, and executes private v2.20 writers.

`AdapterOutcome` exists only at the host boundary. Adapter-private writer
results are named `NativeWriterResult`, converted to `WriterResult`, and then
mapped by the host to the exact existing public result shape.

Serialized form and DCS definitions are captured as adapter inputs. They never
enter `unica-format-core`. Core contains no `Path`, MCP parameter name,
`serde_json::Map`, XML name, platform format version, `AdapterOutcome`, or
serialized native payload.

## Family Responsibility Map

| Family | Neutral command intents | Adapter-private owner | Host responsibility after Task 8 |
| --- | --- | --- | --- |
| Shared helpers and transactions | all writer commands | `common.rs`, `compile_transaction.rs`, `filesystem.rs`, `single_file_publisher.rs`, `registry.rs`, source-root and owner modules | Public result mapping and BSL-only transaction helpers |
| CF | `configuration.initialize`, `configuration.edit` | `writers/cf.rs` | MCP argument normalization only |
| CFE | `extension.initialize`, `extension.borrow`, `extension.patchMethod` | `writers/cfe.rs`, `writers/module_locator.rs` | MCP mapping; BSL patch text remains host-side |
| External EPF/ERF | `externalArtifact.initializeProcessor`, `externalArtifact.initializeReport` | `writers/external.rs` | MCP argument normalization only |
| Metadata | `metadata.create`, `metadata.edit`, `metadata.remove` | `writers/meta.rs` | Closed host diagnostic mapping and reference orchestration |
| Forms | `form.create`, `form.compile`, `form.edit`, `form.remove` | `writers/form.rs`, `writers/form_edit.rs`, `writers/form_event_registry.rs` | MCP mapping only |
| Templates | `template.create`, `template.remove` | `writers/template.rs` | MCP mapping only |
| Help | `help.create` | `writers/help.rs` | MCP mapping only |
| Interfaces | `interface.edit` | `writers/interface.rs` | MCP mapping only |
| Roles | `role.create` | `writers/role.rs` | MCP mapping only |
| Subsystems | `subsystem.create`, `subsystem.edit` | `writers/subsystem.rs` | MCP mapping only |
| Support | `support.edit` | `writers/support.rs` | MCP mapping only |
| DCS | `dataComposition.create`, `dataComposition.edit` | `writers/dcs.rs` | MCP mapping only |
| MXL | `spreadsheet.create` | `writers/mxl.rs` | MCP mapping only |

The DCS edit path has a regression test proving that neutral source role
`Definition` is translated to the private `definitionFile` key only inside the
adapter.

## Core Command and Port Contract

`unica-format-core/src/commands/mod.rs` defines:

- `WriterCommand`, backed by a private closed command kind.
- `WriterArguments`, with immutable construction and duplicate-kind rejection.
- Closed `WriterArgument`, `WriterSourceRole`, `WriterBorrowScope`,
  `WriterFamily`, `WriterIntent`, and `WriterMode` enums.
- Semantic scalar operands for names, references, query text, property values,
  and other user data. These are not serialized native documents.
- `WriterCommandError` for invalid command construction.

`unica-format-core/src/ports.rs` defines the writer, validator, and neutral BSL
artifact locator ports. BSL source mutation remains in the host. The locator
returns a semantic module artifact and does not expose Platform XML topology.

The writer factory accepts typed `(WriterSourceRole, PathBuf)` bindings.
Inspection keeps its separate Task 7 session path and cannot be used as the
writer-port raw-map escape hatch.

## Preservation Matrix

The keyword counts below are overlapping named-test evidence within the final
834-test private writer suite.

| Invariant | Before Task 8 | After Task 8 evidence |
| --- | --- | --- |
| Family semantics | Host-native implementation and tests | `legacy_parity`: 26/26; private writers: 834/834 |
| Dry run | Host behavior | 8 named private tests plus Task 8 port tests |
| Support guard | Host/native guard chain | 47 named private tests; unchanged host failure baseline |
| Format/no-downgrade guard | Host/native guard chain | 34 format tests; Task 7 architecture and ports remain green |
| Authorability guard | Host/native guard chain | 5 named private tests |
| Cancellation | Implicit host lifecycle | `task8_cancelled_writer_cannot_publish_or_validate_native_arguments` |
| Atomic publication | Native host publisher | 5 named private tests in adapter |
| Rollback | Native host transaction | 5 named private tests in adapter |
| Recovery | Native host transaction | 4 named private tests in adapter |
| Partial-write failure | Native host writer tests | 10 named private tests in adapter |
| Concurrency and lock identity | Native host writer tests | 24 named private tests in adapter |
| Symlink handling | Native host filesystem checks | 11 named private tests plus BSL locator rejection test |
| Windows reparse handling | Native host filesystem checks | 4 named private tests and Windows cross-compile |
| Path containment | Native host guards | 10 named private tests |
| File modes | Native host publisher | 1 named private test |
| Idempotency | Native host writers | 2 named private tests |
| Task 5 parity | Adapter reader parity | 26/26 |
| Task 6 relation graph | Adapter relations | 7/7 |
| Task 7 privacy/evidence/lifecycle | Core and adapter ports | all selected Task 7 suites green |

Private writer test distribution:

| Module | Tests |
| --- | ---: |
| `cf` | 26 |
| `cfe` | 82 |
| `common` | 18 |
| `compile_transaction` | 45 |
| `dcs` | 67 |
| `external` | 12 |
| `filesystem` | 8 |
| `form` | 176 |
| `form_edit` | 3 |
| `form_event_registry` | 24 |
| `help` | 12 |
| `interface` | 12 |
| `meta` | 174 |
| `module_locator` | 2 |
| `mxl` | 26 |
| `platform_xml_owner` | 5 |
| `project_sources` | 17 |
| `role` | 15 |
| `single_file_publisher` | 26 |
| `source_root_types` | 2 |
| `source_roots` | 11 |
| `subsystem` | 31 |
| `support` | 13 |
| `template` | 27 |
| Total | 834 |

## TDD Evidence

### RED 1: writer ownership and preservation

The writer implementation was moved only after Task 8 architecture, port, and
parity coverage was added.

The first adapter-private writer run after the mechanical ownership move was:

```text
780 passed; 54 failed
```

The failures exposed host module assumptions, private registry paths, shared
transaction state, support/format guard ordering, and publication helper
coupling. Fixing those causes reduced the intermediate result to:

```text
825 passed; 9 failed
```

The remaining failures were transaction, publication, and guard integration
gaps. The completed private suite is:

```text
834 passed; 0 failed
```

### RED 2: semantic writer boundary

An architecture review found that the first migration still passed raw
`serde_json::Map` values, public operation labels, and an adapter-local type
named `AdapterOutcome` through the writer boundary. That contradicted the
approved design even though behavioral tests were green.

New RED tests required `WriterArgument`, `WriterArguments`,
`WriterCommandError`, and `WriterSourceRole`; required commands to own immutable
arguments; rejected duplicate arguments; and prohibited raw maps, operation
labels, MCP tool names, paths, native result names, and `AdapterOutcome`.

Initial RED evidence:

```text
unica-format-core/task8_writer_contract:
  unresolved semantic command types and WriterCommand::with_arguments

unica-adapter-platform-xml/task8_writer_architecture:
  raw serde_json::Map exposed by the writer factory
```

Final GREEN evidence:

```text
task8_writer_contract:      5 passed
task8_writer_architecture:  1 passed
task8_writer_ports:         4 passed
DCS semantic role unit:     1 passed
```

## Validation Commands and Results

```text
cargo fmt --all
  PASS

git diff --check
  PASS

cargo test -p unica-format-core --test task8_writer_contract
  5 passed

cargo test -p unica-adapter-platform-xml \
  --test task8_writer_architecture \
  --test task8_writer_ports
  5 passed

cargo test -p unica-adapter-platform-xml \
  --lib semantic_role_tests::data_composition_edit_binds_definition_without_exposing_its_path_to_core
  1 passed

cargo test -p unica-adapter-platform-xml \
  --lib versions::v2_20::writers:: -- --quiet
  834 passed; 196 filtered out

cargo test -p unica-adapter-platform-xml \
  --test legacy_parity \
  --test specialized_relations \
  --test task7_fix_round1_architecture \
  --test task7_fix_round2_architecture \
  --test task7_fix_round2_lazy_source \
  --test task7_fix_round3_architecture \
  --test task7_fix_round3_lazy_revision \
  --test task7_operational_ports -- --quiet
  legacy_parity: 26 passed
  specialized_relations: 7 passed
  Task 7 adapter suites: 50 passed

cargo test -p unica-format-core \
  --test public_json_contract \
  --test task7_fix_round2_contracts \
  --test task7_fix_round3_evidence \
  --test task7_fix_round6_validation \
  --test task7_fix_round7_validation \
  --test task7_fix_round8_validation \
  --test task7_operational_ports -- --quiet
  public_json_contract: 5 passed
  Task 7 core suites: 30 passed

cargo test -p unica-coder --lib -- --quiet
  596 passed; 38 failed; 2 ignored
  The 38 failing test names exactly match Task 7 HEAD and the pre-semantic-port
  Task 8 run. No new host failure was introduced.

cargo check --target x86_64-pc-windows-msvc \
  -p unica-format-core \
  -p unica-adapter-platform-xml
  PASS
```

The full host Windows cross-check reaches external `ring v0.17.14` C
compilation and is blocked on this macOS environment by the missing Windows SDK
header `assert.h`. The format core and the touched adapter compile for
`x86_64-pc-windows-msvc`.

## Implementation Commits

```text
21ab29ec376bcaf0a5f03c914034a39b9116f3f7 refactor: isolate platform xml writers
701e25963a749b5bee362e3cd0a8f96328fddb4a test: cover platform writer cancellation
8ed423ac26c3d80c583c0310e16bd540e81c32bb refactor: bind writers through semantic commands
```

The report commit is intentionally not self-referential; its exact SHA is part
of the final handoff.

## Remaining Host-Native Paths

No remaining host-native path contains a Platform XML parser, serializer,
native Platform XML layout registry, private platform format constant, or
writer source/destination topology.

| Path | Remaining responsibility |
| --- | --- |
| `crates/unica-coder/src/infrastructure/native_operations.rs` | Host dispatch and public failure fallback |
| `crates/unica-coder/src/infrastructure/native_operations/code.rs` | BSL-only code patching through the neutral artifact locator |
| `crates/unica-coder/src/infrastructure/native_operations/common.rs` | Host argument/provenance bridge used by BSL/application logic |
| `crates/unica-coder/src/infrastructure/native_operations/compile_transaction.rs` | BSL-only transaction and guard support |
| `crates/unica-coder/src/infrastructure/native_operations/meta.rs` | Host validation/reference orchestration and closed diagnostic mapping |
| `crates/unica-coder/src/infrastructure/native_operations/registry.rs` | MCP-to-semantic mapping and `WriterResult`-to-`AdapterOutcome` mapping |
| `crates/unica-coder/src/infrastructure/native_operations/single_file_publisher.rs` | BSL-only atomic file publication |
| `crates/unica-coder/src/infrastructure/native_operations/tests.rs` | Host orchestration tests |
| `crates/unica-coder/src/infrastructure/native_operations/typed_result.rs` | Public MCP result/data serialization |

## Exact Changed-File Manifest

```text
Cargo.lock
crates/unica-adapter-platform-xml/Cargo.toml
crates/unica-adapter-platform-xml/src/factory.rs
crates/unica-adapter-platform-xml/src/lib.rs
crates/unica-adapter-platform-xml/src/operations/mod.rs
crates/unica-adapter-platform-xml/src/publication.rs
crates/unica-adapter-platform-xml/src/versions/v2_20/mod.rs
crates/unica-adapter-platform-xml/src/versions/v2_20/operations.rs
crates/unica-adapter-platform-xml/src/versions/v2_20/validation.rs
crates/unica-adapter-platform-xml/src/versions/v2_20/writers/cf.rs
crates/unica-adapter-platform-xml/src/versions/v2_20/writers/cfe.rs
crates/unica-adapter-platform-xml/src/versions/v2_20/writers/common.rs
crates/unica-adapter-platform-xml/src/versions/v2_20/writers/compile_transaction.rs
crates/unica-adapter-platform-xml/src/versions/v2_20/writers/dcs.rs
crates/unica-adapter-platform-xml/src/versions/v2_20/writers/external.rs
crates/unica-adapter-platform-xml/src/versions/v2_20/writers/filesystem.rs
crates/unica-adapter-platform-xml/src/versions/v2_20/writers/form.rs
crates/unica-adapter-platform-xml/src/versions/v2_20/writers/form_edit.rs
crates/unica-adapter-platform-xml/src/versions/v2_20/writers/form_event_registry.rs
crates/unica-adapter-platform-xml/src/versions/v2_20/writers/help.rs
crates/unica-adapter-platform-xml/src/versions/v2_20/writers/interface.rs
crates/unica-adapter-platform-xml/src/versions/v2_20/writers/meta.rs
crates/unica-adapter-platform-xml/src/versions/v2_20/writers/mod.rs
crates/unica-adapter-platform-xml/src/versions/v2_20/writers/module_locator.rs
crates/unica-adapter-platform-xml/src/versions/v2_20/writers/mxl.rs
crates/unica-adapter-platform-xml/src/versions/v2_20/writers/operation_descriptors.rs
crates/unica-adapter-platform-xml/src/versions/v2_20/writers/platform_xml_owner.rs
crates/unica-adapter-platform-xml/src/versions/v2_20/writers/project_source_types.rs
crates/unica-adapter-platform-xml/src/versions/v2_20/writers/project_sources.rs
crates/unica-adapter-platform-xml/src/versions/v2_20/writers/registry.rs
crates/unica-adapter-platform-xml/src/versions/v2_20/writers/role.rs
crates/unica-adapter-platform-xml/src/versions/v2_20/writers/single_file_publisher.rs
crates/unica-adapter-platform-xml/src/versions/v2_20/writers/source_root_types.rs
crates/unica-adapter-platform-xml/src/versions/v2_20/writers/source_roots.rs
crates/unica-adapter-platform-xml/src/versions/v2_20/writers/subsystem.rs
crates/unica-adapter-platform-xml/src/versions/v2_20/writers/support.rs
crates/unica-adapter-platform-xml/src/versions/v2_20/writers/template.rs
crates/unica-adapter-platform-xml/src/versions/v2_20/writers/testing.rs
crates/unica-adapter-platform-xml/tests/task7_fix_round3_architecture.rs
crates/unica-adapter-platform-xml/tests/task8_writer_architecture.rs
crates/unica-adapter-platform-xml/tests/task8_writer_ports.rs
crates/unica-coder/src/application/mod.rs
crates/unica-coder/src/infrastructure/native_operations.rs
crates/unica-coder/src/infrastructure/native_operations/code.rs
crates/unica-coder/src/infrastructure/native_operations/common.rs
crates/unica-coder/src/infrastructure/native_operations/compile_transaction.rs
crates/unica-coder/src/infrastructure/native_operations/meta.rs
crates/unica-coder/src/infrastructure/native_operations/registry.rs
crates/unica-coder/src/infrastructure/native_operations/single_file_publisher.rs
crates/unica-coder/src/infrastructure/native_operations/typed_result.rs
crates/unica-format-core/src/commands/inspection.rs
crates/unica-format-core/src/commands/mod.rs
crates/unica-format-core/src/commands/module_locator.rs
crates/unica-format-core/src/lib.rs
crates/unica-format-core/src/ports.rs
crates/unica-format-core/tests/task8_writer_contract.rs
.superpowers/sdd/2026-07-26-versioned-format-adapter-boundary/task-8-report.md
```

The controller-owned `progress.md` and `task-8-brief.md` are not included.

## Residual Risks

1. The full host suite has 38 inherited failures. The failure-name set is
   unchanged from Task 7 HEAD, but those tests remain red and prevent claiming a
   globally green repository.
2. Existing core suites also retain stale baseline expectations:
   `property_contract` has one inherited failure and `semantic_registry` has
   two inherited failures.
3. The complete `unica-coder` Windows cross-check requires a usable Windows SDK
   C toolchain for `ring`; only the touched core and adapter crates were
   cross-checked successfully here.
4. Existing unused-import, unused-field, and unused-mut warnings remain. They do
   not change behavior and were not expanded into unrelated cleanup.
5. Semantic scalar values remain strings where the public operation accepts
   user-provided names, references, query text, or property values. The command
   and argument language is closed, and serialized native definitions remain
   adapter-owned.

Inherited host failure names:

```text
application::tests::ambiguous_source_set_owner_has_same_structured_failure_for_preview_and_apply
application::tests::cf_edit_add_child_object_prioritizes_newer_existing_target_descriptor
application::tests::cf_edit_rejects_symlink_configuration_without_touching_referent
application::tests::cf_edit_validation_dependencies_block_incompatible_home_page_file
application::tests::cfe_borrow_rejects_edt_config_source_set_target
application::tests::code_patch_apply_is_blocked_for_a_locked_supported_object
application::tests::compile_transaction_and_cf_edit_share_target_lock
application::tests::create_only_initializers_prioritize_exact_newer_planned_xml_targets
application::tests::declared_existing_dcs_output_rejects_wrong_root_before_handler
application::tests::declared_existing_form_output_rejects_wrong_root_before_handler
application::tests::declared_existing_mxl_output_rejects_wrong_root_before_handler
application::tests::declared_form_output_with_nonstandard_suffix_still_blocks_newer_owner
application::tests::detailed_compile_dry_run_rejects_edt_source_set_like_apply
application::tests::detailed_compile_dry_run_rejects_output_escape_like_apply
application::tests::detailed_compile_dry_run_reports_planner_errors_instead_of_masking_them
application::tests::entity_spelled_supported_format_is_invalid_at_the_public_boundary
application::tests::external_init_preview_is_path_guarded_and_source_set_typed
application::tests::external_initializers_validate_every_existing_root_artifact_owner
application::tests::form_compile_dry_run_rejects_edt_source_set_like_apply
application::tests::form_compile_dry_run_rejects_output_escape_like_apply
application::tests::help_add_routes_through_unica_and_creates_help_files
application::tests::incompatible_format_blocks_before_native_handler
application::tests::meta_compile_supports_all_documented_pending_types
application::tests::meta_edit_rejects_ambiguous_or_empty_standalone_metadata_owner_before_handler
application::tests::meta_validate_supports_pipe_separated_batch_paths
application::tests::mutating_cf_edit_blocks_locked_configuration_directory_target
application::tests::mutating_meta_edit_blocks_locked_vendor_object_by_default
application::tests::mutating_native_operation_rejects_output_escape_before_backend_execution
application::tests::mxl_compile_allows_new_standalone_output
application::tests::mxl_compile_blocks_write_inside_older_dump_with_structured_diagnostic
application::tests::native_xml_metadata_tools_reject_edt_source_set_targets
application::tests::numeric_equivalent_noncanonical_format_warns_on_read_and_blocks_public_mutator
application::tests::read_only_path_aliases_warn_for_older_directory_owned_inputs
application::tests::support_edit_set_editable_updates_object_rule_and_meta_info
application::tool_contracts::tests::every_native_path_alias_group_normalizes_to_one_canonical_argument
application::tool_contracts::tests::every_published_argument_is_described
infrastructure::source_adapters::registry::registry_tests::pinned_format_and_foreign_probe_identity_fail_closed
infrastructure::source_adapters::registry::registry_tests::typed_identity_fields_fail_closed_without_inspecting_ordinary_data_keys
```

## Fix Round 1

### Base, branch, and commits

- Requested base: `c60349408acb969c10e0d62277839e14fe511164`.
- Branch: `codex/versioned-source-adapter-design`.
- Structural implementation and tests: `b97a8adf124258d12f51c26778b238827de38d1b` (`fix: harden platform XML writer boundary`).
- The controller-owned `progress.md` remained unstaged and unchanged by this fix round. The ignored `task-8-brief.md` was not staged or rewritten.

### Structural resolution

| Finding | Resolution |
|---|---|
| Duplicate publication/locking | Deleted the host `single_file_publisher`; host BSL and semantic source transactions now use the neutral adapter `ArtifactWritePort`. Adapter XML writers, BSL stores, and single-artifact stores converge on the same private `single_file_publisher`, process lock, filesystem lock, preimage guards, rollback, recovery, mode preservation, and cancellation phases. |
| Generic writer envelope | Production mutation dispatch accepts `WriterCommand` directly. `WriterArgument`, arbitrary key/value reconstruction, operation IDs, and the five generic mutation-value carriers were removed. Read-only inspection arguments remain isolated in `inspection_arguments`; legacy map-based writer fixtures are `cfg(test)` only. |
| Unrestricted writer result | Public core outcomes are closed through `WriterLifecycle`, `WriterFailureKind`, `WriterDiagnostic`, `DiagnosticCode`, `DiagnosticDetail`, `SemanticChange`, `SemanticArtifact`, and `WriterEvidence`. Native paths, tags, operation IDs, stdout, and stderr stay private; host-only code maps the typed outcome to `AdapterOutcome`. |
| Cancellation replacement | The registry no longer allocates a token. The public request token is threaded through host mapping, writer dispatch, planning, loops, lock waiting, staging, publication, rollback, and recovery. Cancellation phase and cleanup state are typed. |
| Preservation coverage | The preservation matrix enumerates all 25 `WriterCommand` variants and all 13 writer families. Expected deltas come from an independent semantic oracle rather than writer serialization. Each variant has success, dry-run/no-change, repeat/idempotency, unsupported-state, rollback/cancellation, and concurrent-write cases. |
| Ownership direction | Host `.xml` derivation and platform topology ownership were removed. The adapter exposes opaque semantic locator/read/write leases. CFE method interpretation and BSL generation are host application logic; the adapter only resolves and atomically stores supplied BSL bytes. |
| Changed host failures | All seven base failures called out by review now pass. Detailed planner/owner errors and original guard ordering are retained. The full current host run has no new failed test relative to the base failure set. |
| Windows wording | Touched core and adapter cross-check green. Full host cross-check remains blocked by the missing Windows C toolchain/CRT headers, including both `ring` at `assert.h` and `libsqlite3-sys` at `stdlib.h`; this is not reported as a code pass. |

### Closed command inventory

| Family | `WriterCommand` variants |
|---|---|
| CF | `ConfigurationInitialize`, `ConfigurationEdit` |
| CFE | `ExtensionInitialize`, `ExtensionBorrow`, `ExtensionPatchMethod` |
| External | `ExternalProcessorInitialize`, `ExternalReportInitialize` |
| Metadata | `MetadataCreate`, `MetadataEdit`, `MetadataRemove` |
| Forms | `FormCreate`, `FormCompile`, `FormEdit`, `FormRemove` |
| Templates | `TemplateCreate`, `TemplateRemove` |
| Help | `HelpCreate` |
| Interfaces | `InterfaceEdit` |
| Roles | `RoleCreate` |
| Subsystems | `SubsystemCreate`, `SubsystemEdit` |
| Support | `SupportEdit` |
| DCS | `DataCompositionCreate`, `DataCompositionEdit` |
| MXL | `SpreadsheetCreate` |

The command payloads are immutable validated structs. User-controlled values use purpose-specific validated newtypes. Behavior switches use closed enums, including `ConfigurationMutation`, `MetadataMutation`, `InterfaceEdit`, `SubsystemEdit`, `DataCompositionMutation`, `ExtensionPurpose`, `FormPurpose`, `FormCompileSource`, and `DefaultFormAssignment`. DCS `NoSelection`, interface/subsystem definition sources, and omitted-versus-false form defaults are represented explicitly rather than dropped during translation.

### Responsibility map

| Family | Host/application responsibility | Core responsibility | Private adapter responsibility |
|---|---|---|---|
| CF | Parse MCP parameters; map typed outcome | CF command/value vocabulary | Plan, validate, serialize, transact, publish |
| CFE | Parse request; interpret/generate BSL patch | Extension, borrow, interceptor, and emission semantics | Resolve native artifacts, mutate XML, store supplied BSL atomically |
| External EPF/ERF | Parse public request | Separate processor/report commands | Scaffold and publish platform source |
| Metadata | Parse compile/edit/remove request and preserve detailed planner errors | Closed create/edit/remove DTOs | Native object layout, registry mutation, validation, rollback |
| Forms | Parse definition/object source and assignment intent | Closed create/compile/edit/remove DTOs | Native form ownership, serialization, and publication |
| Templates | Parse create/remove request | Closed template kind and commands | Native template layout and owner update transaction |
| Help | Parse public request and host guard context | `HelpCreate` semantics | Native help artifacts and owner mutation |
| Interfaces | Parse semantic edit | Closed `InterfaceEdit` variants | Native command-interface mutation |
| Roles | Parse rights request | Closed role command | Native rights serialization and validation |
| Subsystems | Parse create/edit request | Closed subsystem operations | Native subsystem serialization and mutation |
| Support | Parse support capability/object rule | Closed support semantics | Native support records, authorability checks, atomic update |
| DCS | Parse create/edit request | Closed DCS mutations | Native DCS reader/writer and validation |
| MXL | Parse compile request | Closed spreadsheet command | Native standalone MXL writer and validation |
| Common/transactions | Request orchestration only | Neutral artifact leases, cancellation, lifecycle | One lock/publication/rollback/recovery implementation |

The remaining production host-native files are:

- `crates/unica-coder/src/infrastructure/native_operations/code.rs`: BSL-only interpretation and generation.
- `crates/unica-coder/src/infrastructure/native_operations/common.rs`: neutral host mapping helpers.
- `crates/unica-coder/src/infrastructure/native_operations/compile_transaction.rs`: neutral artifact transaction wrapper.
- `crates/unica-coder/src/infrastructure/native_operations/meta.rs`: metadata command parameter mapping.
- `crates/unica-coder/src/infrastructure/native_operations/registry.rs`: public request to typed command dispatch.
- `crates/unica-coder/src/infrastructure/native_operations/typed_result.rs`: typed result to public `AdapterOutcome` mapping.
- `crates/unica-coder/src/infrastructure/native_operations/tests.rs`: host boundary tests only.

There is no production host XML parser/serializer, native registry/layout constant, sibling scan, or host publication-lock registry. XML-looking strings found by the final bounded scan occur only in inline test fixtures.

### Preservation matrix

| Family | Independent before/after oracle | Per-variant cases |
|---|---|---|
| CF | semantic configuration projection | success; dry-run/no change; idempotent repeat; unsupported; rollback/cancel; concurrent |
| CFE | extension projection plus host-owned BSL content oracle | success; dry-run/no change; idempotent repeat; unsupported; rollback/cancel; concurrent |
| External | standalone descriptor semantic oracle | success; dry-run/no change; idempotent repeat; unsupported; rollback/cancel; concurrent |
| Metadata | semantic object graph projection | success; dry-run/no change; idempotent repeat; unsupported; rollback/cancel; concurrent |
| Forms | independent form/object projection | success; dry-run/no change; idempotent repeat; unsupported; rollback/cancel; concurrent |
| Templates | independent owner/template projection | success; dry-run/no change; idempotent repeat; unsupported; rollback/cancel; concurrent |
| Help | independent owner/help projection | success; dry-run/no change; idempotent repeat; unsupported; rollback/cancel; concurrent |
| Interfaces | semantic command-interface projection | success; dry-run/no change; idempotent repeat; unsupported; rollback/cancel; concurrent |
| Roles | independent rights projection | success; dry-run/no change; idempotent repeat; unsupported; rollback/cancel; concurrent |
| Subsystems | semantic subsystem projection | success; dry-run/no change; idempotent repeat; unsupported; rollback/cancel; concurrent |
| Support | independent support-state projection | success; dry-run/no change; idempotent repeat; unsupported; rollback/cancel; concurrent |
| DCS | standalone DCS semantic oracle | success; dry-run/no change; idempotent repeat; unsupported; rollback/cancel; concurrent |
| MXL | standalone spreadsheet semantic oracle | success; dry-run/no change; idempotent repeat; unsupported; rollback/cancel; concurrent |

The shared publication-port suite separately injects partial-write, rollback, recovery, cancellation-before-install, cancellation-after-backup, lock-wait cancellation, concurrency, symlink/reparse-style rejection, and path-containment failures. Existing private writer suites retain file-mode, no-downgrade, transaction, and authorability coverage.

### RED/GREEN evidence

RED evidence:

- `cargo test -p unica-adapter-platform-xml --test task8_fix_round1_architecture` initially failed because the production generic writer-argument carrier still existed (`/tmp/task8-fix1-red-generic-writer-envelope.log`).
- After separating inspections, the first architecture assertion also exposed an over-broad test that counted read-only inspection arguments as mutation envelopes; the assertion was narrowed to production writer call paths, not weakened for writers (`/tmp/task8-fix1-architecture-green.log`).
- Typed conversion initially exposed lost DCS `NoSelection`, interface/subsystem definition sources, form source/default semantics, and two CFE fail-closed fixture regressions. The closed enums and test-only legacy mapping were corrected before GREEN.

GREEN evidence:

| Command | Result |
|---|---|
| `cargo fmt --all -- --check` | pass |
| `cargo test -p unica-format-core --test task8_writer_contract` | 5 passed |
| `cargo test -p unica-format-core --test task8_fix_round1_contract` | 4 passed |
| `cargo test -p unica-adapter-platform-xml --test legacy_parity` | 26 passed |
| `cargo test -p unica-adapter-platform-xml --test specialized_relations` | 7 passed |
| Task 7 architecture/lazy/operational integration targets | 5 + 2 + 3 + 25 + 4 + 11 passed |
| `cargo test -p unica-adapter-platform-xml --test task8_writer_architecture` | 1 passed |
| `cargo test -p unica-adapter-platform-xml --test task8_fix_round1_architecture` | 5 passed |
| `cargo test -p unica-adapter-platform-xml --test task8_fix_round1_preservation_matrix` | 2 passed |
| `cargo test -p unica-adapter-platform-xml --features test-support --test task8_writer_ports` | 5 passed |
| 18 exact CFE fail-closed regression filters | 18 passed |
| `cargo test -p unica-adapter-platform-xml --lib -- --nocapture` | 1018 passed; 13 failed; all writer tests passed; residuals reproduced at base individually |
| `cargo test -p unica-coder --lib -- --nocapture` at base | 596 passed; 38 failed; 2 ignored |
| same host command at current | 576 passed; 31 failed; 2 ignored; 27 host-native tests moved with implementation ownership |
| `cargo check --target x86_64-pc-windows-gnu -p unica-format-core -p unica-adapter-platform-xml` | pass |
| `git diff --check` before implementation commit | pass |

Cancellation/locking checks that pass include public pre-cancellation, `call_tool_cancellable` token propagation, mid-plan cancellation, cancellation after first publication with rollback, cancellation before stage install, cancellation after backup, cancellation while waiting for the shared lock, and CF/meta/BSL contention against the common transaction lock. The three host contention tests and the adapter lock-wait test are deterministic passes.

### Seven reviewed host signatures

| Test | Base normalized failure | Current |
|---|---|---|
| `compile_transaction_and_cf_edit_share_target_lock` | CF edit did not contend; timeout assertion | pass |
| `detailed_compile_dry_run_reports_planner_errors_instead_of_masking_them` | generic `unica.meta.compile failed` | pass with detailed typed planner error |
| `help_add_routes_through_unica_and_creates_help_files` | support guard reported unknown state | pass |
| `meta_compile_supports_all_documented_pending_types` | `BusinessProcess` path reached the wrong `GraphicalSchema` root check | pass |
| `meta_validate_supports_pipe_separated_batch_paths` | result omitted native validator detail | pass |
| `mxl_compile_allows_new_standalone_output` | support guard blocked new standalone output | pass |
| `support_edit_set_editable_updates_object_rule_and_meta_info` | expected authorable, received unknown/read-only | pass |

Base/current failure-set comparison found seven fixed tests and no new failed test. All 31 current host failures also fail at the base. Thirty preserve their normalized first failure line. `external_init_preview_is_path_guarded_and_source_set_typed` remains failed at both revisions, but its normalized message changed from `unica.epf.init must target the exact configured source-set root` to `assertion left == right failed`; it is therefore reported as a shared failure with changed signature, not as an unchanged inherited signature.

### Exact residual failures

Current host failures shared with the base (31):
- ``application::tests::ambiguous_source_set_owner_has_same_structured_failure_for_preview_and_apply
- ``application::tests::cf_edit_add_child_object_prioritizes_newer_existing_target_descriptor
- ``application::tests::cf_edit_rejects_symlink_configuration_without_touching_referent
- ``application::tests::cf_edit_validation_dependencies_block_incompatible_home_page_file
- ``application::tests::cfe_borrow_rejects_edt_config_source_set_target
- ``application::tests::code_patch_apply_is_blocked_for_a_locked_supported_object
- ``application::tests::create_only_initializers_prioritize_exact_newer_planned_xml_targets
- ``application::tests::declared_existing_dcs_output_rejects_wrong_root_before_handler
- ``application::tests::declared_existing_form_output_rejects_wrong_root_before_handler
- ``application::tests::declared_existing_mxl_output_rejects_wrong_root_before_handler
- ``application::tests::declared_form_output_with_nonstandard_suffix_still_blocks_newer_owner
- ``application::tests::detailed_compile_dry_run_rejects_edt_source_set_like_apply
- ``application::tests::detailed_compile_dry_run_rejects_output_escape_like_apply
- ``application::tests::entity_spelled_supported_format_is_invalid_at_the_public_boundary
- ``application::tests::external_init_preview_is_path_guarded_and_source_set_typed
- ``application::tests::external_initializers_validate_every_existing_root_artifact_owner
- ``application::tests::form_compile_dry_run_rejects_edt_source_set_like_apply
- ``application::tests::form_compile_dry_run_rejects_output_escape_like_apply
- ``application::tests::incompatible_format_blocks_before_native_handler
- ``application::tests::meta_edit_rejects_ambiguous_or_empty_standalone_metadata_owner_before_handler
- ``application::tests::mutating_cf_edit_blocks_locked_configuration_directory_target
- ``application::tests::mutating_meta_edit_blocks_locked_vendor_object_by_default
- ``application::tests::mutating_native_operation_rejects_output_escape_before_backend_execution
- ``application::tests::mxl_compile_blocks_write_inside_older_dump_with_structured_diagnostic
- ``application::tests::native_xml_metadata_tools_reject_edt_source_set_targets
- ``application::tests::numeric_equivalent_noncanonical_format_warns_on_read_and_blocks_public_mutator
- ``application::tests::read_only_path_aliases_warn_for_older_directory_owned_inputs
- ``application::tool_contracts::tests::every_native_path_alias_group_normalizes_to_one_canonical_argument
- ``application::tool_contracts::tests::every_published_argument_is_described
- ``infrastructure::source_adapters::registry::registry_tests::pinned_format_and_foreign_probe_identity_fail_closed
- ``infrastructure::source_adapters::registry::registry_tests::typed_identity_fields_fail_closed_without_inspecting_ordinary_data_keys

Current adapter read-side failures (13), each replayed individually at the base with the same failure class/message and present in the current full run:

- `versions::v2_20::decoder::direct_type_property_tests::direct_foreign_qname_fails_closed_instead_of_becoming_a_scalar`
- `versions::v2_20::decoder::direct_type_property_tests::unbound_direct_qname_is_rejected_by_type_namespace_resolution`
- `versions::v2_20::decoder::tests::duplicate_inline_child_names_are_identity_collisions`
- `versions::v2_20::decoder::tests::scalar_annotation_rejects_alien_or_conflicting_qnames_locally`
- `versions::v2_20::probe::tests::configuration_unknown_child_fails_closed`
- `versions::v2_20::probe::tests::unknown_metadata_class_fails_closed`
- `versions::v2_20::probe::tests::unknown_nested_structural_features_fail_closed_for_representative_classes`
- `versions::v2_20::projector::tests::empty_annotated_fill_value_preserves_string_but_not_invalid_decimal`
- `versions::v2_20::projector::tests::fill_value_accepts_only_lossless_decimal_or_string_annotations`
- `versions::v2_20::projector::tests::fill_value_uses_exact_native_scalar_annotation_not_text`
- `versions::v2_20::projector::tests::fill_value_without_a_known_annotation_is_unresolved`
- `versions::v2_20::projector::tests::form_is_always_partial_and_inspection_only_before_form_internals_exist`
- `versions::v2_20::projector::tests::malformed_decimal_and_local_scalar_failure_remain_property_local`

Current core residual (one), reproduced exactly at base and current:

- `property_contract::property_definition_registry_is_complete_unique_and_finite`: expected-list mismatch for `EmptyReference`.

### Windows evidence

The touched boundary command is green:

```text
cargo check --target x86_64-pc-windows-gnu -p unica-format-core -p unica-adapter-platform-xml
Finished dev profile
```

The full host command is blocked before Rust host validation because `x86_64-w64-mingw32-gcc` and the Windows CRT headers are unavailable. A clang-fronted run records `ring v0.17.14` failing on `assert.h`. The combined host cross-check also records `libsqlite3-sys v0.30.1` failing on `stdlib.h`; a fresh isolated bundled-SQLite replay stops one include earlier at `stdio.h`. The blocker is therefore the absent Windows C SDK for both dependencies, not only `ring`, and full-host Windows status is blocked rather than green.

### Residual risks

- Full `unica-coder` Windows validation still requires a real MinGW/Windows SDK environment; the touched core and adapter boundary itself cross-checks successfully.
- The 31 shared host failures, 13 shared adapter read-side failures, and one shared core property-registry failure remain outside this fix round. Per-test base evidence was captured rather than inferring inheritance from suite totals.
- One shared host failure has a changed assertion signature as documented above and should be investigated in its owning Task 7/public-boundary follow-up.
- Private adapter and host builds emit existing unused-import/dead-code warnings; they do not change the tested behavior but should be cleaned in a separate warning-focused change.
- Test-only legacy map fixtures remain under `cfg(test)` to preserve legacy regression coverage. Production writer dispatch has no generic argument envelope.
