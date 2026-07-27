# Task 6 Report: Model Specialized Child Entities and Relations

Date: 2026-07-27

Base: `bc5b7e0d71302f40b174af6c20f5188bd3ac1c3f`

Implementation commit: `4b95e6b25f47a39ab49a469cf000ff8504b677aa` (`feat: expose complete metadata relation graph`)

## Scope

Task 6 models specialized child entities and relation traversal through the closed, domain-neutral navigation graph. It does not implement the architecture enforcement checks reserved for Task 10. The controller's ignored `task-6-brief.md` was preserved unchanged.

## Implemented behavior

- Partitioned every registered semantic relation role into exactly one closed kind: containment or reference. Relation selection now derives the kind from the role and rejects explicit role/kind mismatches in direct core values, decoded cursors, runtime requests, and the MCP JSON schema.
- Retained generic children, attributes, tabular sections, columns, forms, templates, and commands while exposing dimensions, resources, enumeration values, URL templates, HTTP methods, web-service operations, parameters, access children, and unknown children through their closed semantic roles.
- Preserved decoder order for each child/relation target and collision-safe semantic object references. Relation identities now include the semantic role, so two differently-typed edges with the same source and target cannot collapse.
- Deferred reference projection until containment projection is complete. A reference reuses an already projected semantic node when available; otherwise the adapter creates one derived read-only node with a typed `metadata.name` property. Every reference page therefore points to a traversable node.
- Preserved unknown specialized children as separate ordered `unknown` nodes with readable facts and partial coverage. No unknown occurrence is silently merged into a known child or sibling.
- Extended `unica.meta.info` schema/runtime contracts and the `meta-info` skill with closed role traversal and typed property examples. Legacy `Name`, `Mode`, offset, arbitrary relation strings, and native class/role vocabulary were not added.

## Task 5 prerequisite repair 1: EmptyReference cache integration

Task 5 introduced the distinct core `PropertyType::EmptyReference` and `PropertyValue::EmptyReference` but omitted them from two production exhaustive cache-validation matches. Those two arms were added as payload-free scalar cases. They do not map the value to null, absence, or unresolved state and therefore preserve core validation and serialization semantics.

The focused regression compiles the test-only depth observer too, which had its own exhaustive scalar match; the same zero-child arm was added there. `task6_cache_preserves_empty_reference_as_a_distinct_scalar_value` proves the cached property remains present with both value and type equal to `EmptyReference`.

No unrelated application refactoring was performed.

## Task 5 prerequisite repair 2: stale const profile callers

Task 5 commit `75a5941a9d4393fe324ffe22904a19e79d65fcea` consolidated separate version/class accessors into `PlatformXmlAdapterFactory::profile()` and left two coder constants calling it. Commit `71f366d95f419ccc736895b2042bf36192aee7cc` correctly made that profile runtime when metadata classes moved from a duplicate const list to the authoritative JSON-backed registry, but did not migrate those const callers.

The repair preserves that architecture:

- only immutable `platform_line` and `export_format` have narrow const accessors;
- `ACTIVE_FORMAT_PROFILE` is a two-field compile-time identity used by existing host constants;
- metadata kind tags are fetched at runtime from `profile().legacy_metadata_classes`;
- the registry test proves the caller receives the same authoritative runtime slice by pointer identity.

The runtime profile remains non-const. No metadata-class list was restored or duplicated.

## TDD evidence

### RED: closed role/kind selection

Command:

`cargo test -p unica-format-core --test public_json_contract task6_relation_selection_derives_and_enforces_the_closed_role_kind -- --exact --nocapture`

Initial result: exit 101; 0 passed, 1 failed. `basedOn` was incorrectly constructed as `Contains` instead of `References`.

### RED: traversable reference targets

Command:

`cargo test -p unica-adapter-platform-xml --test specialized_relations task6_reference_relations_are_traversable_nodes_with_stable_semantic_targets -- --exact --nocapture`

Initial result: exit 101; 0 passed, 1 failed. The reference target `Order` had no navigation node.

### RED: blocked coder contract execution

Command:

`cargo test -p unica-coder task6_ -- --nocapture`

The first attempt failed compilation on the missing Task 5 `EmptyReference` cache match arms. After that approved repair, the same command exposed `E0015` at `domain/format_profile.rs:5` and `infrastructure/metadata_kinds.rs:84` because runtime `profile()` was still called from const contexts. A Task 6 test-harness-only `BTreeSet` use also produced `E0277` because `RelationKind` intentionally has no ordering contract; the assertion was changed to unordered membership without changing production types.

These failures occurred before the three coder Task 6 tests could execute and establish the prerequisite repairs required for scoped validation.

## Final GREEN validation

Only the Task 6 and approved prerequisite surfaces were run. No workspace-wide suite, lint, format, or unrelated validation was run.

1. `cargo test -p unica-format-core --test public_json_contract task6_relation_selection_derives_and_enforces_the_closed_role_kind -- --exact --nocapture`
   Result: 1 passed; 0 failed; 4 filtered out.
2. `cargo test -p unica-application snapshot_cache::tests::task6_cache_preserves_empty_reference_as_a_distinct_scalar_value -- --exact --nocapture`
   Result: 1 passed; 0 failed; 49 filtered out.
3. `cargo test -p unica-adapter-platform-xml --test specialized_relations -- --nocapture`
   Result: 3 passed; 0 failed.
4. `cargo test -p unica-coder domain::format_profile::tests::active_profile_identity_remains_const_usable -- --exact --nocapture`
   Result: 1 passed; 0 failed; 1495 filtered out.
5. `cargo test -p unica-coder infrastructure::metadata_kinds::tests::registry_has_unique_canonical_tags_and_directories -- --exact --nocapture`
   Result: 1 passed; 0 failed; 1495 filtered out.
6. `cargo test -p unica-coder task6_ -- --nocapture`
   Result: 3 passed; 0 failed; 1493 filtered out.

All six commands exited 0. The coder runs emitted 21 existing dead-code warnings from source-adapter infrastructure; no new warning was reported for Task 6 code.

## Guarded invariants

- Every closed relation role belongs to exactly one relation kind; the test partitions `SemanticRelationId::ALL` and rejects unclassified or multiply classified roles.
- Omitted relation kind is role-derived; an explicit incompatible kind fails both schema and runtime decoding.
- Specialized child order and target order are preserved, and every relation target resolves to a stable object reference present in `nodes`.
- Distinct unknown child occurrences remain distinct partial facts.
- Typed properties remain closed IDs; examples use `metadata.name`, `httpService.method.httpMethod`, and `webService.parameter.direction`.
- Native class/property/role mapping remains in the private `v2_20` adapter implementation. Mechanical architecture enforcement remains Plan Task 10.
- Runtime metadata classes continue to come from the authoritative adapter coverage registry; only immutable version identity is const-usable.

## Files in the implementation commit

- `crates/unica-format-core/src/semantic_ids.rs`
- `crates/unica-format-core/src/navigation.rs`
- `crates/unica-format-core/tests/public_json_contract.rs`
- `crates/unica-adapter-platform-xml/src/factory.rs`
- `crates/unica-adapter-platform-xml/src/versions/v2_20/projector.rs`
- `crates/unica-adapter-platform-xml/tests/specialized_relations.rs`
- `crates/unica-application/src/snapshot_cache.rs`
- `crates/unica-coder/src/application/metadata_navigation.rs`
- `crates/unica-coder/src/application/mod.rs`
- `crates/unica-coder/src/application/tool_contracts.rs`
- `crates/unica-coder/src/domain/format_profile.rs`
- `crates/unica-coder/src/infrastructure/metadata_kinds.rs`
- `crates/unica-coder/src/infrastructure/native_operations/cf.rs`
- `plugins/unica/skills/meta-info/SKILL.md`

## Concerns and intentional limits

- References with no already-projected target use deterministic derived identity from semantic kind and name because the source relation carries no persistent target UUID. The identity is truthful as `Derived`, not fabricated as persistent.
- Architecture dependency/textual guards are intentionally not added here; they remain Task 10 after Tasks 7-9.
- The ignored controller brief remains present and unchanged but is not committed, consistent with prior task briefs.
