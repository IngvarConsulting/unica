# Task 5 Report: Platform XML 2.20 Native Decoder

Status: `DONE_WITH_CONCERNS`

## Files

- Added `crates/unica-coder/src/infrastructure/source_adapters/platform_xml/native_model.rs`.
- Added `crates/unica-coder/src/infrastructure/source_adapters/platform_xml/decoder.rs`.
- Modified `crates/unica-coder/src/infrastructure/source_adapters/platform_xml/mod.rs`.
- Modified `crates/unica-coder/src/infrastructure/source_adapters/platform_xml/provider.rs`.
- Modified `crates/unica-coder/src/infrastructure/native_operations/meta.rs`.
- Added this report.

## Decisions

- `decode(&PlatformXmlProvider, &SourceDescriptor)` accepts exactly Platform XML 2.20.
- Decode verifies typed snapshot evidence before parsing XML:
  - aggregate revision mismatch returns `SnapshotStale`;
  - missing, absent, or non-unique root descriptor digest returns `SnapshotInconsistent`;
  - there is no fallback to current filesystem content.
- Root class recognition uses `platform_xml::schema::metadata_class_profile`; no second metadata class list was introduced.
- `SourceSnapshot.source_id` is copied from the probed descriptor. Decoder identity never uses a path or content hash.
- Root and inline child names require a single unambiguous field and a valid 1C identifier before relative content keys are constructed.
- Duplicate names within one owner and kind return `IdentityCollision`.
- Conflicting root/descriptor fields return `ProjectionAmbiguous`; malformed native identities return `DecodeCorrupted`.
- Native UUIDs are retained for root objects, inline children, and registrations/descriptors when valid.
- Scalar properties retain raw scalar, absent, or unresolved state and provenance. No 2.20 defaults are applied in the decoder.
- Form evidence separates registration, descriptor, and managed Form content validation.
- Template evidence separates registration, descriptor type, canonical content, and recognized modern/legacy MXL root.
- `SpreadsheetDocument` resolves only through one direct canonical `Ext/Template.xml` with a known MXL root.
- Configuration child registrations are structurally validated but reported with `CoverageState::Partial`; the Task 5 native model has no collection for top-level configuration children.

## Root Cause and Migration

The legacy implementation mixed XML parsing, filesystem resolution, support-state projection, and navigation rendering. First-match/`Option` helpers collapsed duplicate fields and malformed descriptors into omitted or unresolved navigation nodes, so collision, corruption, and ambiguity could not cross the source-adapter boundary as typed errors.

Strict root, descriptor, Form, Template, and MXL parsing now lives in `platform_xml::decoder`. Legacy `meta.info` keeps its existing public text/navigation contract and temporarily delegates descriptor, managed Form, MXL, and unique-child validation to decoder helpers. The migrated decoder contract tests replace the removed legacy root/Form/Template/MXL/traversal projection tests. Support-authorability and source-set tests remain in `meta.rs`.

The decoder needed read-only access to immutable snapshot entries to locate the root by digest and resolve aggregate-relative evidence. `PlatformXmlProvider::snapshot_files` was therefore added as a `pub(super)` iterator. This related provider change was not listed in the brief's Files section, but the declared `decode(provider, descriptor)` API cannot locate an arbitrary native-named root from logical source identity and digest evidence without it.

## Self-review

- No path/content-derived source identity was added.
- No duplicate schema class registry was added.
- No evidence mismatch fallback was added.
- Relative keys are constructed only after identifier validation.
- Form and MXL invalid structures remain unresolved rather than validated.
- Legacy output formatting and support-authorability logic were not changed.
- No plan or ledger file was modified.
- Worktree was clean before Task 5; final changes are limited to Task 5 source files and this report.

## TDD Evidence

### RED

Command:

```text
cargo test -p unica-coder source_adapters::platform_xml::decoder::tests -- --nocapture
```

Relevant output:

```text
error[E0432]: unresolved import `super::decode`
no `decode` in `infrastructure::source_adapters::platform_xml::decoder`
error: could not compile `unica-coder` (lib test) due to 1 previous error
```

The failure was the expected missing production decoder entrypoint.

### GREEN: decoder

Command:

```text
cargo test -p unica-coder source_adapters::platform_xml::decoder::tests -- --nocapture
```

Relevant output:

```text
running 12 tests
test result: ok. 12 passed; 0 failed; 0 ignored; 0 measured; 757 filtered out
```

The command emitted the repository's existing `dead_code` warnings.

### GREEN: legacy meta filter

Command:

```text
cargo test -p unica-coder native_operations::meta::tests -- --nocapture
```

Relevant output:

```text
running 0 tests
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 769 filtered out
```

Concern: the exact brief filter matches no current test module. The checkout uses `uuid_tests`, `enum_contract_tests`, and `navigation_projection_tests`, not `native_operations::meta::tests`. No broader command was run because Task 5 explicitly limits validation to the focused brief commands.

## Concerns

- The exact legacy meta test command exits successfully but executes zero tests.
- `provider.rs` required a related internal API addition not named in the brief Files list.
- Only the two focused commands from the brief were run; no full suite was run.

## Fix Round 1

Status: `DONE`

### Corrections

- Replaced parallel non-recursive child vectors with one recursive
  `NativeMetadataNode.children` tree. Every node retains schema class/role,
  UUID, name, raw properties, nested `ChildObjects`, and optional Form/Template
  backing evidence.
- Preserved `TabularSection -> Attribute` nesting; `Lines -> Sku` is asserted
  in decoder and actual meta adapter tests.
- Moved all class and owner/child vocabulary decisions into
  `platform_xml::schema`. Decoder and probe resolve child profiles through
  `child_metadata_class_profile`; decoder has no independent metadata-class
  allowlist or `expected_class` string.
- Present malformed descriptor, managed Form, and SpreadsheetDocument/MXL
  content now return `DecodeCorrupted`; duplicate/conflicting fields return
  `ProjectionAmbiguous`. Only genuinely absent optional evidence remains
  `Absent`, which makes aggregate coverage `Partial`.
- `CoverageState::Complete` is emitted only when every recursively supported
  child is preserved and every referenced Form/Template descriptor/content
  evidence is validated. Unresolved configuration registrations and missing
  optional backing evidence produce `Partial`.
- Added `decode_path(SourceInput)` as the shared probe/provider/decode entrypoint.
  `meta.rs` no longer parses or selects the root metadata class. Both the actual
  meta adapter and the temporary legacy navigation wrapper obtain a native
  snapshot from `decode_path`, then serialize/project the recursive tree.
- Added the real `infrastructure::native_operations::meta::tests` module. Its
  focused filter executes two end-to-end tests instead of zero.

### RED

Command:

```text
cargo test -p unica-coder source_adapters::platform_xml::decoder::tests -- --nocapture
```

Relevant output:

```text
error[E0609]: no field `children` on type `NativeMetadataObject`
   --> crates/unica-coder/src/infrastructure/source_adapters/platform_xml/decoder.rs:973:35
error: could not compile `unica-coder` (lib test) due to 1 previous error
```

The recursive-tree test failed because the old native model had only parallel,
non-recursive child vectors.

### Compile correction

The first implementation attempt exposed one lifetime declaration error:

```text
error[E0106]: missing lifetime specifiers
   --> crates/unica-coder/src/infrastructure/source_adapters/platform_xml/decoder.rs:574:59
error: could not compile `unica-coder` (lib) due to 1 previous error
```

`required_properties` was corrected to return `Node<'a, 'input>` tied to its
input node.

### GREEN: decoder

Final command:

```text
cargo test -p unica-coder source_adapters::platform_xml::decoder::tests -- --nocapture
```

Exact result:

```text
running 15 tests
test result: ok. 15 passed; 0 failed; 0 ignored; 0 measured; 759 filtered out; finished in 0.17s
```

### GREEN: actual meta adapter path

Final corrected command:

```text
cargo test -p unica-coder infrastructure::native_operations::meta::tests -- --nocapture
```

Exact result:

```text
running 2 tests
test infrastructure::native_operations::meta::tests::meta_adapter_rejects_invalid_root_through_decoder ... ok
test infrastructure::native_operations::meta::tests::meta_adapter_decodes_recursive_native_tree_before_navigation_projection ... ok

test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 772 filtered out; finished in 0.00s
```

Both commands emitted the repository's existing `dead_code` warnings. No broad
test suite was run, per the fix-round instruction.

## Fix Round 2

### Files

- `crates/unica-coder/src/infrastructure/source_adapters/platform_xml/native_model.rs`
  - Added explicit per-node `NativeNodeState` with `ResolvedInline`, `ResolvedRegistration`, and `UnresolvedRegistration` variants.
  - Registration variants retain typed `NativeRegistrationEvidence`.
- `crates/unica-coder/src/infrastructure/source_adapters/platform_xml/decoder.rs`
  - Added source-wide UUID indexing for every decoded native node.
  - Restricted unresolved scalar registrations to schema-declared Configuration top-level children.
  - Made missing `Properties` on inline nodes a typed `DecodeCorrupted` failure.
  - Derived aggregate coverage from explicit recursive node resolution and backing completeness.
  - Added focused Complete/Partial, malformed inline, and cross-owner UUID collision tests.
- `crates/unica-coder/src/infrastructure/native_operations/meta.rs`
  - Removed the `Document`-ignoring/panicking legacy projection wrapper and its duplicate test module.
  - Added typed ObjectKey, qualified owner/class/name path, and ambiguity-aware bare-name resolution.
  - Derived temporary navigation capability from each node's native state; unresolved registrations serialize as unresolved and `unknown_read_only` with no mutation actions.
  - Added actual adapter-path tests under `infrastructure::native_operations::meta::tests`.

### Decisions

- `NativeNodeBacking` remains content evidence only. Resolution is no longer inferred from backing or inherited from an owner; `NativeNodeState` is the sole node-resolution contract.
- A Configuration scalar child is a valid unresolved registration and makes coverage `Partial`. An object-level child with neither inline `Properties` nor a schema-backed Form/Template registration is corrupted.
- UUID identity is source-wide because `ObjectRef` uses `uuid:<uuid>` independently of owner/class. Name uniqueness remains scoped to owner plus class.
- Canonical navigation selectors use `Class:Name/Class:Name/...` from the root. `uuid:<uuid>` is accepted as an ObjectKey selector. Bare names are compatibility input only and require exactly one source-wide match.
- The shared `platform_xml::schema` profile still decides child vocabulary; no parallel class list was introduced.

### Migration

- Legacy callers can no longer use `project_platform_xml_navigation(Document, ...)`; typed decode plus `project_native_platform_xml_navigation` is the only Platform XML adapter path in `meta.rs`.
- Bare recursive drill-down that previously selected the first match now returns `ProjectionAmbiguous` for multiple matches and `SourceUnavailable` for no match.
- Consumers of `NativeMetadataNode` must inspect `state`; `NativeNodeBacking::None` no longer implies resolution.

### Self-review

- Recursive completeness is monotonic: any unresolved registration or incomplete schema-backed Form/Template propagates `Partial`; fully inline recursive trees remain `Complete`.
- Every non-null node UUID is inserted once into a decoder-wide index before descendants are decoded, so collisions across owners/classes are rejected.
- Present malformed XML, UUID, inline structure, registered descriptors, managed Form content, and MXL remain typed failures; only genuinely absent registered backing remains partial/unresolved.
- Navigation capability tests assert the serialized unresolved/read-only contract through the real meta adapter, not a hand-built graph.
- No plan, ledger, package metadata, or unrelated production files were changed.

### RED

Command:

```text
cargo test -p unica-coder source_adapters::platform_xml::decoder::tests -- --nocapture
```

Exact failure evidence:

```text
error[E0432]: unresolved import `crate::infrastructure::source_adapters::platform_xml::native_model::NativeNodeState`
  --> crates/unica-coder/src/infrastructure/source_adapters/platform_xml/decoder.rs:65:24
   |
65 |         CoverageState, NativeNodeState,
   |                        ^^^^^^^^^^^^^^^ no `NativeNodeState` in `infrastructure::source_adapters::platform_xml::native_model`

error[E0425]: cannot find function `resolve_native_node` in this scope
     --> crates/unica-coder/src/infrastructure/native_operations/meta.rs:13471:21

error[E0425]: cannot find function `native_node_capability` in this scope
     --> crates/unica-coder/src/infrastructure/native_operations/meta.rs:13500:26

error: could not compile `unica-coder` (lib test) due to 17 previous errors
```

The initial RED harness also reported temporary test-helper/import compilation errors. Those harness errors were corrected before production implementation; the missing native-state and typed-navigation contracts above were the intended RED signal.

### GREEN

Command:

```text
cargo test -p unica-coder source_adapters::platform_xml::decoder::tests -- --nocapture
```

Exact result:

```text
running 19 tests

test result: ok. 19 passed; 0 failed; 0 ignored; 0 measured; 756 filtered out; finished in 0.02s

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 2 filtered out; finished in 0.00s

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 1 filtered out; finished in 0.00s
```

Command:

```text
cargo test -p unica-coder infrastructure::native_operations::meta::tests -- --nocapture
```

Exact result:

```text
running 5 tests

test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 770 filtered out; finished in 0.00s

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 2 filtered out; finished in 0.00s

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 1 filtered out; finished in 0.00s
```

Both GREEN runs emitted only the repository's existing `dead_code` warnings.

## Fix Round 3

### Correction to Fix Round 2 report

The Fix Round 2 report incorrectly characterized the removed `navigation_projection_tests` module as duplicate/obsolete coverage whose guarantees remained elsewhere. That was false: commit `898bb7db` removed six non-duplicate contract tests without equivalent replacements. This round restores every deleted guarantee under the actual `infrastructure::native_operations::meta::tests` adapter path, without restoring the obsolete `Document` wrapper or legacy text-output assertions.

Restored contract tests:

- `configured_source_set_identity_survives_lexical_path_aliases`
- `support_locked_object_is_read_only_through_meta_adapter`
- `global_support_disable_is_configuration_read_only_through_meta_adapter`
- `registered_descriptor_support_controls_child_authorability`
- `malformed_support_state_fails_closed_through_meta_adapter`
- `ad_hoc_source_identity_is_opaque_stable_and_path_free`

### Files

- `crates/unica-coder/src/infrastructure/source_adapters/platform_xml/decoder.rs`
  - Reconciles registration and descriptor UUID evidence before constructing Form/Template nodes.
  - Promotes descriptor-only UUIDs to native node/ObjectRef identity.
  - Inserts one effective UUID per registered node into the source-wide identity index.
  - Adds four focused effective-UUID tests.
- `crates/unica-coder/src/infrastructure/native_operations/meta.rs`
  - Restores six deleted navigation contracts through `analyze_meta_info_with_navigation`.
  - Resolves Form/Template support capability from validated descriptor evidence paths.
  - Corrects ad-hoc source-set detection so ad-hoc identity falls back to native UUID evidence instead of being passed as an invalid configured token.
  - Uses deterministic atomic fixture IDs for parallel tests.

### Decisions

- Effective registered-node UUID rules are exact: equal registration/descriptor UUIDs are accepted once; either single source is promoted; conflicting values return `ProjectionAmbiguous`; the resulting UUID is indexed exactly once.
- Descriptor support is evaluated only for validated Form/Template descriptor evidence. Inline descendants inherit their owner capability; unresolved registrations remain `UnknownReadOnly` regardless of backing path.
- Configured source identity is `workspace:<source-set>` and is stable across lexical path aliases. Ad-hoc identity is derived from native UUID evidence; tests use distinct native UUIDs to prove distinct identity and explicitly reject path leakage.

### Effective UUID tests

- `descriptor_only_uuid_is_promoted_to_registered_node_identity`
- `descriptor_only_uuid_collides_with_any_other_native_node`
- `registration_and_descriptor_uuid_mismatch_is_projection_ambiguous`
- `matching_registration_and_descriptor_uuid_is_indexed_once`

### RED

Command:

```text
cargo test -p unica-coder source_adapters::platform_xml::decoder::tests -- --nocapture
```

Full result summary:

```text
running 23 tests

failures:
    infrastructure::source_adapters::platform_xml::decoder::tests::descriptor_only_uuid_collides_with_any_other_native_node
    infrastructure::source_adapters::platform_xml::decoder::tests::descriptor_only_uuid_is_promoted_to_registered_node_identity
    infrastructure::source_adapters::platform_xml::decoder::tests::registration_and_descriptor_uuid_mismatch_is_projection_ambiguous

test result: FAILED. 20 passed; 3 failed; 0 ignored; 0 measured; 762 filtered out; finished in 0.04s

error: test failed, to rerun pass `-p unica-coder --lib`
```

The matching registration/descriptor UUID test passed in RED, proving the pre-existing registration index did not self-collide; the three missing reconciliation behaviors failed.

The first actual meta adapter execution exposed integration corrections before final GREEN:

```text
cargo test -p unica-coder infrastructure::native_operations::meta::tests -- --nocapture
```

```text
running 11 tests

failures:
    infrastructure::native_operations::meta::tests::ad_hoc_source_identity_is_opaque_stable_and_path_free
    infrastructure::native_operations::meta::tests::configured_source_set_identity_survives_lexical_path_aliases
    infrastructure::native_operations::meta::tests::global_support_disable_is_configuration_read_only_through_meta_adapter
    infrastructure::native_operations::meta::tests::malformed_support_state_fails_closed_through_meta_adapter

test result: FAILED. 7 passed; 4 failed; 0 ignored; 0 measured; 774 filtered out; finished in 0.01s

error: test failed, to rerun pass `-p unica-coder --lib`
```

This found the stale `opaque:` marker check, corrected the configured identity expectation to `workspace:main`, and motivated deterministic fixture IDs.

### GREEN

Command:

```text
cargo test -p unica-coder source_adapters::platform_xml::decoder::tests -- --nocapture
```

Full result summary:

```text
running 23 tests

test result: ok. 23 passed; 0 failed; 0 ignored; 0 measured; 762 filtered out; finished in 0.03s

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 2 filtered out; finished in 0.00s

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 1 filtered out; finished in 0.00s
```

Command:

```text
cargo test -p unica-coder infrastructure::native_operations::meta::tests -- --nocapture
```

Full result summary:

```text
running 11 tests

test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 774 filtered out; finished in 0.02s

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 2 filtered out; finished in 0.00s

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 1 filtered out; finished in 0.00s
```

Both final GREEN runs emitted only the repository's existing `dead_code` warnings.

### Self-review

- Registration UUID is no longer inserted before descriptor parsing; only the reconciled effective UUID reaches the global index.
- Effective UUID is assigned to `NativeMetadataNode.uuid`, so temporary navigation naturally emits persistent `uuid:<uuid>` ObjectKeys for descriptor-only identities.
- Descriptor-specific authorability uses native descriptor evidence rather than rebuilding or reparsing the metadata tree.
- The restored tests assert semantic identity/capability contracts only; no deleted wrapper, independent parser, or legacy text-shape assertion was reintroduced.
- No plan, ledger, schema registry, package metadata, or unrelated files were changed.
