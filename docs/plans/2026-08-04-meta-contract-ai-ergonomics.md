# Meta Contract AI Ergonomics Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the four-tool `unica.meta.*` surface self-describing and round-trip safe for MCP agents without restoring the retired Meta JSON DSL.

**Architecture:** Keep one read and three mutations. Publish the five metadata operations as one closed discriminated union reused by `meta.edit` and optional `meta.add.operations`; apply creation operations to the private template image before one guarded publication. Extend the provider-neutral result model so every public mutation is observable through `meta.info`, every preview reports semantic effects, and the MCP adapter publishes the existing `OperationResult` as structured protocol data.

**Tech Stack:** Rust 2021, `serde`/`serde_json`, `rmcp` 2.2, `roxmltree`, existing Platform XML template/edit/publisher code, Python contract and JSON-RPC smoke tests.

## Global Constraints

- The public Meta surface remains exactly `unica.meta.info`, `unica.meta.add`, `unica.meta.edit`, and `unica.meta.remove`; retired names and DSL aliases stay absent.
- Existing-object identity remains `sourceSet + metadataPath`; creation remains `sourceSet + kind + name` and creates exactly one top-level object.
- `meta.add.operations` and `meta.edit.operations` use the same schema generator and the same closed `Vec<MetaEditOperation>` conversion.
- A failed creation operation leaves no descriptor, child resource, registration, cache event, or workspace state.
- `meta.info` is local-only when `sections` is omitted; related-section `limit` defaults to 20 and is bounded to 50.
- Every value accepted by a public mutation must be visible in the next `meta.info`, or explicitly listed as a removed legacy capability in the migration ledger.
- Preview returns semantic effects and never embeds complete Platform XML.
- `tools/list.outputSchema` and `tools/call.structuredContent` describe the same `OperationResult`; domain failures set MCP `isError: true` without losing diagnostics or partial data.
- All production behavior follows red-green-refactor. Documentation and contract owners change in the same commits as the behavior they specify.

---

### Task 1: Replace the shallow operation schema with a closed union

**Files:**

- Modify: `crates/unica-coder/src/application/metadata.rs`
- Modify: `tests/ci/test_meta_surface_contract.py`
- Modify: `crates/unica-coder/src/application/mod.rs`
- Modify: `spec/architecture/invariants.md`

**Interfaces:**

- Produce `fn operation_schema() -> Value` with `oneOf` variants for `setProperties`, `add`, `update`, `remove`, and `editRelations`.
- Each variant has `additionalProperties: false`, a single-value `op` enum, and exact `required` fields.
- Reuse `property_values_schema`, `scope_schema`, add/update element schemas, and relation target schemas; do not copy five diverging nested type definitions.

- [ ] **Step 1: Write contract tests that reject the current shallow form**

Add Rust assertions that inspect `metadata_input_schema(MetadataOperation::Edit)` and require these literal requirements:

```text
setProperties -> op, values
add           -> op, collection, elements
update        -> op, collection, elements
remove        -> op, collection, names
editRelations -> op, relation, mode, targets
```

Update `test_schema_path_has_exact_lower_camel_arguments_and_typed_edit_items` so the Python contract requires `oneOf` rather than forbidding composition.

- [ ] **Step 2: Run the narrow tests and observe RED**

```sh
cargo test -p unica-coder application::metadata::tests::edit_schema -- --exact
python3.12 -m unittest tests.ci.test_meta_surface_contract.MetaSurfaceContractTests.test_schema_path_has_exact_lower_camel_arguments_and_typed_edit_items
```

Expected: the current single object with `required: ["op"]` fails both assertions.

- [ ] **Step 3: Implement the union from shared schema builders**

Use a helper with the concrete signature:

```rust
fn tagged_operation_variant(
    tag: MetaEditOperationTag,
    properties: Map<String, Value>,
    required: &[&str],
) -> Value;
```

Give every nested public field a description. Encode `MetaPosition` as exactly one of `before` or `after`; encode metadata type and fill-value variants with their own required fields and closed objects.

- [ ] **Step 4: Run schema and parser tests GREEN**

```sh
cargo test -p unica-coder application::metadata -- --test-threads=1
python3.12 -m unittest tests.ci.test_meta_surface_contract
```

- [ ] **Step 5: Synchronize the invariant owner**

Extend `INV-MCP-META-SURFACE` so its rule requires conditionally complete operation variants in the published schema. Keep ADR-0025 and the approved design as the normative owner.

---

### Task 2: Make `meta.info` cheap by default and bounded when enriched

**Files:**

- Modify: `crates/unica-coder/src/application/metadata.rs`
- Modify: `crates/unica-coder/src/application/meta_info_surface_tests.rs`
- Modify: `plugins/unica/skills/meta-info/SKILL.md`
- Modify: `spec/architecture/tool-surface.md`

**Interfaces:**

- `parse_info_sections(None)` returns `Vec::new()`.
- `parse_bounded_usize(value, field, default, maximum)` accepts `1..=50` for `limit`.
- Empty selected sections produce an empty `MetaRelatedSections` without constructing or probing RLM.

- [ ] **Step 1: Add failing request and provider-observation tests**

Test that an omitted `sections` produces no related-provider calls, `sections: []` is equivalent, `limit: 50` succeeds, and `limit: 51` returns `invalid_arguments` with `field: "limit"`.

- [ ] **Step 2: Run RED**

```sh
cargo test -p unica-coder meta_info_surface_tests::info_without_sections_is_local_only -- --exact
cargo test -p unica-coder application::metadata::tests::info_limit_is_bounded -- --exact
```

- [ ] **Step 3: Implement the empty default and hard maximum**

Remove `DEFAULT_INFO_SECTIONS`, publish `default: []`, add `maximum: 50`, and return an empty related result before RLM setup when no section is selected.

- [ ] **Step 4: Run GREEN and update the skill example**

```sh
cargo test -p unica-coder meta_info -- --test-threads=1
python3.12 -m unittest tests.ci.test_unica_skills
```

The first `meta-info` example remains local-only. Add one explicit enrichment example with `sections` and `limit`.

---

### Task 3: Apply optional creation operations in the add transaction

**Files:**

- Modify: `crates/unica-coder/src/application/metadata.rs`
- Modify: `crates/unica-coder/src/application/meta_add_surface_tests.rs`
- Modify: `crates/unica-coder/src/infrastructure/metadata_operations.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/meta/edit.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/meta/publisher.rs`
- Modify: `plugins/unica/skills/meta-add/SKILL.md`

**Interfaces:**

```rust
pub(crate) struct MetaAddRequest {
    pub(crate) source_set: String,
    pub(crate) kind: MetadataKind,
    pub(crate) name: String,
    pub(crate) operations: Vec<MetaEditOperation>,
    pub(crate) dry_run: bool,
}
```

- `operations` is optional at the top level but non-empty when present.
- Extract a provider helper that applies operations to descriptor bytes and returns the post-image plus `TypedChildResourcePlan`; both add and edit call it.
- Add commits template files, operation-created child resources, dependency guards, and owner registration through one `CompileTransaction`.

- [ ] **Step 1: Add failing parser, preview, apply, and rollback tests**

Use a Catalog request with `setProperties(Comment)` and `add(attributes)` in one `meta.add`. Assert preview has no writes, apply creates the configured descriptor, and invalid operation index 1 leaves the root byte-for-byte unchanged.

- [ ] **Step 2: Run RED**

```sh
cargo test -p unica-coder meta_add_surface_tests::add_applies_operations_atomically -- --exact
cargo test -p unica-coder meta_add_surface_tests::failed_add_operation_leaves_no_object -- --exact
```

- [ ] **Step 3: Parse add operations with the edit converter**

Refactor the current edit-only parser to:

```rust
fn parse_operations(value: Option<&Value>, required: bool) -> Result<Vec<MetaEditOperation>, MetaFailure>;
```

For `meta.add`, absence returns an empty vector and an explicitly empty array is invalid. For `meta.edit`, absence and empty are invalid.

- [ ] **Step 4: Build the complete private post-image before transaction registration**

Apply operations to the template descriptor before adding any transaction mutation. Merge child-resource plans and relation dependency guards into the same `CompileTransaction`; validate the final descriptor, registration, dependencies, and child resources.

- [ ] **Step 5: Run add/edit/publisher suites GREEN**

```sh
cargo test -p unica-coder meta_add -- --test-threads=1
cargo test -p unica-coder typed_edit -- --test-threads=1
cargo test -p unica-coder publisher -- --test-threads=1
```

---

### Task 4: Make mutation effects and info readback symmetric

**Files:**

- Modify: `crates/unica-coder/src/domain/metadata/results.rs`
- Modify: `crates/unica-coder/src/application/ports.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/meta/info.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/meta/edit.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/meta/publisher.rs`
- Modify: `crates/unica-coder/src/application/meta_info_surface_tests.rs`

**Interfaces:**

```rust
pub(crate) struct MetaMutationEffect {
    pub(crate) operation_index: Option<u64>,
    pub(crate) operation: String,
    pub(crate) target: String,
    pub(crate) before: Option<Value>,
    pub(crate) after: Option<Value>,
}

pub(crate) struct MetaRelationsData {
    pub(crate) owners: Vec<MetaRelationTargetData>,
    pub(crate) register_records: Vec<MetaRelationTargetData>,
    pub(crate) based_on: Vec<MetaRelationTargetData>,
    pub(crate) input_by_string: Vec<MetaRelationTargetData>,
}

pub(crate) struct MetaRelationTargetData {
    pub(crate) kind: String,
    pub(crate) value: String,
}
```

- Add `effects: Vec<MetaMutationEffect>` to `MetaMutationData`.
- Add `required`, `fillValue`, and recursively typed nested attributes to `MetaElementData`.
- Replace the owners-only read projection with `relations: MetaRelationsData`; array order remains the authoritative collection order.

- [ ] **Step 1: Add failing round-trip tests**

Apply each of the five operation variants, then call `meta.info` and assert the changed property, element fields, collection order, and relation target are visible. Assert each mutation preview has one effect per input operation and that add-without-operations has one `createTemplate` effect.

- [ ] **Step 2: Run RED**

```sh
cargo test -p unica-coder meta_info_surface_tests::info_observes_every_typed_mutation_field -- --exact
cargo test -p unica-coder application::metadata::tests::preview_effects_follow_operation_order -- --exact
```

- [ ] **Step 3: Parse the complete public read model**

Reuse the existing typed parsers for `MetadataType`, fill values, relations, and collection nodes. Preserve source order rather than sorting elements. A malformed optional field sets `incomplete: true` and emits a validation diagnostic; it must not silently become a different value.

- [ ] **Step 4: Capture effects while applying operations**

Have the typed edit visitor capture the relevant semantic value before and after each operation. Do not compute effects by diffing serialized XML. Removal uses `after: null`, addition uses `before: null`, and update/property/relation effects contain normalized typed JSON.

- [ ] **Step 5: Run read/edit/add suites GREEN**

```sh
cargo test -p unica-coder meta_info -- --test-threads=1
cargo test -p unica-coder typed_edit -- --test-threads=1
cargo test -p unica-coder meta_add -- --test-threads=1
```

---

### Task 5: Close the retired DSL capability ledger

**Files:**

- Create: `spec/architecture/meta-capability-parity.json`
- Modify: `tests/ci/test_meta_surface_contract.py`
- Modify: `docs/migrations/0.12.0-meta-surface.md`
- Modify: `plugins/unica/skills/meta-edit/SKILL.md`
- Modify as proved: `crates/unica-coder/src/domain/metadata/properties.rs`
- Modify as proved: `crates/unica-coder/src/infrastructure/native_operations/meta/edit.rs`
- Modify as proved: `crates/unica-coder/src/infrastructure/native_operations/meta/info.rs`

**Interfaces:**

- The ledger has one entry per documented donor capability with fields `legacyKey`, `legacyKinds`, `status`, and exactly one of `typedOperation` or `removalReason`.
- `status` is `supported` only when schema, writer, validator, info readback, and round-trip fixture all exist.
- A removed capability names a concrete unsupported scenario; blanket reasons such as “DSL removed” are invalid.

- [ ] **Step 1: Add a failing completeness test against the tracked donor snapshot**

Parse the retired `types-*.md` tables under `tests/fixtures/provenance/retired_meta_dsl/meta-compile/reference/`, normalize each documented key, and fail when the ledger omits it or marks it supported without a corresponding current registry entry.

- [ ] **Step 2: Run RED**

```sh
python3.12 -m unittest tests.ci.test_meta_surface_contract.MetaSurfaceContractTests.test_retired_dsl_capabilities_are_accounted_for
```

- [ ] **Step 3: Port scalar properties that fit `setProperties`**

For each scalar property already emitted and validated by the current 23-kind templates, add a `MetadataPropertySpec` with exact value type, enum values where applicable, allowed-kind matrix, generic XML writer support, info parser support, and a literal round-trip test. Do not mark compound child structures supported by pretending they are scalar strings.

- [ ] **Step 4: Record compound removals explicitly**

For legacy batch creation, nested HTTP/Web service structures, schedule definitions, and child flags that have no typed operation in this PR, record the exact removed scenario and migration consequence. The migration note must no longer claim unconditional `compile -> add + edit` parity.

- [ ] **Step 5: Run ledger and round-trip tests GREEN**

```sh
python3.12 -m unittest tests.ci.test_meta_surface_contract
cargo test -p unica-coder metadata::properties -- --test-threads=1
cargo test -p unica-coder typed_properties -- --test-threads=1
```

---

### Task 6: Publish MCP-native structured results

**Files:**

- Modify: `crates/unica-coder/src/application/mod.rs`
- Modify: `crates/unica-coder/src/interfaces/mcp.rs`
- Modify: `tests/ci/test_unica_mcp_smoke.py`
- Modify: `scripts/ci/smoke-unica-mcp.py`

**Interfaces:**

- `ToolCallHandler` returns `Result<OperationResult, (i32, String)>` rather than rendered text.
- `CallToolResult::structured(value)` is used for `ok: true`; `CallToolResult::structured_error(value)` for `ok: false`.
- The four Meta tools publish one shared closed `OperationResult` output schema. `data`, `diagnostics`, and `job` remain open JSON subtrees because their shapes are operation-specific; non-Meta tools retain their existing wire representation in this PR.
- JSON-RPC routing/internal failures remain `ErrorData`; application/domain failures are tool results with `isError: true`.

- [ ] **Step 1: Add failing transport tests**

Assert `tools/list` contains `outputSchema`, a successful Meta call has matching `structuredContent` and `isError: false`, and invalid Meta arguments have `structuredContent.ok: false`, diagnostics, and `isError: true`.

- [ ] **Step 2: Run RED**

```sh
cargo test -p unica-coder interfaces::mcp::tests::tool_results_are_structured -- --exact
python3.12 -m unittest tests.ci.test_unica_mcp_smoke.UnicaMcpSmokeTests.test_meta_calls_publish_structured_results
```

- [ ] **Step 3: Move serialization to the protocol boundary**

Keep application calls returning `OperationResult`. Build a common output schema with required `ok`, `summary`, `changes`, `warnings`, `errors`, `artifacts`, and `cache`; attach it to Meta specs using `Tool::with_raw_output_schema`. Let rmcp create the compatibility text content from the same `Value` so text and structured representations cannot diverge; render non-Meta results through the existing text path.

- [ ] **Step 4: Run transport and smoke suites GREEN**

```sh
cargo test -p unica-coder interfaces::mcp -- --test-threads=1
python3.12 -m unittest tests.ci.test_unica_mcp_smoke tests.ci.test_unica_mcp_script_parity
```

---

### Task 7: Synchronize delivery contracts and verify the PR

**Files:**

- Modify: `spec/architecture/tool-surface.md`
- Modify: `docs/migrations/0.12.0-meta-surface.md`
- Modify: `plugins/unica/skills/meta-add/SKILL.md`
- Modify: `plugins/unica/skills/meta-edit/SKILL.md`
- Modify: `plugins/unica/skills/meta-info/SKILL.md`
- Modify: `spec/architecture/tool-surface-review.json` only if its machine contract changes

**Interfaces:**

- All examples conform to the published schema and use only four Meta tools.
- The migration note distinguishes preserved typed capabilities from deliberately removed ones by linking `spec/architecture/meta-capability-parity.json`.

- [ ] **Step 1: Update surface, skills, and migration examples**

Add one atomic rich-create example, examples for all five edit variants including scoped update/remove, an explicit related-info example, and structured result/effects guidance.

- [ ] **Step 2: Run focused contract and documentation gates**

```sh
python3.12 -m unittest \
  tests.ci.test_meta_surface_contract \
  tests.ci.test_tool_surface_ledger \
  tests.ci.test_unica_skills \
  tests.ci.test_package_unica_plugin \
  tests.ci.test_unica_mcp_smoke
python3.12 scripts/ci/check-architecture-sync.py --base origin/main
```

- [ ] **Step 3: Run full Rust quality gates**

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace -- --test-threads=1
```

- [ ] **Step 4: Run the complete Python and format gates**

```sh
python3.12 -m unittest discover -s tests/ci -p 'test_*.py'
python3.12 -m unittest tests.dev.test_verify_8_3_27_xml
git diff --check
```

- [ ] **Step 5: Review, commit, push, and update draft PR #309**

Review `origin/main...HEAD` against every acceptance bullet, request an independent code review, fix Critical and Important findings, commit intentional slices, push `codex/meta-surface-contract-design`, and rewrite the PR body so it names the current contract, verified commands, capability removals, and remaining risks.

## Release size assessment

The completed four-tool contract serializes to **1,275,431 bytes** of compact
JSON in the MCP `tools/list` result on 2026-08-04. The executable test ratchets
the immediate ceiling from 1,300,000 to 1,285,000 bytes, leaving schema growth
visible rather than silently consuming more model context. A follow-up schema
optimization should target less than 1,200,000 bytes by reducing repeated
per-kind branches and definitions while preserving the same four public tools;
the ratchet should be lowered with every measured reduction.
