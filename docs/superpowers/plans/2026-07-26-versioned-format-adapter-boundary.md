# Versioned Format Adapter Boundary Implementation Plan

> **Required skill:** Use `superpowers:executing-plans` to implement this plan task by task.

**Goal:** Make PR #210 return semantically complete typed metadata JSON while physically confining every Platform XML parser, version rule, reader, writer, and validator to a versioned family adapter crate.

**Architecture:** Add `unica-format-core` for closed neutral contracts, `unica-application` for format-neutral navigation use cases, and `unica-adapter-platform-xml` for private format 2.20 implementation. Keep `unica-coder` as MCP host/composition root. Replace direct native calls with core ports, preserve all useful legacy `meta.info` facts as typed properties/nodes/relations, and enforce dependency/API boundaries in CI.

**Tech stack:** Rust 2021 workspace, Serde JSON, roxmltree/quick-xml inside the Platform XML adapter, existing MCP host, Python CI contract checks.

**Design source:** `docs/superpowers/specs/2026-07-26-versioned-format-adapter-boundary-design.md`

## Constraints

- Legacy text `meta.info` is removed; backward compatibility is intentionally broken.
- Legacy `Mode=full` plus drill-down is the minimum semantic information baseline.
- Native tags, namespaces, paths, parser types, and binary layout identifiers do not cross adapter boundaries.
- An unmapped meaningful fact returns `status=partial`; it cannot return `ready`.
- Platform XML 2.20 is the only implementation in this PR.
- EDT, direct CF, and direct FileDB crates are not added until they have implementations.
- Existing Platform XML mutations retain behavior through semantic writer ports.

## Task 1: Add the neutral format core crate

**Files:**

- Modify: `Cargo.toml`
- Create: `crates/unica-format-core/Cargo.toml`
- Create: `crates/unica-format-core/src/lib.rs`
- Move: `crates/unica-coder/src/domain/navigation.rs` to `crates/unica-format-core/src/navigation.rs`
- Move: `crates/unica-coder/src/domain/navigation_limits.rs` to `crates/unica-format-core/src/limits.rs`
- Move: `crates/unica-coder/src/domain/source_adapters.rs` to `crates/unica-format-core/src/source.rs`
- Create: `crates/unica-format-core/src/semantic_ids.rs`
- Create: `crates/unica-format-core/src/ports.rs`
- Modify: `crates/unica-coder/Cargo.toml`
- Modify: `crates/unica-coder/src/domain/mod.rs`

**Steps:**

1. Add `unica-format-core` to the workspace and give it only `serde`, `serde_json`, and `sha2` dependencies needed by neutral value objects.
2. Make navigation, source identity, snapshots, diagnostics, capabilities, and resource limits public core types.
3. Introduce non-arbitrary `SemanticPropertyId`, `SemanticRelationId`, `SemanticFacetId`, and `SemanticObjectKind` types with private storage and core-owned constants.
4. Define neutral probe, capture, read, write, validation, and capability ports. Do not expose `serde_json::Value` as a native escape hatch in writer commands.
5. Temporarily re-export moved types from `unica-coder::domain` so the host migration can proceed in compilable slices.
6. Move existing unit tests with the types and add compile-time tests that semantic IDs cannot be constructed from arbitrary adapter strings.
7. Commit: `refactor: extract neutral format core`.

## Task 2: Add the Platform XML family adapter crate

**Files:**

- Modify: `Cargo.toml`
- Create: `crates/unica-adapter-platform-xml/Cargo.toml`
- Create: `crates/unica-adapter-platform-xml/src/lib.rs`
- Create: `crates/unica-adapter-platform-xml/src/factory.rs`
- Create: `crates/unica-adapter-platform-xml/src/versions/mod.rs`
- Move: `crates/unica-coder/src/infrastructure/source_adapters/platform_xml/` to `crates/unica-adapter-platform-xml/src/versions/v2_20/`
- Move: `crates/unica-coder/src/infrastructure/source_adapters/certification.rs` to `crates/unica-adapter-platform-xml/src/certification.rs`
- Move: `crates/unica-coder/src/infrastructure/platform_xml_owner.rs` to `crates/unica-adapter-platform-xml/src/owner.rs`
- Move format-specific parts of `crates/unica-coder/src/domain/format_profile.rs` to `crates/unica-adapter-platform-xml/src/versions/v2_20/profile.rs`
- Modify: `crates/unica-coder/src/infrastructure/source_adapters/mod.rs`
- Modify: `crates/unica-coder/src/infrastructure/source_adapters/registry.rs`

**Steps:**

1. Add `roxmltree`, `quick-xml`, `serde`, `serde_json`, `sha2`, and `unica-format-core` only to the adapter crate.
2. Make `versions` and `v2_20` private; expose only `PlatformXmlAdapterFactory` and registration.
3. Convert existing adapter traits to implementations of core-owned ports.
4. Move support decoding and owner/version resolution into the adapter; expose neutral support/authorability and compatibility results.
5. Keep native model, decoder, schema, XML helper, projector, provider, owner parser, and profile types private.
6. Add API/compile-fail tests proving an external crate cannot name `v2_20`, native XML documents, projectors, schemas, or owner parser types.
7. Commit: `refactor: isolate platform xml adapter`.

## Task 3: Extract format-neutral application orchestration

**Files:**

- Modify: `Cargo.toml`
- Create: `crates/unica-application/Cargo.toml`
- Create: `crates/unica-application/src/lib.rs`
- Create: `crates/unica-application/src/navigation.rs`
- Create: `crates/unica-application/src/snapshot_cache.rs`
- Create: `crates/unica-application/src/selection.rs`
- Create: `crates/unica-application/src/commands.rs`
- Modify: `crates/unica-coder/Cargo.toml`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/meta.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/typed_result.rs`
- Modify: `crates/unica-coder/src/application/mod.rs`

**Steps:**

1. Move snapshot retention, object-ref continuation, cursor authentication, selection normalization, relation paging, and navigation status orchestration out of `meta.rs`.
2. Depend only on `unica-format-core`; inject erased core ports and a source-locator resolver.
3. Preserve immutable snapshot behavior, authorization binding, FIFO byte accounting, path-free HMAC cursors, default page size 25, and maximum page size 100.
4. Replace the `meta-info` native-operation special case with a call to `unica-application::MetadataNavigationService`.
5. Leave MCP argument parsing and final `data.navigation` serialization in `unica-coder`.
6. Move the existing navigation/cache/cursor tests to the application crate.
7. Commit: `refactor: extract metadata navigation application`.

## Task 4: Define the complete semantic vocabulary

**Files:**

- Modify: `crates/unica-format-core/src/semantic_ids.rs`
- Modify: `crates/unica-format-core/src/navigation.rs`
- Create: `crates/unica-format-core/src/property.rs`
- Create: `crates/unica-format-core/src/value.rs`
- Create: `crates/unica-format-core/src/facets.rs`
- Create: `crates/unica-format-core/tests/public_json_contract.rs`
- Create: `crates/unica-format-core/tests/semantic_registry.rs`

**Steps:**

1. Make properties the sole value source and facets core-owned lists of property/relation IDs.
2. Support boolean, integer, decimal, string, localized string, UUID, enum, date, type set, object reference, list, structure, null, and unknown values.
3. Preserve `explicit`, `defaulted`, `inherited`, `computed`, `absent`, and `unresolved` value states.
4. Add neutral provenance and per-property capability without descriptor paths or native evidence names.
5. Register semantic IDs for generic identity, presentations, support, documents, catalogs, registers, constants, reports, defined types, modules, jobs, subscriptions, HTTP services, web services, enums, fields, tabular sections, forms, templates, and commands.
6. Add JSON shape tests for exactly `schemaVersion`, `status`, `snapshot`, `root`, `nodes`, `relations`, and `diagnostics` at navigation top level.
7. Commit: `feat: define complete metadata semantic vocabulary`.

## Task 5: Make Platform XML 2.20 projection information-complete

**Files:**

- Modify: `crates/unica-adapter-platform-xml/src/versions/v2_20/schema.rs`
- Modify: `crates/unica-adapter-platform-xml/src/versions/v2_20/native_model.rs`
- Modify: `crates/unica-adapter-platform-xml/src/versions/v2_20/decoder.rs`
- Modify: `crates/unica-adapter-platform-xml/src/versions/v2_20/projector.rs`
- Modify: `crates/unica-adapter-platform-xml/src/versions/v2_20/provider.rs`
- Create: `crates/unica-adapter-platform-xml/src/versions/v2_20/semantic_map.rs`
- Create: `crates/unica-adapter-platform-xml/src/versions/v2_20/coverage.json`
- Create: `crates/unica-adapter-platform-xml/tests/legacy_parity.rs`
- Create: `crates/unica-adapter-platform-xml/tests/unmapped_fact.rs`

**Steps:**

1. Replace the small scalar whitelist with an explicit version-owned map from native facts to core semantic IDs.
2. Preserve lexical values internally until typed projection succeeds; never collapse an unrecognized scalar to a generic summary while reporting ready.
3. Project every legacy parity property listed in the design spec.
4. Parse complete type descriptions into structured type sets instead of rendered strings.
5. Project localized presentation variants separately.
6. Set `partial` and emit `unmappedSemanticFact` when a meaningful direct property or known child role has no semantic mapping.
7. Keep native names and evidence in adapter-internal diagnostics only.
8. Add a coverage manifest using semantic IDs only.
9. Commit: `feat: complete platform xml semantic projection`.

## Task 6: Model specialized child entities and relations

**Files:**

- Modify: `crates/unica-format-core/src/semantic_ids.rs`
- Modify: `crates/unica-format-core/src/navigation.rs`
- Modify: `crates/unica-adapter-platform-xml/src/versions/v2_20/schema.rs`
- Modify: `crates/unica-adapter-platform-xml/src/versions/v2_20/decoder.rs`
- Modify: `crates/unica-adapter-platform-xml/src/versions/v2_20/projector.rs`
- Modify: `crates/unica-coder/src/application/tool_contracts.rs`
- Modify: `plugins/unica/skills/meta-info/SKILL.md`

**Steps:**

1. Add node kinds and relations for dimensions, resources, enum values, URL templates, HTTP methods, web-service operations, parameters, register records, and based-on references.
2. Retain existing attributes, tabular sections, forms, templates, commands, and generic children.
3. Preserve semantic ordering and stable object references for every child node.
4. Extend relation selection schema without reintroducing `Name`, `Mode`, offset, or arbitrary relation strings.
5. Update skill examples to traverse the new relation roles and typed properties.
6. Commit: `feat: expose complete metadata relation graph`.

## Task 7: Move Platform XML reads, guards, and validation behind ports

**Files:**

- Move format-aware logic from: `crates/unica-coder/src/infrastructure/format_guard.rs`
- Move format-aware logic from: `crates/unica-coder/src/infrastructure/support_guard.rs`
- Move format-aware logic from: `crates/unica-coder/src/infrastructure/metadata_kinds.rs`
- Move format-aware logic from: `crates/unica-coder/src/infrastructure/native_operations/meta_validation_context.rs`
- Move format-aware logic from: `crates/unica-coder/src/infrastructure/platform/full_dump_publication.rs`
- Create: `crates/unica-adapter-platform-xml/src/guards.rs`
- Create: `crates/unica-adapter-platform-xml/src/validation.rs`
- Create: `crates/unica-adapter-platform-xml/src/publication.rs`
- Modify: `crates/unica-format-core/src/ports.rs`
- Modify: `crates/unica-application/src/commands.rs`
- Modify: `crates/unica-coder/src/infrastructure/tool_context.rs`
- Modify: `crates/unica-coder/src/infrastructure/internal_adapters.rs`

**Steps:**

1. Replace direct owner/support/profile parsing with neutral authorability, compatibility, validation, and publication port calls.
2. Keep filesystem source location internal to the command/session and out of public JSON.
3. Preserve current 8.3.27 / format 2.20 validation and no-downgrade behavior.
4. Ensure application code can express policy without matching XML root names or format versions.
5. Commit: `refactor: route platform xml guards through adapter`.

## Task 8: Move existing Platform XML writers into the family adapter

**Files:**

- Move Platform XML implementation from: `crates/unica-coder/src/infrastructure/native_operations/common.rs`
- Move Platform XML implementation from: `crates/unica-coder/src/infrastructure/native_operations/compile_transaction.rs`
- Move Platform XML implementation from: `crates/unica-coder/src/infrastructure/native_operations/cf.rs`
- Move Platform XML implementation from: `crates/unica-coder/src/infrastructure/native_operations/cfe.rs`
- Move Platform XML implementation from: `crates/unica-coder/src/infrastructure/native_operations/dcs.rs`
- Move Platform XML implementation from: `crates/unica-coder/src/infrastructure/native_operations/external.rs`
- Move Platform XML implementation from: `crates/unica-coder/src/infrastructure/native_operations/form.rs`
- Move Platform XML implementation from: `crates/unica-coder/src/infrastructure/native_operations/help.rs`
- Move Platform XML implementation from: `crates/unica-coder/src/infrastructure/native_operations/interface.rs`
- Move Platform XML implementation from: `crates/unica-coder/src/infrastructure/native_operations/meta.rs`
- Move Platform XML implementation from: `crates/unica-coder/src/infrastructure/native_operations/mxl.rs`
- Move Platform XML implementation from: `crates/unica-coder/src/infrastructure/native_operations/role.rs`
- Move Platform XML implementation from: `crates/unica-coder/src/infrastructure/native_operations/subsystem.rs`
- Move Platform XML implementation from: `crates/unica-coder/src/infrastructure/native_operations/support.rs`
- Move Platform XML implementation from: `crates/unica-coder/src/infrastructure/native_operations/template.rs`
- Create: `crates/unica-adapter-platform-xml/src/operations/mod.rs`
- Create semantic command/result types under: `crates/unica-format-core/src/commands/`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/registry.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/typed_result.rs`

**Steps:**

1. Split orchestration inputs from native implementation before moving each operation family.
2. Put semantic command/result DTOs in core; do not move MCP parameter names or `AdapterOutcome` into the adapter.
3. Implement core writer/validator ports in the adapter for each existing operation family.
4. Keep BSL-only code patching in the host/application unless it interprets Platform XML to locate an artifact; move only that locator behind a read port.
5. Map neutral adapter outcomes to MCP `AdapterOutcome` in the host.
6. Preserve dry-run, transactional publication, support guards, format guards, and existing output details.
7. Add semantic before/after preservation tests for every writer family.
8. Delete direct parser and serializer code from `unica-coder` after each family is ported.
9. Commit: `refactor: isolate platform xml writers`.

## Task 9: Make `unica-coder` a host/composition root

**Files:**

- Modify: `crates/unica-coder/src/lib.rs`
- Modify: `crates/unica-coder/src/main.rs`
- Modify: `crates/unica-coder/src/application/mod.rs`
- Modify: `crates/unica-coder/src/application/ports.rs`
- Modify: `crates/unica-coder/src/infrastructure/mod.rs`
- Modify: `crates/unica-coder/src/infrastructure/application_ports.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations.rs`
- Modify: `crates/unica-coder/src/infrastructure/source_adapters/mod.rs`
- Modify: `crates/unica-coder/src/infrastructure/source_adapters/registry.rs`
- Modify: `crates/unica-coder/src/interfaces/mcp.rs`

**Steps:**

1. Construct Platform XML factories in one composition module.
2. Inject core ports into `unica-application` use cases.
3. Retain MCP tool names and one public server named `unica`.
4. Remove host dependencies on `roxmltree` and `quick-xml` once no host code uses them.
5. Remove transitional domain and infrastructure re-exports.
6. Commit: `refactor: reduce unica-coder to composition host`.

## Task 10: Add enforceable architecture guards

**Files:**

- Create: `scripts/ci/check-format-architecture.py`
- Create: `tests/ci/test_format_architecture.py`
- Modify: `.github/workflows/ci.yml` or the existing Rust/contract workflow that runs repository CI checks
- Modify: `Cargo.toml`
- Create: `crates/unica-adapter-platform-xml/tests/public_api.rs`
- Modify: `spec/architecture/change-checklist.md`

**Steps:**

1. Use `cargo metadata` to reject adapter dependencies from `unica-application` and reverse dependencies from core.
2. Reject `roxmltree`, `quick-xml`, adapter imports, Platform XML namespace literals, native tag constants, and binary-layout identifiers outside adapter crates and explicit fixture/test locations.
3. Inspect public navigation JSON recursively for forbidden native transport fields.
4. Verify the adapter public API exposes only its factory/registration and core-owned types.
5. Verify every coverage manifest ID exists in the core registry.
6. Add the guard to CI and architecture change checklist.
7. Commit: `test: enforce format adapter architecture`.

## Task 11: Update public contracts and documentation

**Files:**

- Modify: `plugins/unica/skills/meta-info/SKILL.md`
- Modify: `plugins/unica/references/specs/1c-config-objects-spec.md`
- Modify: `spec/architecture/arc42/06-runtime-view.md`
- Modify: `spec/acceptance/unica-mcp-validation.md`
- Modify: `README.md`
- Modify: PR #210 description

**Steps:**

1. Document JSON-only navigation, semantic property envelopes, facets, specialized relations, `ready/partial/unsupported`, and cursor rules.
2. State implemented coverage exactly as Platform XML 2.20.
3. Remove legacy mode/name/offset examples and text-output claims.
4. Document future family crates without claiming implementations.
5. Commit: `docs: document versioned format adapters`.

## Task 12: Validation and PR completion

**Commands to run only with explicit user approval:**

1. `cargo fmt --all -- --check`
2. `cargo test -p unica-format-core`
3. `cargo test -p unica-application`
4. `cargo test -p unica-adapter-platform-xml`
5. `cargo test -p unica-coder`
6. `python3.12 -m unittest tests.ci.test_format_architecture`
7. Existing package, skill, MCP smoke, and diff checks required by the PR workflow.
8. Inspect the final diff and public API for accidental native leakage.
9. Push the branch and monitor PR #210 checks through completion.

## Completion definition

- The useful legacy information baseline is fully represented by typed semantic JSON.
- Unknown meaningful facts cannot be silently dropped.
- All Platform XML readers, validators, support/format guards, and writers live in the Platform XML adapter crate.
- `unica-application` has no adapter or parser dependency.
- `unica-coder` contains composition and transport only.
- Architecture guards fail on intentional boundary violations.
- Approved validation commands and PR checks pass.
