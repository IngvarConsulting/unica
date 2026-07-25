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
