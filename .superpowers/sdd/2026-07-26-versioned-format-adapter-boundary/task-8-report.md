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

## Fix Round 2

Base: `352e91a8151471c584ee5ee8f7c09c5a3438153c`

Implementation commit: `f6ce6b4e97fa9672e8304e22a7d0541ea0e1da26`

### Responsibility and payload closure

The host now parses public MCP arguments into closed family DTOs. The production adapter dispatch matches `WriterCommand` variants and typed fields directly. It does not reconstruct operation IDs, tool IDs, `serde_json::Map` envelopes, definition JSON, or operation/value pairs. `PlatformWriterSession` contains opaque source bindings and execution context only; compatibility selection is the neutral preserve/adapter-default intent. Cancellation remains on `WriterRequest` and is propagated unchanged through dispatch, planning, transactions, lock waits, publication, rollback, and recovery.

| Family | Closed command variants | Semantic payload owned by core | Native responsibility |
|---|---|---|---|
| Configuration | `ConfigurationInitialize`, `ConfigurationEdit` | name and closed property/home-page/child mutations | private configuration projection and publication |
| Extension | `ExtensionInitialize`, `ExtensionBorrow`, `ExtensionPatchMethod` | extension identity, borrow identity, module role, interception intent, callable/context | private extension locator; host BSL emitter supplies content |
| External | `ExternalProcessorInitialize`, `ExternalReportInitialize` | external artifact identity | private descriptor/module layout and atomic publication |
| Metadata | `MetadataCreate`, `MetadataEdit`, `MetadataRemove` | common and kind-specific definitions, typed properties, children, closed patches | private metadata projection, registration, and native method normalization |
| Form | `FormCreate`, `FormCompile`, `FormEdit`, `FormRemove` | form tree, elements, attributes, commands, events, and closed edits | private form projection and owner locator |
| Template | `TemplateCreate`, `TemplateRemove` | owner, template identity, kind, synonym, main-DCS intent | private owner/template locator and artifact publication |
| Help | `HelpCreate` | owner identity, language, semantic help content intent | private owner locator, help artifacts, and form linkage |
| Interface | `InterfaceEdit` | closed interface replacement and visibility/order edits | private command-interface projection |
| Role | `RoleCreate` | role identity, rights, restrictions, templates | private rights projection |
| Subsystem | `SubsystemCreate`, `SubsystemEdit` | subsystem tree/content and closed property/content/child edits | private subsystem projection and parent resolution |
| Support | `SupportEdit` | closed capability/object-rule enums | private support registry projection |
| DCS | `DataCompositionCreate`, `DataCompositionEdit` | datasets, queries, fields, parameters, settings expressions, and closed edits | private DCS projection |
| MXL | `SpreadsheetCreate` | document, areas, rows, cells, spans, values, and styles | private spreadsheet projection |

The complete closed inventory is 25 variants:

`ConfigurationInitialize`, `ConfigurationEdit`, `ExtensionInitialize`, `ExtensionBorrow`, `ExtensionPatchMethod`, `ExternalProcessorInitialize`, `ExternalReportInitialize`, `MetadataCreate`, `MetadataEdit`, `MetadataRemove`, `FormCreate`, `FormCompile`, `FormEdit`, `FormRemove`, `TemplateCreate`, `TemplateRemove`, `HelpCreate`, `InterfaceEdit`, `RoleCreate`, `SubsystemCreate`, `SubsystemEdit`, `SupportEdit`, `DataCompositionCreate`, `DataCompositionEdit`, and `SpreadsheetCreate`.

Purpose-specific validated newtypes carry user names, descriptions, expressions, code, and help text. Closed enums carry modes, verbs, kinds, support rules, element roles, rights, and compatibility intent. Non-empty/positive wrappers make metadata property changes, metadata clear sets, metadata child changes, DCS parameter order, spreadsheet cell content, and home-page dimensions invariant-safe during construction and deserialization.

### Static and wire guards

`task8_fix_round2_contract` serializes and deserializes every concrete command payload and recursively injects unknown fields. It also rejects unknown variants, property/value mismatches, empty semantic operations, zero dimensions, and empty cell content.

`task8_fix_round2_architecture` proves:

- all 25 host registry arms construct the corresponding closed variant;
- core and adapter production dispatch contain no `WriterArgument`, operation/tool ID carrier, raw definition/session payload, `serde_json::Map`, or generic operation/value reconstruction;
- the adapter dispatch matches variants directly;
- production writers do not serialize commands back into legacy inputs;
- `PlatformWriterSession` contains no compatibility string, operation, value, or definition payload;
- the preservation test executes writer and reader ports and does not scan source/test names as a substitute for behavior.

Test-only legacy fixtures remain behind `cfg(test)` for old private writer regression tests. They are not on the core/adapter production boundary.

### Executable preservation matrix

`task8_fix_round1_preservation_matrix` now executes exactly `WriterCommandKind::ALL` multiplied by `Scenario::ALL`, for 25 variants and 150 required cases. Every case creates an initial fixture, obtains independent before facts, executes the production writer port, reopens a fresh session, obtains after facts through the production semantic reader/projection or a separate standalone oracle, and compares normalized fact multisets with hand-authored expected before/delta/after data.

| Scenario | Required assertion for every variant |
|---|---|
| `Success` | applied lifecycle and exact hand-authored semantic delta |
| `DryRun` | preview lifecycle and unchanged reopened facts |
| `Idempotent` | first apply reaches expected facts; repeat preserves the same facts |
| `Denied` | unsupported/denied source remains byte- and fact-unchanged |
| `Cancelled` | typed cancellation and unchanged facts after rollback/recovery |
| `Concurrent` | both writers serialize through the common lock and converge on expected facts |

Configuration, extension, metadata, form, interface, role, subsystem, template, help, and support cases use production semantic projections. Standalone DCS, MXL, and external cases use independent format-neutral oracles separate from writer code. Expected fact sets are literals in the matrix and are not generated by the writer or its serializer.

The shared publication-port suite additionally covers pre-cancel, mid-write cancellation, cancellation during lock wait, partial-write injection, rollback, recovery, concurrent CF/meta/BSL/single-file contention, file modes, symlink/reparse-style targets, and path containment.

### External public contract

External initialization now returns two path-free typed semantic artifact references: the external object descriptor and its module source, each with a closed artifact kind and object identity. Preview and apply no longer return zero artifacts or expose paths. The unchanged public integration test asserts both artifacts and recursively applies the path/native denylist.

### RED evidence

| Evidence | Initial result |
|---|---|
| `/tmp/task8-fix2-red-serde.log` | 0 passed, 2 failed: unknown fields and empty configuration patches were accepted |
| `/tmp/task8-fix2-red-nested-payloads.log` | 0 passed, 1 failed: property/value mismatches were accepted |
| `/tmp/task8-fix2-red-nested-invariants.log` | 4 passed, 1 failed: empty metadata changes were accepted |
| `/tmp/task8-fix2-red-architecture.log` | 0 passed, 4 failed: raw session payloads, writer reconstruction, and source-name matrix scanning remained |
| `/tmp/task8-fix2-red-architecture-typed-dcs.log` | 1 passed, 5 failed: typed DCS and direct dispatch were incomplete |
| `/tmp/task8-fix2-red-external.log` | 0 passed, 1 failed: external preview returned zero semantic artifacts |

Typed direct-writer conversion also exposed missing form companion projection, information-register periodicity naming, common-module server-call projection, scheduled-job/event-subscription method qualification, legacy subsystem preview defaults, and unqualified owner lookup. Each was fixed in its owning adapter or host parsing layer rather than by reopening the command envelope.

### GREEN evidence

| Command or target set | Result |
|---|---|
| Core Task 7 targets | 30 passed |
| `task8_writer_contract` | 5 passed |
| `task8_fix_round1_contract` | 4 passed |
| `task8_fix_round2_contract` | 5 passed |
| `legacy_parity` | 26 passed |
| `specialized_relations` | 7 passed |
| Adapter Task 7 targets | 50 passed |
| `task8_writer_architecture` | 1 passed |
| `task8_fix_round1_architecture` | 5 passed |
| `task8_fix_round2_architecture` | 6 passed |
| `task8_fix_round1_preservation_matrix` | 1 harness test, all 150 variant/scenario cases |
| `task8_writer_ports` with `test-support` | 5 passed |
| `platform_external_init` | 1 passed |
| `cargo fmt --all -- --check` | pass |
| `git diff --check` | pass |
| `cargo check --target x86_64-pc-windows-gnu -p unica-format-core -p unica-adapter-platform-xml` | pass |

The clean final host command is:

```text
cargo test -p unica-coder --lib -- --nocapture
579 passed; 28 failed; 2 ignored
```

The exact base command produced `576 passed; 31 failed; 2 ignored`. Mechanical set comparison found no current-only failure and these three base failures are fixed:

- `application::tests::external_init_preview_is_path_guarded_and_source_set_typed`
- `application::tests::external_initializers_validate_every_existing_root_artifact_owner`
- `application::tests::incompatible_format_blocks_before_native_handler`

All Task 8 payload, writer, public external, form removal, help, template, metadata compile, planner-error, subsystem-preview, process-cancellation, and shared-lock regressions pass. One loaded full-suite run transiently exceeded the two-second threshold in `bsl_session_initialize_uses_operation_cancellation_for_stuck_and_fragmented_output`; its exact rerun passed in 0.59 seconds and the clean full-suite rerun returned the stable 28-failure set.

### Residual host signatures

All 28 current failures are present in the exact base run. Twenty-three retain the same normalized first panic signature. Five retain the failing test but deliberately use the stable public diagnostic code mapping introduced by the boundary:

| Test group | Base code | Current code | Remaining mismatch |
|---|---|---|---|
| ambiguous source-set owner | `sourceRevisionNewer` | `platformVersionUnsupported` | owner is still classified newer; test expects invalid |
| declared existing DCS/form/MXL wrong root | `sourceMalformed` | `formatVersionInvalid` | compatibility remains `malformed`; tests expect `invalid` |
| nonstandard-suffix newer form owner | `sourceMalformed` | `formatVersionInvalid` | owner is still classified malformed; test expects unsupported-newer |

These are reported as shared failures with changed signatures, not as unchanged inherited failures.

Exact current residuals:

- `application::tests::ambiguous_source_set_owner_has_same_structured_failure_for_preview_and_apply`
- `application::tests::cf_edit_add_child_object_prioritizes_newer_existing_target_descriptor`
- `application::tests::cf_edit_rejects_symlink_configuration_without_touching_referent`
- `application::tests::cf_edit_validation_dependencies_block_incompatible_home_page_file`
- `application::tests::cfe_borrow_rejects_edt_config_source_set_target`
- `application::tests::code_patch_apply_is_blocked_for_a_locked_supported_object`
- `application::tests::create_only_initializers_prioritize_exact_newer_planned_xml_targets`
- `application::tests::declared_existing_dcs_output_rejects_wrong_root_before_handler`
- `application::tests::declared_existing_form_output_rejects_wrong_root_before_handler`
- `application::tests::declared_existing_mxl_output_rejects_wrong_root_before_handler`
- `application::tests::declared_form_output_with_nonstandard_suffix_still_blocks_newer_owner`
- `application::tests::detailed_compile_dry_run_rejects_edt_source_set_like_apply`
- `application::tests::detailed_compile_dry_run_rejects_output_escape_like_apply`
- `application::tests::entity_spelled_supported_format_is_invalid_at_the_public_boundary`
- `application::tests::form_compile_dry_run_rejects_edt_source_set_like_apply`
- `application::tests::form_compile_dry_run_rejects_output_escape_like_apply`
- `application::tests::meta_edit_rejects_ambiguous_or_empty_standalone_metadata_owner_before_handler`
- `application::tests::mutating_cf_edit_blocks_locked_configuration_directory_target`
- `application::tests::mutating_meta_edit_blocks_locked_vendor_object_by_default`
- `application::tests::mutating_native_operation_rejects_output_escape_before_backend_execution`
- `application::tests::mxl_compile_blocks_write_inside_older_dump_with_structured_diagnostic`
- `application::tests::native_xml_metadata_tools_reject_edt_source_set_targets`
- `application::tests::numeric_equivalent_noncanonical_format_warns_on_read_and_blocks_public_mutator`
- `application::tests::read_only_path_aliases_warn_for_older_directory_owned_inputs`
- `application::tool_contracts::tests::every_native_path_alias_group_normalizes_to_one_canonical_argument`
- `application::tool_contracts::tests::every_published_argument_is_described`
- `infrastructure::source_adapters::registry::registry_tests::pinned_format_and_foreign_probe_identity_fail_closed`
- `infrastructure::source_adapters::registry::registry_tests::typed_identity_fields_fail_closed_without_inspecting_ordinary_data_keys`

### Windows cross-check

The touched core and adapter remain green for `x86_64-pc-windows-gnu`. Full-host Windows validation remains environment-blocked by the absent Windows C SDK: `ring` cannot find `assert.h`, and `libsqlite3-sys` cannot find `stdlib.h` (an isolated bundled-SQLite attempt may stop one include earlier at `stdio.h`). This is a blocker for both native dependencies, not only `ring`.

### Residual risks

- The 28 base-shared host failures remain outside the three Fix Round 2 findings; five have the explicitly documented public-code signature change.
- Full-host Windows validation requires a MinGW/Windows SDK environment even though the touched core+adapter boundary cross-check is green.
- Test-only legacy input helpers remain for private parity tests; architecture guards keep them off production dispatch.
- Native vocabulary and source topology remain private to `unica-adapter-platform-xml`; host native-operation code is limited to MCP-to-DTO mapping, typed public-result mapping, orchestration, and BSL-only logic.

## Fix Round 3

Date: 2026-07-28

Base: `b47ef47bdbfaab38ef335d659ad1d07bae17d894`

Implementation commit: `f927c3c97eec61349573151cdd955788bf362bf0`

The controller-owned `progress.md` modification was neither edited nor staged.

### Responsibility map

| Concern | Owner after Fix Round 3 | Evidence |
|---|---|---|
| Public MCP argument parsing | `unica-coder` registry | Maps every documented CF/CFE field into closed core DTOs; no XML/version/path interpretation |
| Compatibility intent | `unica-format-core` | Ordered validated `VersionNumber` and closed `CapabilityRequirement::{Preserve, AdapterDefault, Explicit}` |
| Artifact classification and version policy | adapter-private v2_20 operations/writers | Classification starts from semantic owner/root and parsed descriptor kind; basename is not a classifier |
| Platform XML mutation | adapter-private v2_20 writers | Typed dispatch only; no reconstruction of legacy operation IDs or raw envelopes |
| BSL interpretation/generation | host/application | CFE patch intent is interpreted in host; adapter locates an opaque module artifact and publishes supplied content |
| Atomic publication, locking, rollback/recovery | adapter publication port | One shared lock/publication path for XML, BSL, and standalone artifacts |
| Public `AdapterOutcome` mapping | host | Adapter returns typed lifecycle/status/change/artifact/diagnostic results only |
| Semantic preservation oracle | adapter integration test | Production reader facts where navigation applies plus an independent structural oracle for standalone artifacts |

Remaining host-native paths contain no Platform XML parser/serializer or native layout registry:

- `crates/unica-coder/src/infrastructure/native_operations/code.rs`: BSL-only patching.
- `crates/unica-coder/src/infrastructure/native_operations/common.rs`: host orchestration helpers.
- `crates/unica-coder/src/infrastructure/native_operations/compile_transaction.rs`: host transaction orchestration.
- `crates/unica-coder/src/infrastructure/native_operations/meta.rs`: thin semantic metadata wrapper.
- `crates/unica-coder/src/infrastructure/native_operations/registry.rs`: MCP-to-command mapping and dispatch.
- `crates/unica-coder/src/infrastructure/native_operations/typed_result.rs`: typed adapter-result to public-result mapping.
- `crates/unica-coder/src/infrastructure/native_operations/tests.rs`: host boundary tests.

### Finding 1: owner/family-aware compatibility classification

The basename-based preflight was deleted. Descriptor candidates are classified from their owning semantic root and parsed artifact root before applying a version policy. Create operations preflight the owner and any existing parseable target through the same compatibility port; malformed partial scaffolds remain create-only validation failures.

- DCS and MXL `Template.xml` roots at schema revision `1.0` use DCS/MXL family policy rather than configuration export-version policy.
- `CommandInterface` sidecars are considered with their owning descriptor; a newer owner takes precedence over an older sidecar.
- Same-basename adversarial coverage places metadata, DCS, and MXL `Template.xml` artifacts beside one another and proves different policies.
- The complete private writer namespace is green: `836 passed`, including all 36 base-failing writer tests.

### Finding 2: complete CF/CFE public semantics

Core now owns format-neutral compatibility intent:

- `VersionNumber`: validated two-to-four-component ordered version number, no platform-version string.
- `CapabilityRequirement::Preserve`.
- `CapabilityRequirement::AdapterDefault`.
- `CapabilityRequirement::Explicit(VersionNumber)`.

The host maps CF vendor, configuration version, and compatibility requirement. It maps CFE synonym, purpose, prefix, vendor, configuration version, `noRole`, and compatibility requirement. The adapter emits supplied values exactly. Defaults are applied only when the public request omits a value: CF uses adapter default intent; CFE preserves the base capability.

Round-trip, public-mapping, direct writer, and reader-after-write tests cover preserve/default/explicit compatibility and all CFE vendor/version/role combinations.

### Finding 3: executable full-fact preservation oracle

`task8_fix_round1_preservation_matrix` now executes all 25 `WriterCommand` variants through the production writer port. Each case opens an initial fixture, obtains before facts, writes, reopens a fresh session, obtains after facts, and compares exact multisets and deltas against frozen hand-authored expectations.

The normalized production reader envelope includes:

- envelope status, root identity, consistency, coverage, and diagnostics;
- object identity, kind, capabilities, actions, and visibility;
- every property ID, semantic type, state, value, provenance, and capability;
- facets, actions, and relations.

The independent standalone oracle parses complete element/attribute/text structures for external, DCS, MXL, form/help/interface companion artifacts and BSL content. It does not use writer serializers and has no `contains` booleans. Canonicalization is limited to nondeterministic UUIDs and derived opaque hashes; the complete normalized fact entries and multiplicities remain compared.

Cancellation is injected after the first publication mutation and requires `DuringPublication` plus rollback `Performed` with unchanged facts. Denial uses an actual newer-format or support-locked source. Concurrent cases require both operations to complete under serialization or a deterministic typed conflict, followed by an exact final-fact comparison.

### Typed command inventory and preservation matrix

Every row executes all six scenarios. `Exact delta` means a frozen full before/after/delta fact multiset; `same` means exact before equals exact after.

| `WriterCommand` variant | Semantic oracle | Success | Dry run | Idempotent repeat | Unsupported/denied | Post-mutation cancel | Concurrent |
|---|---|---|---|---|---|---|---|
| `ConfigurationInitialize` | production reader | Exact delta | same | same | same | rollback, same | serialized |
| `ConfigurationEdit` | production reader | Exact delta | same | same | same | rollback, same | serialized |
| `ExtensionInitialize` | production reader | Exact delta | same | same | same | rollback, same | serialized |
| `ExtensionBorrow` | production reader | Exact delta | same | same | same | rollback, same | serialized |
| `ExtensionPatchMethod` | production reader plus independent BSL facts | Exact delta | same | same | same | rollback, same | serialized |
| `ExternalProcessorInitialize` | independent full structure | Exact delta | same | same | same | rollback, same | serialized |
| `ExternalReportInitialize` | independent full structure | Exact delta | same | same | same | rollback, same | serialized |
| `MetadataCreate` | production reader | Exact delta | same | same | same | rollback, same | serialized |
| `MetadataEdit` | production reader | Exact delta | same | same | same | rollback, same | serialized |
| `MetadataRemove` | production reader | Exact delta | same | same | same | rollback, same | serialized |
| `FormCreate` | independent full structure | Exact delta | same | same | same | rollback, same | serialized |
| `FormCompile` | independent full structure | Exact delta | same | same | same | rollback, same | serialized |
| `FormEdit` | independent full structure | Exact delta | same | same | same | rollback, same | serialized |
| `FormRemove` | independent full structure | Exact delta | same | same | same | rollback, same | serialized |
| `TemplateCreate` | production reader | Exact delta | same | same | same | rollback, same | serialized |
| `TemplateRemove` | production reader | Exact delta | same | same | same | rollback, same | serialized |
| `HelpWrite` | independent full structure | Exact delta | same | same | same | rollback, same | serialized |
| `InterfaceEdit` | independent full structure | Exact delta | same | same | same | rollback, same | serialized |
| `RoleCompile` | production reader | Exact delta | same | same | same | rollback, same | serialized |
| `SubsystemCompile` | production reader | Exact delta | same | same | same | rollback, same | serialized |
| `SubsystemEdit` | production reader | Exact delta | same | same | same | rollback, same | serialized |
| `SupportEdit` | production reader | Exact delta | same | same | same | rollback, same | serialized |
| `DataCompositionSchemaCompile` | independent full structure | Exact delta | same | same | same | rollback, same | serialized |
| `DataCompositionSchemaEdit` | independent full structure | Exact delta | same | same | same | rollback, same | serialized |
| `SpreadsheetCompile` | independent full structure | Exact delta | same | same | same | rollback, same | serialized |

Coverage is asserted by exact `(variant, scenario)` keys: 25 variants times 6 scenarios equals 150 required and executed cases. Broad variant matching, test-name scanning, update/dump modes, and expected-value generation from reader/writer output are statically forbidden.

### Finding 4: core-owned metadata applicability

`MetadataKind::ALL` and `MetadataPropertyId::ALL` define a finite 40 by 64 matrix. `metadata_kind_allows_property` is core-owned and is enforced by constructors and deserialization, so invalid combinations cannot reach the adapter.

The exhaustive `2,560`-pair test checks constructor and serde acceptance against the hand-authored applicability table. It explicitly proves `Catalog + Periodicity` rejection and valid `CalculationRegister + Periodicity` and `ScheduledJob + Description` combinations.

### Finding 5: production legacy parser removal

Raw writer reconstructors in CF, DCS, interface, role, and subsystem are deleted from the production compile graph or isolated under `#[cfg(test)]` pending test-fixture removal. Production dispatch does not accept `operation + value`, raw `serde_json::Value`, `serde_json::Map`, tool IDs, native class/tag strings, or raw definition/session payloads. Static architecture tests name each legacy root and require its test-only gate. The remaining subsystem validation map is a validator-internal compatibility path, not a writer command/reconstructor.

### RED evidence

1. Initial full adapter library command:

   `cargo test -p unica-adapter-platform-xml --lib -- --nocapture`

   Result: `982 passed; 49 failed`. The failure set was the 36 writer regressions caused by basename classification plus the pre-existing 13 reader failures listed below.

2. The first full writer rerun exposed four additional owner-preflight regressions not reached by the original 36 and one malformed create-scaffold ordering conflict. Those remained RED until all create targets used owner-aware dependency preflight rather than a filename special case.

3. The first exhaustive metadata matrix exposed two omitted valid pairs, `CalculationRegister + Periodicity` and `ScheduledJob + Description`. Both were added to the core applicability registry; `Catalog + Periodicity` remained rejected.

4. The initial full-fact fixture freeze was nondeterministic across fresh sessions because generated UUIDs, opaque derived hashes, and pre-normalization ordering leaked into the digest. Canonical value normalization and post-normalization sorting made the separately generated rerun deterministic; no update/dump path remains.

5. Static parser-boundary tests were RED while raw CF/DCS/interface/role/subsystem writer parsers remained production reachable. They turned GREEN only after the production compile graph excluded those roots.

### GREEN commands and results

Formatting and patch integrity:

```text
cargo fmt --all -- --check
PASS

git diff --check
PASS
```

Task 8 core contracts:

```text
cargo test -p unica-format-core \
  --test task8_writer_contract \
  --test task8_fix_round1_contract \
  --test task8_fix_round2_contract \
  --test task8_fix_round3_contract
17 passed; 0 failed
```

Task 8 adapter architecture and ports:

```text
cargo test -p unica-adapter-platform-xml --features test-support \
  --test task8_writer_architecture \
  --test task8_fix_round1_architecture \
  --test task8_fix_round2_architecture \
  --test task8_fix_round3_contract \
  --test task8_writer_ports
20 passed; 0 failed
```

Executable preservation matrix:

```text
cargo test -p unica-adapter-platform-xml --features test-support \
  --test task8_fix_round1_preservation_matrix
1 passed; 0 failed; 150 variant/scenario cases executed
```

Complete private writer namespace:

```text
cargo test -p unica-adapter-platform-xml --lib versions::v2_20::writers:: -- --quiet
836 passed; 0 failed; 195 filtered out
```

Task 5 and Task 6 parity/relation scopes:

```text
cargo test -p unica-adapter-platform-xml --test legacy_parity
26 passed; 0 failed

cargo test -p unica-adapter-platform-xml --test specialized_relations
7 passed; 0 failed
```

Task 7 adapter scopes:

```text
cargo test -p unica-adapter-platform-xml --features test-support \
  --test task7_fix_round1_architecture \
  --test task7_fix_round2_architecture \
  --test task7_fix_round2_lazy_source \
  --test task7_fix_round3_architecture \
  --test task7_fix_round3_lazy_revision \
  --test task7_operational_ports
50 passed; 0 failed
```

Task 7 core scopes:

```text
cargo test -p unica-format-core \
  --test public_json_contract \
  --test task7_fix_round2_contracts \
  --test task7_fix_round3_evidence \
  --test task7_fix_round6_validation \
  --test task7_fix_round7_validation \
  --test task7_fix_round8_validation \
  --test task7_operational_ports
35 passed; 0 failed
```

Host full suite, current tree:

```text
cargo test -p unica-coder --lib
581 passed; 28 failed; 2 ignored
```

The current 28 normalized failure names exactly match the 28 names recorded by the Fix Round 2 base run at `b47ef47bdbfaab38ef335d659ad1d07bae17d894`. This round did not reconstruct or relabel a separate base checkout run; the base comparator is the committed Fix Round 2 report, while the current command above was freshly executed on `f927c3c9`. A prior current-tree run had one additional timing-sensitive process test failure; its exact rerun passed, and the final full run above did not reproduce it.

Current host failures:

```text
application::tests::ambiguous_source_set_owner_has_same_structured_failure_for_preview_and_apply
application::tests::cf_edit_add_child_object_prioritizes_newer_existing_target_descriptor
application::tests::cf_edit_rejects_symlink_configuration_without_touching_referent
application::tests::cf_edit_validation_dependencies_block_incompatible_home_page_file
application::tests::cfe_borrow_rejects_edt_config_source_set_target
application::tests::code_patch_apply_is_blocked_for_a_locked_supported_object
application::tests::create_only_initializers_prioritize_exact_newer_planned_xml_targets
application::tests::declared_existing_dcs_output_rejects_wrong_root_before_handler
application::tests::declared_existing_form_output_rejects_wrong_root_before_handler
application::tests::declared_existing_mxl_output_rejects_wrong_root_before_handler
application::tests::declared_form_output_with_nonstandard_suffix_still_blocks_newer_owner
application::tests::detailed_compile_dry_run_rejects_edt_source_set_like_apply
application::tests::detailed_compile_dry_run_rejects_output_escape_like_apply
application::tests::entity_spelled_supported_format_is_invalid_at_the_public_boundary
application::tests::form_compile_dry_run_rejects_edt_source_set_like_apply
application::tests::form_compile_dry_run_rejects_output_escape_like_apply
application::tests::meta_edit_rejects_ambiguous_or_empty_standalone_metadata_owner_before_handler
application::tests::mutating_cf_edit_blocks_locked_configuration_directory_target
application::tests::mutating_meta_edit_blocks_locked_vendor_object_by_default
application::tests::mutating_native_operation_rejects_output_escape_before_backend_execution
application::tests::mxl_compile_blocks_write_inside_older_dump_with_structured_diagnostic
application::tests::native_xml_metadata_tools_reject_edt_source_set_targets
application::tests::numeric_equivalent_noncanonical_format_warns_on_read_and_blocks_public_mutator
application::tests::read_only_path_aliases_warn_for_older_directory_owned_inputs
application::tool_contracts::tests::every_native_path_alias_group_normalizes_to_one_canonical_argument
application::tool_contracts::tests::every_published_argument_is_described
infrastructure::source_adapters::registry::registry_tests::pinned_format_and_foreign_probe_identity_fail_closed
infrastructure::source_adapters::registry::registry_tests::typed_identity_fields_fail_closed_without_inspecting_ordinary_data_keys
```

Full adapter diagnostic run:

```text
cargo test -p unica-adapter-platform-xml --lib -- --nocapture
1018 passed; 13 failed
```

All writer tests pass. The exact remaining failures are pre-existing reader/projection assertions outside the Task 8 writer scope:

```text
versions::v2_20::decoder::direct_type_property_tests::direct_foreign_qname_fails_closed_instead_of_becoming_a_scalar
versions::v2_20::decoder::direct_type_property_tests::unbound_direct_qname_is_rejected_by_type_namespace_resolution
versions::v2_20::decoder::tests::duplicate_inline_child_names_are_identity_collisions
versions::v2_20::decoder::tests::scalar_annotation_rejects_alien_or_conflicting_qnames_locally
versions::v2_20::probe::tests::configuration_unknown_child_fails_closed
versions::v2_20::probe::tests::unknown_metadata_class_fails_closed
versions::v2_20::probe::tests::unknown_nested_structural_features_fail_closed_for_representative_classes
versions::v2_20::projector::tests::empty_annotated_fill_value_preserves_string_but_not_invalid_decimal
versions::v2_20::projector::tests::fill_value_accepts_only_lossless_decimal_or_string_annotations
versions::v2_20::projector::tests::fill_value_uses_exact_native_scalar_annotation_not_text
versions::v2_20::projector::tests::fill_value_without_a_known_annotation_is_unresolved
versions::v2_20::projector::tests::form_is_always_partial_and_inspection_only_before_form_internals_exist
versions::v2_20::projector::tests::malformed_decimal_and_local_scalar_failure_remain_property_local
```

Full core diagnostic run has one pre-existing finite-registry expectation mismatch:

```text
cargo test -p unica-format-core
property_contract::property_definition_registry_is_complete_unique_and_finite
```

The failure expects `EmptyReference` in an older test-owned list; Task 8 scoped core contracts, including all 2,560 applicability pairs, pass.

### Windows cross-check

Touched crates are green:

```text
cargo check --target x86_64-pc-windows-gnu \
  -p unica-format-core -p unica-adapter-platform-xml
PASS
```

The full host cannot be cross-compiled on this macOS runner because the Windows GNU C runtime headers/toolchain are absent. With clang selected, `ring v0.17.14` fails at `fatal error: 'assert.h' file not found`. An isolated bundled `libsqlite3-sys v0.30.1` Windows build first fails at missing `stdio.h`; the reviewer-requested explicit CRT preinclude probe fails at `fatal error: 'stdlib.h' file not found`. These are environment blockers, not touched core/adapter failures.

### Files in implementation commit

```text
crates/unica-adapter-platform-xml/src/factory.rs
crates/unica-adapter-platform-xml/src/versions/v2_20/coverage.json
crates/unica-adapter-platform-xml/src/versions/v2_20/decoder.rs
crates/unica-adapter-platform-xml/src/versions/v2_20/operations.rs
crates/unica-adapter-platform-xml/src/versions/v2_20/projector.rs
crates/unica-adapter-platform-xml/src/versions/v2_20/semantic_map.rs
crates/unica-adapter-platform-xml/src/versions/v2_20/writers/cf.rs
crates/unica-adapter-platform-xml/src/versions/v2_20/writers/cfe.rs
crates/unica-adapter-platform-xml/src/versions/v2_20/writers/common.rs
crates/unica-adapter-platform-xml/src/versions/v2_20/writers/compile_transaction.rs
crates/unica-adapter-platform-xml/src/versions/v2_20/writers/dcs.rs
crates/unica-adapter-platform-xml/src/versions/v2_20/writers/form.rs
crates/unica-adapter-platform-xml/src/versions/v2_20/writers/interface.rs
crates/unica-adapter-platform-xml/src/versions/v2_20/writers/role.rs
crates/unica-adapter-platform-xml/src/versions/v2_20/writers/single_file_publisher.rs
crates/unica-adapter-platform-xml/src/versions/v2_20/writers/subsystem.rs
crates/unica-adapter-platform-xml/src/versions/v2_20/writers/template.rs
crates/unica-adapter-platform-xml/tests/task8_fix_round1_preservation_matrix.rs
crates/unica-adapter-platform-xml/tests/task8_fix_round2_architecture.rs
crates/unica-adapter-platform-xml/tests/task8_fix_round3_contract.rs
crates/unica-coder/src/infrastructure/native_operations/registry.rs
crates/unica-format-core/src/commands/writer_payloads.rs
crates/unica-format-core/tests/task8_fix_round2_contract.rs
crates/unica-format-core/tests/task8_fix_round3_contract.rs
```

### Residual risks

- The 28 host boundary-contract failures remain unchanged from the recorded Fix Round 2 base and need a separate reconciliation with the post-Task-7 public contract; they were not relabeled as inherited successes.
- The 13 adapter reader/projection failures and one core registry-list failure remain exact, visible non-Task-8 residuals.
- Full `unica-coder` Windows GNU validation requires a runner with the Windows GNU C runtime headers for both `ring` and bundled SQLite; touched core+adapter Windows checks are green.
- Test-only legacy parser fixtures remain isolated under `#[cfg(test)]`; production compile-graph guards prevent them from becoming a compatibility surface.


## Fix Round 4

### Baseline and commits

- Requested base: `9425041ec325dcd113f5bc29e92bcc683ba8da6f`.
- Branch: `codex/versioned-source-adapter-design`.
- Implementation commit: `f1328ed2d0d50bc46dc02b3713eaed575daa6182` (`fix(adapter): close Task 8 round 4 findings`).
- This report is committed separately without amend.
- Controller-owned `.superpowers/sdd/2026-07-26-versioned-format-adapter-boundary/progress.md` remained modified, unstaged, and uncommitted.

### Finding 1: family-owned DCS/MXL revision policy

Compatibility now classifies the actual captured qualified root before interpreting its revision. Filename and basename are not inputs to this policy.

| Family | Required qualified root | Revision | Typed decision | Publication |
|---|---|---:|---|---|
| DCS | `DataCompositionSchema` in the DCS schema namespace | `0.x` | `SourceRevisionOlder` | rejected; no implicit upgrade or downgrade |
| DCS | same | `1.0` | compatible | preview/apply allowed |
| DCS | same | `1.1` or later | `SourceRevisionNewer` / public `UnsupportedFormat` | rejected before write |
| MXL | `document` in the spreadsheet namespace | `0.x` | `SourceRevisionOlder` | rejected; no implicit upgrade or downgrade |
| MXL | same | `1.0` | compatible | preview/apply allowed |
| MXL | same | `1.1` or later | `SourceRevisionNewer` / public `UnsupportedFormat` | rejected before write |
| Both | wrong root or namespace | any | `SourceMalformed` | rejected before write |
| Both | missing or non-`major.minor` revision | missing/malformed | `SourceMalformed` | rejected before write |

The writer registry performs the family-owned preflight before both preview and apply. DCS and MXL writer validators retain defense-in-depth root, namespace, and `1.0` checks. Generated DCS and MXL documents and the reviewed 8.3.27 spreadsheet fixture now carry `version="1.0"`.

### Finding 2: authoritative metadata applicability and projection

- Core owns one exhaustive 40-kind applicability registry over all 64 `MetadataKindPropertyName` values.
- `AccountingRegister + EnableTotalsSplitting` is valid and emits `EnableTotalsSplitting` from the typed boolean value instead of a hardcoded default.
- `CalculationRegister + Periodicity` emits native `Periodicity`.
- `InformationRegister + Periodicity` remains the distinct native `InformationRegisterPeriodicity` property.
- Adapter projection validates kind/property applicability before returning a target property.
- `tests/fixtures/task8-metadata-property-targets.json` is an independent reviewed 64-property target crosswalk with the two kind-specific periodicity overrides.
- The adapter unit audit executes all `40 * 64 = 2,560` pairs. The pre-existing independent core matrix owns valid/invalid applicability, while the adapter crosswalk owns target spelling, so synchronized omissions do not pass.
- Mutation assertions reject `EnableTotalSplitting`, calculation-to-information periodicity substitution, and the inverse substitution.

### Finding 3: readable exact semantic oracle

The preservation matrix no longer contains `Sha256`, `fact_set_digest`, `expected_fact_digests`, marker predicates, or before/after digest strings.

Each of the 25 command variants has a readable JSON file under `tests/fixtures/task8-writer-semantic-facts/` with:

- schema version and exact command variant;
- provenance pointing to the approved Task 8 brief, Task 5 contract, and pre-migration v2_20 legacy oracle;
- counted exact removed facts;
- counted exact added facts.

The production reader envelope is normalized without discarding nodes into status/root/consistency, diagnostics, full identity/capability/action/facet state, typed properties and provenance, actions, and relations. Standalone artifacts are independently parsed into every element's ordinal structure, semantic kind, scalar value, attributes, document identity, family, and namespace. UUIDs and opaque content hashes are canonicalized only as identities before exact multiset comparison; no implementation hash is an expected value.

`FormCreate` and `FormRemove` have independent exact add/remove files. DCS/MXL/external/form/interface/help/CFE standalone facts include namespace identity. Mutation tests prove that wrong property value, property type, relation, namespace, addition, removal, and FormRemove direction all fail exact comparison.

### Finding 4: genuine denial and deterministic concurrency

All 150 rows (`25 variants * 6 scenarios`) execute production capture, reader, writer, fresh reopen, exact semantic comparison, and complete XML reparsing.

| Variant family | Denial source |
|---|---|
| CF and configuration-owned meta/form/template/help/interface/role/subsystem/support | structurally valid owner advanced from `2.20` to `2.21` |
| CFE borrow/patch | complete same-family extension owner advanced from `2.20` to `2.21` |
| CFE initialize | structurally valid same-family `2.21` extension descriptor |
| External EPF/ERF initialize | structurally valid same-family `2.21` external descriptor |
| DCS create/edit | valid DCS root and namespace at `1.1` |
| MXL create | valid spreadsheet root and namespace at `1.1` |

Exact denial outcomes are asserted, never `any Rejected`:

- `InvalidRequest`: ConfigurationInitialize, ExtensionInitialize, preserving create-only guard ordering.
- `AlreadyExists`: ExternalProcessorInitialize, ExternalReportInitialize.
- `UnsupportedFormat`: every other variant, including DCS and MXL future roots.

Concurrency uses the adapter publication lock pause plus contention signal. Writer A is held after acquiring the shared lock, writer B must signal contention on that same lock, then A is released. Exact serialized contender outcomes are:

- `InvalidRequest`: ConfigurationInitialize, ExtensionInitialize, FormCreate, HelpCreate.
- `AlreadyExists`: ExternalProcessorInitialize, ExternalReportInitialize, FormEdit, TemplateCreate.
- `NotFound`: MetadataRemove, FormRemove, TemplateRemove.
- `Applied`: all other variants.

Every concurrency row compares final facts to its reviewed success delta and reparses all XML, proving no partial write. Cancellation crosses the publication mutation checkpoint and requires `DuringPublication + Performed` rollback with exact before facts restored.

### RED evidence

| Scope | Command | RED result |
|---|---|---|
| Core applicability | `cargo test -p unica-format-core --test task8_fix_round3_contract` | 2 passed, 1 failed: AccountingRegister rejected EnableTotalsSplitting |
| Oracle architecture | `cargo test -p unica-adapter-platform-xml --features test-support --test task8_fix_round4_architecture` | 0 passed, 2 failed: SHA oracle remained and readable fixtures were absent |
| Revision/mapping behavior | `cargo test -p unica-adapter-platform-xml --features test-support --test task8_fix_round4_contract` | 2 passed, 2 failed: DCS/MXL `0.9` classified compatible and accounting property rejected |
| Preview/apply parity | same focused contract after first implementation | 3 passed, 1 failed: preview bypassed DCS/MXL revision preflight |
| Existing writers | `cargo test -p unica-adapter-platform-xml --lib versions::v2_20::writers::` | 791 passed, 47 failed: valid DCS/MXL test roots lacked required `1.0`; one spreadsheet golden lacked it |
| Old architecture assertion | `cargo test -p unica-adapter-platform-xml --features test-support --test task8_fix_round2_architecture` | 5 passed, 1 failed: test still required deleted digest helpers |

RED logs: `/tmp/task8-fix4-red-core.log`, `/tmp/task8-fix4-red-adapter.log`, `/tmp/task8-fix4-red-adapter-contract.log`, `/tmp/task8-fix4-adapter-writers.log`, and `/tmp/task8-fix4-task8-round2-arch.log`.

### GREEN validation

| Scope | Command/result |
|---|---|
| Formatting | `cargo fmt --all -- --check`: exit 0 |
| Diff hygiene | `git diff --check`: exit 0 |
| Core Task 8 | four test targets: 17 passed, 0 failed |
| Round 4 contracts | contract 4/4; architecture 2/2 |
| Preservation matrix | 2 harness tests green; exact coverage is 150/150 rows |
| Metadata projection | 2/2, including all 2,560 kind/property pairs |
| Private writer suite | 838 passed, 0 failed; requested 836 baseline plus 2 new projection tests |
| Remaining Task 8 adapter suites | 20 passed, 0 failed across writer architecture/ports and Fix Rounds 1-3 |
| Task 5 parity | 26 passed, 0 failed with Python 3.12 `lxml` supplied via `/tmp/task8-fix4-python312` |
| Task 6 relations | 7 passed, 0 failed |
| Task 7 adapter scopes | 50 passed, 0 failed |
| Task 7 core scopes | 35 passed, 0 failed |
| Host external contract | 1 passed, 0 failed |
| Host CF initialization contract | 6 passed, 0 failed, 1 ignored |
| Touched Windows crates | `cargo check --target x86_64-pc-windows-gnu -p unica-format-core -p unica-adapter-platform-xml`: exit 0 |

Final GREEN logs include `/tmp/task8-fix4-core-task8.log`, `/tmp/task8-fix4-adapter-round4-final.log`, `/tmp/task8-fix4-metadata-projection.log`, `/tmp/task8-fix4-adapter-writers-green.log`, `/tmp/task8-fix4-adapter-task8-remaining.log`, `/tmp/task8-fix4-legacy-parity.log`, `/tmp/task8-fix4-task6-relations.log`, `/tmp/task8-fix4-adapter-task7.log`, and `/tmp/task8-fix4-core-task7.log`.

### Exact residual host failures

`cargo test -p unica-coder --lib` remains exactly `581 passed; 28 failed; 2 ignored`:

```text
application::tests::ambiguous_source_set_owner_has_same_structured_failure_for_preview_and_apply
application::tests::cf_edit_add_child_object_prioritizes_newer_existing_target_descriptor
application::tests::cf_edit_rejects_symlink_configuration_without_touching_referent
application::tests::cf_edit_validation_dependencies_block_incompatible_home_page_file
application::tests::cfe_borrow_rejects_edt_config_source_set_target
application::tests::code_patch_apply_is_blocked_for_a_locked_supported_object
application::tests::create_only_initializers_prioritize_exact_newer_planned_xml_targets
application::tests::declared_existing_dcs_output_rejects_wrong_root_before_handler
application::tests::declared_existing_form_output_rejects_wrong_root_before_handler
application::tests::declared_existing_mxl_output_rejects_wrong_root_before_handler
application::tests::declared_form_output_with_nonstandard_suffix_still_blocks_newer_owner
application::tests::detailed_compile_dry_run_rejects_edt_source_set_like_apply
application::tests::detailed_compile_dry_run_rejects_output_escape_like_apply
application::tests::entity_spelled_supported_format_is_invalid_at_the_public_boundary
application::tests::form_compile_dry_run_rejects_edt_source_set_like_apply
application::tests::form_compile_dry_run_rejects_output_escape_like_apply
application::tests::meta_edit_rejects_ambiguous_or_empty_standalone_metadata_owner_before_handler
application::tests::mutating_cf_edit_blocks_locked_configuration_directory_target
application::tests::mutating_meta_edit_blocks_locked_vendor_object_by_default
application::tests::mutating_native_operation_rejects_output_escape_before_backend_execution
application::tests::mxl_compile_blocks_write_inside_older_dump_with_structured_diagnostic
application::tests::native_xml_metadata_tools_reject_edt_source_set_targets
application::tests::numeric_equivalent_noncanonical_format_warns_on_read_and_blocks_public_mutator
application::tests::read_only_path_aliases_warn_for_older_directory_owned_inputs
application::tool_contracts::tests::every_native_path_alias_group_normalizes_to_one_canonical_argument
application::tool_contracts::tests::every_published_argument_is_described
infrastructure::source_adapters::registry::registry_tests::pinned_format_and_foreign_probe_identity_fail_closed
infrastructure::source_adapters::registry::registry_tests::typed_identity_fields_fail_closed_without_inspecting_ordinary_data_keys
```

The relevant host platform corpus remains `25 passed; 1 failed; 1 ignored`. Exact remaining failure:

```text
cfe_patch_method_inventory_covers_atomic_xml_and_bsl_change
unica.form.add failed; errors=["missing required ObjectName argument"]
```

No Task 8 round-4 adapter regression is hidden behind those signatures. Full logs: `/tmp/task8-fix4-host-lib.log` and `/tmp/task8-fix4-host-platform-contracts.log`.

### Windows cross-check

- Core plus adapter touched crates are green for `x86_64-pc-windows-gnu`.
- Host check is blocked in `ring 0.17.14` because the local GNU cross environment lacks `assert.h`.
- Isolated `libsqlite3-sys 0.30.1` first reports missing `stdio.h`; a diagnostic probe exposing only that first header reaches and confirms the additional missing `stdlib.h` blocker.
- These are cross-toolchain/sysroot header blockers, not Rust errors in touched core/adapter code.

Logs: `/tmp/task8-fix4-windows-core-adapter.log`, `/tmp/task8-fix4-windows-host.log`, `/tmp/task8-fix4-windows-sqlite.log`, and `/tmp/task8-fix4-windows-sqlite-stdlib.log`.

### Changed responsibility map

| Area | Responsibility after Fix Round 4 |
|---|---|
| `unica-format-core` | exhaustive metadata kind/property applicability only; no native property names |
| adapter compatibility | private qualified-root and family revision interpretation |
| adapter writer registry | preview/apply family preflight and typed command dispatch |
| private DCS/MXL writers | exact root/namespace/`1.0` defense-in-depth and versioned generation |
| private metadata writer | validated semantic-kind to reviewed native target projection |
| matrix | production reader normalization, independent standalone parsing, exact reviewed facts, cancellation, denial, and shared-lock concurrency |
| host | unchanged neutral-result mapping and public MCP boundary |

### Residual risks

- The 28 host unit failures and one host corpus failure above remain; they are reported per test and were not relabeled as inherited success.
- Task 5 oracle regeneration requires Python 3.12 `lxml`; it is not declared by this Rust change and was supplied only in `/tmp` for validation.
- A complete Windows host build still requires a real MinGW-compatible C sysroot providing both the `ring/assert.h` and SQLite standard-library headers.

## Fix Round 5

### Baseline and commits

- Requested base: `6ee536956731b6c79a2418157249bb4d9743866f`.
- Branch: `codex/versioned-source-adapter-design`.
- Implementation commit: `65c29a5f20e03108961e2c3c90ec3733f1306d8e` (`fix(adapter): close Task 8 round 5 findings`).
- This report is committed separately without amend.
- Controller-owned `.superpowers/sdd/2026-07-26-versioned-format-adapter-boundary/progress.md` remained modified, unstaged, and uncommitted.

### Approved prerequisite test-data correction

During the initial DCS fixture rewrite, one affected Task 8 test fixture contained a duplicated `xmlns:xsi` declaration. The controller explicitly approved correcting that test data before continuing. The duplicate declaration was removed without changing the genuine native DCS root local name, default namespace, or namespace set. This was a prerequisite fixture correction, not a production-format change, and is covered by the final XML parse and generated-root assertions.

### Finding 1: genuine DCS/MXL schema identity

The invented DCS/MXL root `version` attribute has been removed from classification, writer validation, fixtures, and generated output. XML declarations such as `<?xml version="1.0"?>` remain; only the nonexistent root attribute was removed.

The adapter-private policy now starts from the captured qualified root and complete schema signature:

| Family | Current native signature | Decision |
|---|---|---|
| DCS | `DataCompositionSchema` in `http://v8.1c.ru/8.1/data-composition-system/schema`, using only the reviewed DCS/data/schema namespaces | compatible |
| MXL | `document` in `http://v8.1c.ru/8.2/data/spreadsheet`, using only the reviewed spreadsheet/data/schema namespaces | compatible |
| Either | current root namespace plus an unknown descendant or attribute namespace | `SourceRevisionNewer`, fail closed before preview/apply |
| Either | unknown root namespace or missing root namespace | `SourceRevisionNewer`, fail closed as unsupported/possibly newer |
| Either | wrong root local name or malformed XML | `SourceMalformed`, fail closed before preview/apply |

No tracked native spec or local legacy fixture identifies a genuine older DCS/MXL namespace signature. Therefore the reviewed older-signature lists are deliberately empty: this round does not relabel invented `0.x` root attributes as historical evidence. If a genuine older signature is added later, it has a separate explicit `Older` policy and cannot silently downgrade or overwrite. Unknown signatures remain denied rather than guessed.

DCS/MXL create commands now use strict existing-XML-target preflight. An existing malformed destination is no longer skipped as if absent; preview and apply both return the typed malformed rejection. Generic template creation retains its separate arbitrary-payload behavior.

`task8_fix_round4_contract` covers, for both families:

- current spec-shaped no-version input;
- unknown root namespace;
- unknown child schema namespace;
- missing namespace;
- wrong root;
- malformed XML;
- preview/apply parity;
- newly generated output with exact root name/namespace and no root `version` attribute.

The real private DCS/MXL writer tests are included in the final `838/838` writer result.

### Finding 2: published form-add owner selection

`unica.form.add` again works with documented `ObjectPath` alone. Ownership direction is now:

1. Host binds `ObjectPath` only as opaque `WriterSourceRole::Object` evidence.
2. Core carries `FormOwnerSelection::CapturedObject`; it has no path, XML class, directory, or platform vocabulary.
3. Adapter resolves file/directory form-add paths, reads the captured metadata descriptor, derives the neutral `FormOwnerReference`, and validates it before mutation.
4. The public optional `ObjectName` contract is preserved as validated `FormOwnerName`. When supplied, the adapter compares it with the captured owner name after path resolution; when omitted, the captured owner is sufficient.

The host no longer requires `ObjectName` and no longer parses native metadata paths. The public schema proves `ObjectPath` is present, `ObjectName` remains optional, and both path-only and path-plus-name requests validate. The host mapper test proves the optional name reaches the typed DTO. The previously failing corpus case `cfe_patch_method_inventory_covers_atomic_xml_and_bsl_change` is green, and the complete 27-case corpus is `26 passed; 1 ignored`.

### Finding 3: verified readable semantic fixture provenance

All 25 command-variant oracle files remain explicit counted semantic fact JSON. They no longer contain workspace/session IDs, source/object/relation/group keys, parser ordinals, semantic hash placeholders, semantic ID placeholders, document indexes, or captured UUID values.

The documented normalization now:

- retains envelope status, root identity, consistency, coverage, and diagnostics;
- retains semantic identities, kinds, properties, values, states, facets, relations, namespaces, roots, and standalone domain structures;
- recursively unwraps Task 5 typed-value envelopes into readable semantic values;
- omits only transport-generated UUID properties and standalone generated ID elements;
- cancels identical facts from both delta directions, because an unchanged fact is not a semantic delta.

The fixture guard rejects any unchanged fact appearing in both `added` and `removed`. It also rejects missing/unapproved provenance, hash drift, invalid line ranges, unverified claims, and every reader-internal marker above.

Every provenance path is tracked and exists. SHA-256 is recomputed during the architecture test; the reviewed sources are:

```text
spec/designs/2026-07-26-versioned-source-adapter-architecture.md
  87f85c9346b031287041a05cb90238aba6a070e74ee9dd26d0f0467398b4a3db
crates/unica-adapter-platform-xml/tests/legacy_parity.rs
  d901fffef0ca68ef2ee749bdb1c60177a125082e9625b2942e109ee7034c0d3d
crates/unica-adapter-platform-xml/tests/fixtures/v2_20/legacy-oracle/legacy-semantic-oracle.json
  75feda9cab247618c221c35777046681033d1883c3814b21cdb42dc420d1dcbd
plugins/unica/references/specs/1c-dcs-spec.md
  8d97d7be34a41a5493b45030e4f70144b99b25c1204bb0f4279e45aea2ec8bf0
plugins/unica/references/specs/1c-spreadsheet-spec.md
  ffe3648707528e76f03a71c18132ce45485ab67b8efcce4a0120d79663cd5052
```

Each fixture records exact source line ranges and claims; tests verify that every range exists. No production reader/writer update or dump path generates expected files. The committed expectations are readable facts, while production reader observations are created only at test execution and compared as typed multisets.

The preservation matrix remains exhaustive: all 25 `WriterCommand` variants execute success, dry run, idempotent repeat, genuine denial, post-mutation cancellation/rollback, and deterministic same-lock concurrency. Final coverage is exactly `25 * 6 = 150` rows. Mutation tests still prove that wrong value, type, relation, namespace, addition, or removal fails comparison.

### RED evidence

| Scope | RED result |
|---|---|
| Public corpus | `cfe_patch_method_inventory_covers_atomic_xml_and_bsl_change`: `unica.form.add` failed with `missing required ObjectName argument` |
| DCS/MXL source guard | artificial root `version="1.0"` remained in classifier/validators/emitters/fixtures |
| Provenance audit | 97 invalid markers/sources, including nonexistent provenance and reader-internal IDs/hashes |
| First genuine-schema contract | valid no-version create/apply was rejected because private DCS/MXL root validators still required the invented attribute |
| Second genuine-schema contract | malformed existing DCS create target previewed as no-change because create-only preflight skipped malformed XML |
| First provenance architecture run | `<semantic-id>` remained and an overbroad static guard rejected legitimate metadata version handling |
| First full preservation run | stale FormCompile unchanged facts and noncanonical fixture ordering failed exact multiset comparison |
| Public optional-owner contract | schema still published optional `ObjectName`, while the new test incorrectly asserted it absent and the host mapper ignored it |

The approved duplicated-`xmlns:xsi` test-data error was corrected before the GREEN runs and is explicitly recorded above.

### GREEN validation

| Scope | Command/result |
|---|---|
| Formatting | `cargo fmt --all -- --check`: exit 0 |
| Diff hygiene | `git diff --check`: exit 0 |
| Private writer namespace | `cargo test -p unica-adapter-platform-xml --lib versions::v2_20::writers:: -- --quiet`: `838 passed; 0 failed` |
| Preservation matrix | `cargo test -p unica-adapter-platform-xml --features test-support --test task8_fix_round1_preservation_matrix -- --quiet`: 2 harness tests green; `150/150` rows |
| Task 8 core contracts | four targets: `17 passed; 0 failed` |
| Task 8 adapter architecture/contracts/ports | seven targets: `28 passed; 0 failed` |
| Task 5 parity | `legacy_parity`: `26 passed; 0 failed` |
| Task 6 relations | `specialized_relations`: `7 passed; 0 failed` |
| Task 7 adapter scopes | six targets: `50 passed; 0 failed` |
| Task 7 core scopes | seven targets: `35 passed; 0 failed` |
| Host platform corpus | `format_8_3_27_xml_corpus`: `26 passed; 0 failed; 1 ignored` |
| Host form-add typed mapping | exact registry test: `1 passed; 0 failed` |
| Host form-add public contract | exact tool-contract test: `1 passed; 0 failed` |
| Touched Windows crates | `cargo check --target x86_64-pc-windows-gnu -p unica-format-core -p unica-adapter-platform-xml`: exit 0 |

The broad adapter diagnostic command was also rerun:

```text
cargo test -p unica-adapter-platform-xml --features test-support -- --nocapture
1020 passed; 13 failed
```

All 13 failures are outside the writer namespace; the exact names match the base report:

```text
versions::v2_20::decoder::direct_type_property_tests::direct_foreign_qname_fails_closed_instead_of_becoming_a_scalar
versions::v2_20::decoder::direct_type_property_tests::unbound_direct_qname_is_rejected_by_type_namespace_resolution
versions::v2_20::decoder::tests::duplicate_inline_child_names_are_identity_collisions
versions::v2_20::decoder::tests::scalar_annotation_rejects_alien_or_conflicting_qnames_locally
versions::v2_20::probe::tests::configuration_unknown_child_fails_closed
versions::v2_20::probe::tests::unknown_metadata_class_fails_closed
versions::v2_20::probe::tests::unknown_nested_structural_features_fail_closed_for_representative_classes
versions::v2_20::projector::tests::empty_annotated_fill_value_preserves_string_but_not_invalid_decimal
versions::v2_20::projector::tests::fill_value_accepts_only_lossless_decimal_or_string_annotations
versions::v2_20::projector::tests::fill_value_uses_exact_native_scalar_annotation_not_text
versions::v2_20::projector::tests::fill_value_without_a_known_annotation_is_unresolved
versions::v2_20::projector::tests::form_is_always_partial_and_inspection_only_before_form_internals_exist
versions::v2_20::projector::tests::malformed_decimal_and_local_scalar_failure_remain_property_local
```

Full core diagnostic retains one exact base failure:

```text
property_contract::property_definition_registry_is_complete_unique_and_finite
actual includes EmptyReference; the older test-owned expected list omits it
```

Full host library result is `582 passed; 28 failed; 2 ignored`. The additional pass versus the base count is the new form-add mapping test. The exact 28 failure names are unchanged from the base report:

```text
application::tests::ambiguous_source_set_owner_has_same_structured_failure_for_preview_and_apply
application::tests::cf_edit_add_child_object_prioritizes_newer_existing_target_descriptor
application::tests::cf_edit_rejects_symlink_configuration_without_touching_referent
application::tests::cf_edit_validation_dependencies_block_incompatible_home_page_file
application::tests::cfe_borrow_rejects_edt_config_source_set_target
application::tests::code_patch_apply_is_blocked_for_a_locked_supported_object
application::tests::create_only_initializers_prioritize_exact_newer_planned_xml_targets
application::tests::declared_existing_dcs_output_rejects_wrong_root_before_handler
application::tests::declared_existing_form_output_rejects_wrong_root_before_handler
application::tests::declared_existing_mxl_output_rejects_wrong_root_before_handler
application::tests::declared_form_output_with_nonstandard_suffix_still_blocks_newer_owner
application::tests::detailed_compile_dry_run_rejects_edt_source_set_like_apply
application::tests::detailed_compile_dry_run_rejects_output_escape_like_apply
application::tests::entity_spelled_supported_format_is_invalid_at_the_public_boundary
application::tests::form_compile_dry_run_rejects_edt_source_set_like_apply
application::tests::form_compile_dry_run_rejects_output_escape_like_apply
application::tests::meta_edit_rejects_ambiguous_or_empty_standalone_metadata_owner_before_handler
application::tests::mutating_cf_edit_blocks_locked_configuration_directory_target
application::tests::mutating_meta_edit_blocks_locked_vendor_object_by_default
application::tests::mutating_native_operation_rejects_output_escape_before_backend_execution
application::tests::mxl_compile_blocks_write_inside_older_dump_with_structured_diagnostic
application::tests::native_xml_metadata_tools_reject_edt_source_set_targets
application::tests::numeric_equivalent_noncanonical_format_warns_on_read_and_blocks_public_mutator
application::tests::read_only_path_aliases_warn_for_older_directory_owned_inputs
application::tool_contracts::tests::every_native_path_alias_group_normalizes_to_one_canonical_argument
application::tool_contracts::tests::every_published_argument_is_described
infrastructure::source_adapters::registry::registry_tests::pinned_format_and_foreign_probe_identity_fail_closed
infrastructure::source_adapters::registry::registry_tests::typed_identity_fields_fail_closed_without_inspecting_ordinary_data_keys
```

### Windows cross-check

Touched core and adapter crates are green for `x86_64-pc-windows-gnu`.

The current full host check is environment-blocked before Rust host code is checked:

- default compiler discovery fails because `x86_64-w64-mingw32-gcc` is absent;
- with `CC_x86_64_pc_windows_gnu=clang`, `ring 0.17.14` reaches `fatal error: 'assert.h' file not found`;
- the separately established bundled `libsqlite3-sys 0.30.1` probe remains blocked by the missing Windows CRT headers, including `stdlib.h` (and initially `stdio.h`). That isolated SQLite probe was not rerun in Fix Round 5; the wording is retained from the verified Fix Round 4 evidence rather than presented as a new run.

### Files in implementation commit

```text
crates/unica-adapter-platform-xml/src/owner.rs
crates/unica-adapter-platform-xml/src/publication.rs
crates/unica-adapter-platform-xml/src/versions/v2_20/operations.rs
crates/unica-adapter-platform-xml/src/versions/v2_20/writers/common.rs
crates/unica-adapter-platform-xml/src/versions/v2_20/writers/dcs.rs
crates/unica-adapter-platform-xml/src/versions/v2_20/writers/form.rs
crates/unica-adapter-platform-xml/src/versions/v2_20/writers/mxl.rs
crates/unica-adapter-platform-xml/src/versions/v2_20/writers/registry.rs
crates/unica-adapter-platform-xml/tests/fixtures/task8-writer-semantic-facts/ConfigurationEdit.json
crates/unica-adapter-platform-xml/tests/fixtures/task8-writer-semantic-facts/ConfigurationInitialize.json
crates/unica-adapter-platform-xml/tests/fixtures/task8-writer-semantic-facts/DataCompositionCreate.json
crates/unica-adapter-platform-xml/tests/fixtures/task8-writer-semantic-facts/DataCompositionEdit.json
crates/unica-adapter-platform-xml/tests/fixtures/task8-writer-semantic-facts/ExtensionBorrow.json
crates/unica-adapter-platform-xml/tests/fixtures/task8-writer-semantic-facts/ExtensionInitialize.json
crates/unica-adapter-platform-xml/tests/fixtures/task8-writer-semantic-facts/ExtensionPatchMethod.json
crates/unica-adapter-platform-xml/tests/fixtures/task8-writer-semantic-facts/ExternalProcessorInitialize.json
crates/unica-adapter-platform-xml/tests/fixtures/task8-writer-semantic-facts/ExternalReportInitialize.json
crates/unica-adapter-platform-xml/tests/fixtures/task8-writer-semantic-facts/FormCompile.json
crates/unica-adapter-platform-xml/tests/fixtures/task8-writer-semantic-facts/FormCreate.json
crates/unica-adapter-platform-xml/tests/fixtures/task8-writer-semantic-facts/FormEdit.json
crates/unica-adapter-platform-xml/tests/fixtures/task8-writer-semantic-facts/FormRemove.json
crates/unica-adapter-platform-xml/tests/fixtures/task8-writer-semantic-facts/HelpCreate.json
crates/unica-adapter-platform-xml/tests/fixtures/task8-writer-semantic-facts/InterfaceEdit.json
crates/unica-adapter-platform-xml/tests/fixtures/task8-writer-semantic-facts/MetadataCreate.json
crates/unica-adapter-platform-xml/tests/fixtures/task8-writer-semantic-facts/MetadataEdit.json
crates/unica-adapter-platform-xml/tests/fixtures/task8-writer-semantic-facts/MetadataRemove.json
crates/unica-adapter-platform-xml/tests/fixtures/task8-writer-semantic-facts/RoleCreate.json
crates/unica-adapter-platform-xml/tests/fixtures/task8-writer-semantic-facts/SpreadsheetCreate.json
crates/unica-adapter-platform-xml/tests/fixtures/task8-writer-semantic-facts/SubsystemCreate.json
crates/unica-adapter-platform-xml/tests/fixtures/task8-writer-semantic-facts/SubsystemEdit.json
crates/unica-adapter-platform-xml/tests/fixtures/task8-writer-semantic-facts/SupportEdit.json
crates/unica-adapter-platform-xml/tests/fixtures/task8-writer-semantic-facts/TemplateCreate.json
crates/unica-adapter-platform-xml/tests/fixtures/task8-writer-semantic-facts/TemplateRemove.json
crates/unica-adapter-platform-xml/tests/task8_fix_round1_preservation_matrix.rs
crates/unica-adapter-platform-xml/tests/task8_fix_round3_contract.rs
crates/unica-adapter-platform-xml/tests/task8_fix_round4_architecture.rs
crates/unica-adapter-platform-xml/tests/task8_fix_round4_contract.rs
crates/unica-coder/src/application/tool_contracts.rs
crates/unica-coder/src/infrastructure/native_operations/registry.rs
crates/unica-format-core/src/commands/writer_payloads.rs
crates/unica-format-core/tests/task8_fix_round2_contract.rs
tests/fixtures/platform_8_3_27/mxl/Template.xml
```

### Residual risks

- No reviewed source currently identifies a genuine older DCS/MXL namespace signature. Older-signature handling is closed and explicit, but evidence must precede adding any signature.
- The private official-doc corpus was unavailable as a complete local manifest in this worktree. The attempted downloader was stopped before publication because its projected run was hours; this round relies on tracked native specs and legacy fixtures and records that evidence limitation rather than inventing a format marker.
- Semantic normalization intentionally excludes transport-generated UUID values while retaining user-meaningful identities and relations. The exclusion is statically guarded and documented.
- The 13 adapter reader/projection failures, one core finite-list mismatch, and 28 host boundary failures above remain exact non-writer residuals.
- Full host Windows GNU validation requires a MinGW-compatible compiler and CRT/sysroot satisfying both `ring/assert.h` and `libsqlite3-sys/stdlib.h`.
