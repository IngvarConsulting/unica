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
