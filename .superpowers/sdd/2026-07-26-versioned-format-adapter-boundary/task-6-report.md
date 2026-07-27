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

## Fix Round 1

### Commit and scope

- Base: `7777ad47350d3983c2da800c2a83261927aadcba`
- Implementation commit: `3875a1fc` (`fix(adapter): harden specialized relation identities`)
- Scope remained Task 6. No Task 10 architecture guard was added, and no adapter boundary was rolled back.
- The controller-owned `progress.md` and ignored task brief/review artifacts were preserved and excluded from the implementation commit.

### Guarded invariants

1. A derived owned-child key includes the closed semantic owning relation role and a private native identity discriminator. UUID is preferred, canonical backing path is next, and a deterministic same-role/class/name occurrence is used only when stronger evidence is absent.
2. Child projection preserves native semantic order. Same-name children across roles and duplicate same-name children within one role receive distinct stable object references.
3. Reference resolution never compares public display name or semantic kind. The private v2_20 decoder hashes the complete available native target identity, and the projector resolves only through the exact discriminator or UUID index.
4. Private native class/qualified-target/path evidence is never serialized. Public object keys contain only authenticated semantic key material or its digest.
5. A target absent from the captured graph is a non-owned unresolved reference stub: partial coverage, `UnknownReadOnly`, no containment relation, no modeled action, partial envelope, and the format-neutral `referenceTargetUnresolved` diagnostic.
6. A forward reference resolves to the actual loaded node after the complete owned graph has populated the exact identity index. Ambiguous duplicate native target identities do not resolve by guess.
7. Materialized relation pages cover every new closed role, preserve relation and object references across pages, expose consistent facets, and use selection-bound authenticated opaque cursors.
8. The skill assigns `registerRecords` and `basedOn` to document owners, traverses operations and parameters through returned semantic references, validates every JSON example against the live MCP schema, and explains `ready`, `partial`, and `unavailable` handling through neutral diagnostics and coverage.

### RED evidence

Command:

```text
cargo test -p unica-adapter-platform-xml --test specialized_relations task6_fix1_ -- --nocapture
```

Initial result: exit 101; 1 passed and 3 failed.

- Duplicate no-UUID same-name children failed decode with `IdentityCollision`: `Platform XML owner has duplicate child identities of the same class`.
- Unknown native classes with the same public name collided at projection with `duplicate generated semantic relation key`.
- An external known reference incorrectly produced an `Available` envelope instead of `Partial` and inherited resolved owner semantics.
- The forward-reference case passed only because the old projector guessed by kind/display name; the new assertions made that accidental behavior insufficient once collision and external-target cases were considered together.

Command:

```text
cargo test -p unica-coder application::tool_contracts::tests::task6_meta_info_skill_teaches_specialized_semantic_navigation -- --exact --nocapture
```

Initial test-harness compile exposed an `E0308` wrapper omission in the new schema assertion. After the minimal harness correction, the semantic RED remained: the documented cursor `"<returned opaque cursor>"` violated the live MCP cursor pattern, and the document-only relation/status semantics were absent.

The new application materialization fixture passed before production changes. That was expected: it is a focused coverage regression for the existing application materializer, while the adapter RED cases above exercise the broken production behavior.

### GREEN evidence

```text
cargo test -p unica-adapter-platform-xml --test specialized_relations task6_fix1_ -- --nocapture
```

Result: 4 passed, 0 failed.

```text
cargo test -p unica-application navigation::tests::task6_fix1_materializes_every_specialized_role_with_stable_opaque_pages -- --exact --nocapture
```

Result: 1 passed, 0 failed.

```text
cargo test -p unica-coder application::tool_contracts::tests::task6_meta_info_skill_teaches_specialized_semantic_navigation -- --exact --nocapture
```

Result: 1 passed, 0 failed. Existing `unica-coder` dead-code warnings were unchanged.

```text
cargo test -p unica-adapter-platform-xml task6 -- --nocapture
```

Result: 7 Task 6 integration tests passed, 0 failed; 145 adapter unit tests and unrelated integration tests were filtered out.

```text
cargo test -p unica-adapter-platform-xml profile_contract_tests -- --nocapture
```

Result: 2 passed, 0 failed. The version identity accessors remain const-usable, and metadata classes come from `PlatformXmlAdapterFactory::profile().legacy_metadata_classes`, which delegates to the authoritative runtime registry.

```text
cargo test -p unica-application snapshot_cache::tests::task6_cache_preserves_empty_reference_as_a_distinct_scalar_value -- --exact --nocapture
```

Result: 1 passed, 0 failed.

### Task 5 prerequisite repairs retained

#### EmptyReference match repair

The minimal two-arm application repair from the initial Task 6 commit remains intact in `snapshot_cache.rs`. `PropertyValue::EmptyReference` and `PropertyType::EmptyReference` pass the same cache validation and recursive budgeting paths as other core scalar variants while remaining distinct from absent, null, and unresolved values. The focused cache regression above proves round-trip preservation. No unrelated cache refactoring was added in this round.

#### Stale const/runtime-profile caller repair

The authoritative runtime profile remains the only metadata-class registry. This round removed the last stale unit-test import of the deleted `METADATA_CLASS_PROFILES` constant and changed that caller to `PlatformXmlAdapterFactory::profile().legacy_metadata_classes`; it did not restore a duplicated const class list. The existing two-field compile-time identity and narrow const `platform_line`/`export_format` accessors were retained. Focused factory regressions prove both const usability and runtime-registry delegation.

The first scoped adapter unit-target compile documented the stale caller precisely:

```text
error[E0432] at crates/unica-adapter-platform-xml/src/versions/v2_20/probe.rs:273
unresolved import crate::versions::v2_20::schema::METADATA_CLASS_PROFILES
```

The same compile also identified private fixture constructor updates required by the Task 6 discriminator and the Task 5 runtime `NativeMetadataClass` shape at `projector.rs:2011`, `projector.rs:2270`, `projector.rs:2271`, and `projector.rs:2281`. Those callers now construct current private evidence; production architecture was not reverted.

### Files changed in implementation commit

- `crates/unica-adapter-platform-xml/src/factory.rs`
- `crates/unica-adapter-platform-xml/src/versions/v2_20/decoder.rs`
- `crates/unica-adapter-platform-xml/src/versions/v2_20/native_model.rs`
- `crates/unica-adapter-platform-xml/src/versions/v2_20/probe.rs`
- `crates/unica-adapter-platform-xml/src/versions/v2_20/projector.rs`
- `crates/unica-adapter-platform-xml/tests/specialized_relations.rs`
- `crates/unica-application/src/navigation.rs`
- `crates/unica-coder/src/application/tool_contracts.rs`
- `plugins/unica/skills/meta-info/SKILL.md`

### Concerns and bounded gaps

- Platform XML readable reference values in the covered v2_20 evidence expose native class plus qualified target name but no UUID. The private model supports UUID evidence when available; current exact matching therefore uses the full available class/qualified-target discriminator.
- For duplicate children without UUID or canonical backing path, occurrence identity is necessarily order-derived. It is deterministic for an unchanged captured source and preserves pagination continuity, but inserting an indistinguishable earlier sibling can change later fallback identities. This is the explicitly allowed last-resort case.
- No architecture-vocabulary guard was added because that remains approved Plan Task 10.
