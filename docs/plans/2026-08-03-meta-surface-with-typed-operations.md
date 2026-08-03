# Meta Surface with Typed Operations Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the six mixed-mechanism `unica.meta.*` tools with exactly one typed read (`info`) and three typed mutations (`add`, `edit`, `remove`), remove the public/file Meta JSON DSL without compatibility aliases, and preserve validation, RLM enrichment, preview, support/format guards, atomic publication, rollback, and `MetadataChanged` semantics.

**Architecture:** Provider-neutral metadata commands, types, diagnostics, and results live in `domain::metadata`; `application::metadata` owns argument conversion and the read/mutation sequence; Platform XML infrastructure resolves logical targets, builds byte-level post-images from an internal template catalog and typed operations, and publishes guarded plans; the internal validator checks both reads and post-images; RLM contributes only soft-failing `related` sections. The public registry switches once, after the four internal paths are executable, and no old name, physical selector, string mini-language, or file DSL remains callable.

**Tech Stack:** Rust 2021, `serde`/`serde_json`, `roxmltree`, the existing Platform XML resolver and `CompileTransaction`, workspace RLM, Python contract/package tests, MCP JSON-RPC smoke tests, and Platform XML 8.3.27/2.20 fixtures.

## Global Constraints

- The approved contract is `docs/design/2026-08-03-meta-surface-design.md` and ADR-0025. If implementation reveals a material public-contract choice not covered there, stop and amend the design/ADR for approval instead of inventing a second contract in code.
- The final public group is exactly `unica.meta.info`, `unica.meta.add`, `unica.meta.edit`, and `unica.meta.remove`; there are no runtime aliases for `compile`, `profile`, or `validate` and no alternate uppercase argument names.
- Existing accepted ADRs are historical. Do not rewrite ADR-0020, ADR-0021, or ADR-0023 to erase old tool names; change the active registry, ADR-0025, derived architecture documents, skills, and current acceptance material.
- `sourceSet + metadataPath` is the only selector for existing metadata objects. `sourceSet + kind + name` is the only selector for creation. Physical paths and provider identities never become public identity fields.
- `meta.add` creates exactly one minimal valid object and accepts no properties, child collections, template identifier, JSON definition, or batch. Rich creation is `add`, then a separate `edit` transaction.
- `meta.edit.operations` is non-empty, ordered, atomic, and converted to a closed Rust enum before infrastructure sees it. Its public item schema is shallow and has no nested `oneOf`; application validation enforces conditional fields.
- Public mutation arguments use lower camel case and default `dryRun` to `true`. Forced remove apply is valid only for `force=true`, `confirm=true`, and `dryRun=false` together.
- The Platform XML writer receives domain commands plus closed provider handles, never raw MCP JSON. RLM is not a correctness source and cannot turn a valid local `info` read into a hard failure.
- Every feature change begins with a failing test observed for the intended reason. Every task ends with its narrow green command and an intentional commit. Pure file extraction in Task 1 is behavior-preserving and is guarded by before/after characterization tests. This extraction deliberately precedes semantic changes so reviewers can distinguish moved legacy behavior from the replacement contract; the numbered implementation-form list in the design describes review slices, not a normative dependency order.
- Keep the public switch in one commit. Until Task 10, new handlers may be constructed directly by unit tests but must not appear in `tools/list`.
- Preserve BOM, EOL, XML declaration, self-closing style, trailing-newline state, exact preimages, format/support evidence, containment, cooperative locking, rollback, and no-op behavior already proved by current tests.
- Do not scan or delete historical donor snapshots merely because they contain old names. Delete a donor/parity fixture only after its current contract responsibility is either moved to a retained internal regression or deliberately removed from provenance.
- Before marking ADR-0025 `accepted`, refresh `origin/main`; if ADR-0025 has become occupied, renumber the unmerged record and all live references in the same commit.

---

## Target File Structure

Create these provider-neutral modules:

```text
crates/unica-coder/src/domain/metadata/
├── diagnostics.rs
├── mod.rs
├── operations.rs
├── properties.rs
├── results.rs
└── types.rs

crates/unica-coder/src/application/
└── metadata.rs
```

Split the current monolith only along the Meta responsibility boundaries:

```text
crates/unica-coder/src/infrastructure/native_operations/meta/
├── edit.rs
├── info.rs
├── legacy_dsl.rs          # temporary; deleted in Task 11
├── mod.rs
├── publisher.rs
├── remove.rs
├── template_catalog.rs
├── validation.rs
├── validation_context.rs
└── xml_model.rs
```

Add the infrastructure adapter that implements the application port without
making the rest of `native_operations` depend on application JSON:

```text
crates/unica-coder/src/infrastructure/metadata_operations.rs
```

Use a dedicated public integration-test entry point instead of adding another
large test block to `application/mod.rs`:

```text
crates/unica-coder/tests/platform_meta_surface.rs
crates/unica-coder/tests/platform/meta_surface.rs
```

---

### Task 1: Characterize and split the Meta monolith without changing behavior

**Files:**

- Rename/split: `crates/unica-coder/src/infrastructure/native_operations/meta.rs`
- Move: `crates/unica-coder/src/infrastructure/native_operations/meta_validation_context.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/mod.rs`
- Test: the moved `#[cfg(test)]` modules under `native_operations/meta/`

**Interfaces:**

- Preserve every current `pub(crate)` function used by `native_operations/mod.rs`, `application/mod.rs`, `format_guard.rs`, and the format corpus.
- Isolate legacy `JsonPath`, `DefinitionFile`, `Operation + Value`, type-string parsing, and batch helpers in `legacy_dsl.rs`; no new code may call that module after Task 3.
- Put XML parsing/emission primitives shared by templates, edit, info, and validation in `xml_model.rs`; keep behavior-specific orchestration in its named module.

- [ ] **Step 1: Record the characterization baseline**

Run the narrow existing suites before moving code:

```sh
cargo test -p unica-coder meta_ -- --test-threads=1
cargo test -p unica-coder format_guard::tests::meta_ -- --test-threads=1
cargo test -p unica-coder --test format_8_3_27_xml_corpus meta_ -- --test-threads=1
```

Expected: PASS on the current branch. Save the executed commands and counts in
the task notes; do not reinterpret an existing failure as a refactor failure.

- [ ] **Step 2: Create the module directory and move code by function anchor**

Move code, tests, and test-only hooks together. Do not cut by stale line number.
The responsibility anchors are:

- `validate_meta` and `meta_validate_*` → `validation.rs`;
- `analyze_meta_info*` and `meta_info_*` → `info.rs`;
- `remove_metadata_object*` and `meta_remove_*` → `remove.rs`;
- `compile_meta`, JSON-definition parsing, string type grammar, and batch DSL → `legacy_dsl.rs`;
- `meta_compile_*` and `emit_meta_*` routines that emit Platform XML independent of JSON input → `template_catalog.rs` or `xml_model.rs`;
- `edit_meta*` and `meta_edit_*` → `edit.rs`;
- transaction preparation/shared post-image publication → `publisher.rs`;
- the old `meta_validation_context.rs` → `validation_context.rs`.

Keep `mod.rs` limited to module declarations, compatibility re-exports required
by unchanged callers, shared constants during extraction, and colocated test
fixtures/hooks that genuinely cross module boundaries.

- [ ] **Step 3: Prove the split is behavior-preserving**

Run the three Step 1 commands again plus:

```sh
cargo fmt --all -- --check
git diff --check
```

Expected: the same tests pass; public tool names, schemas, and output bytes are
unchanged. Inspect `git diff --stat` and spot-check that code was moved, not
silently rewritten.

- [ ] **Step 4: Commit the structural seam**

```sh
git add crates/unica-coder/src/infrastructure/native_operations
git commit -m "refactor(meta): split native operations by responsibility"
```

---

### Task 2: Introduce the closed metadata domain algebra

**Files:**

- Create: `crates/unica-coder/src/domain/metadata/mod.rs`
- Create: `crates/unica-coder/src/domain/metadata/types.rs`
- Create: `crates/unica-coder/src/domain/metadata/properties.rs`
- Create: `crates/unica-coder/src/domain/metadata/operations.rs`
- Create: `crates/unica-coder/src/domain/metadata/diagnostics.rs`
- Create: `crates/unica-coder/src/domain/metadata/results.rs`
- Modify: `crates/unica-coder/src/domain/mod.rs`
- Modify: `crates/unica-coder/src/infrastructure/metadata_kinds.rs`
- Test: the new domain modules

**Interfaces:**

```rust
pub(crate) enum MetadataKind { /* exactly the current 23 proven kinds */ }

impl MetadataKind {
    pub(crate) const ALL: &'static [Self];
    pub(crate) fn as_str(self) -> &'static str;
    pub(crate) fn parse(value: &str) -> Result<Self, MetaDiagnostic>;
}

pub(crate) struct MetadataType {
    pub(crate) variants: Vec<MetadataTypeVariant>,
}

pub(crate) enum MetadataTypeVariant {
    String { length: u32, allowed_length: StringLengthMode },
    Number { digits: u32, fraction: u32, sign: NumberSign },
    Boolean,
    Date { fractions: DateFractions },
    ValueStorage,
    Reference { metadata_path: MetadataAddress },
    DefinedType { metadata_path: MetadataAddress },
}

pub(crate) enum MetaCollection {
    Attributes,
    TabularSections,
    Dimensions,
    Resources,
    EnumValues,
    Columns,
    Forms,
    Templates,
    Commands,
}

pub(crate) struct MetaScope {
    pub(crate) tabular_section: String,
}

pub(crate) struct MetaPosition {
    pub(crate) before: Option<String>,
    pub(crate) after: Option<String>,
}

pub(crate) enum MetaEditOperation {
    SetProperties { values: MetaPropertyChanges },
    Add { collection: MetaCollection, scope: Option<MetaScope>, elements: Vec<MetaElementDefinition> },
    Update { collection: MetaCollection, scope: Option<MetaScope>, elements: Vec<MetaElementUpdate> },
    Remove { collection: MetaCollection, scope: Option<MetaScope>, names: Vec<String> },
    EditRelations { relation: MetaRelation, mode: RelationEditMode, targets: Vec<MetadataReference> },
}
```

`MetadataKind::ALL` is the only 23-kind creation list. Convert
`infrastructure::metadata_kinds` into a physical layout registry keyed by this
domain enum; delete `META_COMPILE_SUPPORTED_TYPES` after callers migrate.

Model public element input as a shallow superset, then convert it to closed
domain variants. `add` uses `name`, optional `synonym`, `comment`, structured
`type`, `required`, `fillValue`, nested `attributes` for a new tabular section,
and structured `position` where applicable. `update` additionally accepts
`newName`; `name` remains the existing-element selector. Reject an empty update.
`remove` uses non-empty `names`. Collection-specific field legality belongs in
the domain conversion registry, not in writer branches.

Define a single domain `MetadataPropertySpec` registry in `properties.rs` with
public JSON name, value kind, and allowed metadata kinds. Give each entry a
closed `MetaPropertyKey`; map that key to Platform XML only in
`infrastructure/native_operations/meta/xml_model.rs`. `MetaPropertyChanges` may
not retain arbitrary `serde_json::Value` or an unknown key after conversion.

Define diagnostics with exactly these stable codes:

```text
invalid_arguments, unsupported_kind, capability_unavailable,
target_not_found, already_exists, support_locked, reference_conflict,
validation_failed, concurrent_modification, provider_unavailable,
rollback_failed
```

Each `MetaDiagnostic` carries `severity`, `message`, optional logical
`metadataPath`, optional zero-based `operationIndex`, and optional `field`.
Define typed `MetaInfoData`, `MetaMutationData`, validation status, publication
plan entries, related sections, and completeness/freshness enums in `results.rs`.

- [ ] **Step 1: Write failing domain tests**

Cover:

- all 23 `MetadataKind` spellings and rejection of `Bot`/unknown/case aliases;
- non-empty, unique composite type variants and numeric/string bounds;
- logical reference parsing through existing `MetadataAddress`;
- exactly one of `position.before`/`position.after`;
- allowed collection/scope combinations (`scope.tabularSection` only for nested attributes);
- `add` duplicate semantics, `update`/`remove` missing-target semantics as explicit domain errors, never upsert;
- exhaustive stable diagnostic-code serialization;
- property registry rejects an unknown key and a property unsupported by the selected kind.

Run:

```sh
cargo test -p unica-coder domain::metadata -- --test-threads=1
```

Expected RED: `domain::metadata` and the named types do not exist.

- [ ] **Step 2: Implement the minimum closed types and registries**

Reuse `domain::source_target::MetadataAddress`; do not create a second parser
for `Catalog.X`. Derive `Serialize` only for stable public result types. Keep
writer-only XML names out of serialized domain values.

- [ ] **Step 3: Run the domain suite and remove the duplicate kind list**

Run:

```sh
cargo test -p unica-coder domain::metadata -- --test-threads=1
cargo test -p unica-coder metadata_kinds -- --test-threads=1
```

Expected GREEN: 23 kinds have one owner and all conversion invariants pass.

- [ ] **Step 4: Commit the domain layer**

```sh
git add crates/unica-coder/src/domain crates/unica-coder/src/infrastructure/metadata_kinds.rs
git commit -m "feat(meta): add typed metadata domain algebra"
```

---

### Task 3: Build strict MCP argument conversion and shallow schemas off-registry

**Files:**

- Create: `crates/unica-coder/src/application/metadata.rs`
- Modify: `crates/unica-coder/src/application/mod.rs`
- Modify: `crates/unica-coder/src/application/tool_contracts.rs`
- Test: `crates/unica-coder/src/application/metadata.rs`
- Test: `crates/unica-coder/src/application/tool_contracts.rs`

**Interfaces:**

```rust
pub(crate) enum MetadataOperation { Info, Add, Edit, Remove }

pub(crate) enum MetadataRequest {
    Info(MetaInfoRequest),
    Add(MetaAddRequest),
    Edit(MetaEditRequest),
    Remove(MetaRemoveRequest),
}

pub(crate) fn parse_metadata_request(
    operation: MetadataOperation,
    args: &Map<String, Value>,
) -> Result<MetadataRequest, MetaFailure>;

pub(crate) fn metadata_input_schema(operation: MetadataOperation) -> Value;
```

Add `ToolHandler::Metadata { operation: MetadataOperation }`, but do not add a
new `ToolSpec` to `tools()` yet. Unit tests may construct a private `ToolSpec`
to call `input_schema_for_tool`.

Exact public argument sets:

- `info`: required `sourceSet`, `metadataPath`; optional `sections`, `limit`;
- `add`: required `sourceSet`, `kind`, `name`; optional `dryRun`;
- `edit`: required `sourceSet`, `metadataPath`, `operations`; optional `dryRun`;
- `remove`: required `sourceSet`, `metadataPath`; optional `dryRun`, `force`, `confirm`.

All four schemas use `additionalProperties: false`. `dryRun` defaults to true;
`limit` defaults to 20 per requested related section. Default info sections are
`modules`, `roles`, `subscriptions`, and `functionalOptions`;
`predefinedItems` is opt-in.

`operations.items` publishes the superset properties `op`, `values`,
`collection`, `scope`, `elements`, `names`, `relation`, `mode`, and `targets`,
but no nested `oneOf`. The converter checks required/forbidden combinations,
sets zero-based `operationIndex`, and emits field-specific diagnostics. Do not
route public types through `parse_meta_string_type`, pipe splitting, `;;`, or a
definition-file loader.

- [ ] **Step 1: Write failing schema snapshots**

Assert the exact property sets, required arrays, defaults, enums, nested
`additionalProperties: false`, 23-kind list, five operation tags, nine
collections, four relations, three relation modes, and absence of `JsonPath`,
`DefinitionFile`, `Operation`, `Value`, `ObjectPath`, `ConfigDir`, `Object`, and
all path aliases.

Also recursively assert that `operations.items` contains no `oneOf`, `anyOf`,
or `allOf`.

Run:

```sh
cargo test -p unica-coder application::tool_contracts::tests::meta_new_contract -- --test-threads=1
```

Expected RED: the off-registry metadata schemas do not exist.

- [ ] **Step 2: Write failing conversion-table tests**

Use table-driven cases for all five operations and every collection. Include
unknown fields, missing conditional fields, empty arrays, illegal scope,
`before + after`, raw string types, pipe flags, `;;`, unknown property keys,
and remove force without the complete triple gate.

Run:

```sh
cargo test -p unica-coder application::metadata::tests::parse_ -- --test-threads=1
```

Expected RED: no request converter exists.

- [ ] **Step 3: Implement schema generation and conversion from the same registries**

The schema and parser must consume `MetadataKind::ALL`, collection/relation
enums, and `MetadataPropertySpec`; do not maintain parallel hand-written
allowed lists. Runtime conversion is authoritative when a host accepts a
shape that violates conditional rules the shallow schema cannot express.

- [ ] **Step 4: Prove schemas and conversion green**

Run both Step 1 and Step 2 commands and:

```sh
cargo test -p unica-coder application::tool_contracts -- --test-threads=1
```

Expected GREEN: old registered tools are still public, while new schemas and
typed conversion pass direct tests.

- [ ] **Step 5: Commit the application contract seam**

```sh
git add crates/unica-coder/src/application crates/unica-coder/src/domain/metadata
git commit -m "feat(meta): define typed MCP request contracts"
```

---

### Task 4: Add one application coordinator and an opaque guarded mutation plan

**Files:**

- Modify: `crates/unica-coder/src/application/metadata.rs`
- Modify: `crates/unica-coder/src/application/ports.rs`
- Modify: `crates/unica-coder/src/application/mod.rs`
- Modify: `crates/unica-coder/src/infrastructure/application_ports.rs`
- Create: `crates/unica-coder/src/infrastructure/metadata_operations.rs`
- Modify: `crates/unica-coder/src/infrastructure/mod.rs`
- Test: `crates/unica-coder/src/application/metadata.rs`

**Interfaces:**

```rust
pub(crate) struct MetadataResourceImage {
    pub(crate) role: MetadataResourceRole,
    pub(crate) bytes: Vec<u8>,
}

pub(crate) struct MetadataValidationSubject {
    pub(crate) target: MetadataAddress,
    pub(crate) resources: Vec<MetadataResourceImage>,
}

pub(crate) struct MetadataRead {
    pub(crate) local: MetaLocalInfo,
    pub(crate) validation_subject: MetadataValidationSubject,
}

pub(crate) trait PreparedMetadataMutation: Send {
    fn preview(&self) -> &MetaMutationData;
    fn validation_subject(&self) -> &MetadataValidationSubject;
    fn publish(
        self: Box<Self>,
        cancellation: &CancellationToken,
    ) -> Result<MetaPublishReport, MetaFailure>;
}
```

Extend `ApplicationPorts` with four provider-neutral calls:

```rust
fn read_metadata_local(...) -> Result<MetadataRead, MetaFailure>;
fn read_metadata_related(...) -> MetaRelatedData;
fn validate_metadata(...) -> MetadataValidationResult;
fn prepare_metadata_mutation(...) -> Result<Box<dyn PreparedMetadataMutation>, MetaFailure>;
```

The opaque prepared object retains closed physical handles, exact preimages,
locks, and the concrete `CompileTransaction` only in infrastructure. Its
validation subject contains logical roles and bytes but no filesystem path.

`application::metadata::invoke` owns these sequences:

```text
info: parse → local read → validate → related enrichment → typed result
mutation: parse → resolve/capability/plan → validate post-image → preview OR publish → event
```

Preview returns `projected_events=[MetadataChanged]`, apply returns
`events=[MetadataChanged]`, and failure/no-op returns neither. Business failures
produce `OperationResult.ok=false` with typed `diagnostics`; schema-level bad
arguments retain the existing MCP invalid-arguments behavior. Successful and
failed Meta operations never duplicate the typed answer in `stdout`.

- [ ] **Step 1: Write coordinator tests with fake ports and a fake prepared plan**

Prove exact call order; validation blocks publish; preview never calls publish;
apply calls publish once; cancellation before publication writes nothing;
no-op emits no event; rollback failure remains an error; `info` keeps local
data when every related section is unavailable.

Run:

```sh
cargo test -p unica-coder application::metadata::tests::coordinator_ -- --test-threads=1
```

Expected RED: the port and coordinator do not exist.

- [ ] **Step 2: Implement the port types, coordinator, and no-op infrastructure adapter**

Wire `ToolHandler::Metadata` in the internal dispatch match, but keep it absent
from `tools()`. The infrastructure adapter may initially return
`capability_unavailable`; later tasks replace one method at a time.

- [ ] **Step 3: Run coordinator and application regressions**

```sh
cargo test -p unica-coder application::metadata -- --test-threads=1
cargo test -p unica-coder application::tests::lists_unica_orchestrator_scope -- --test-threads=1
```

Expected GREEN: application sequencing is proved and the published inventory
still contains the old six tools.

- [ ] **Step 4: Commit the orchestration seam**

```sh
git add crates/unica-coder/src/application crates/unica-coder/src/infrastructure
git commit -m "feat(meta): orchestrate typed metadata operations"
```

---

### Task 5: Make one internal validator serve reads and post-images

**Files:**

- Modify: `crates/unica-coder/src/infrastructure/native_operations/meta/validation.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/meta/validation_context.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/meta/xml_model.rs`
- Modify: `crates/unica-coder/src/infrastructure/metadata_operations.rs`
- Test: `crates/unica-coder/src/infrastructure/native_operations/meta/validation.rs`

**Interfaces:**

```rust
pub(crate) struct MetadataValidator;

impl MetadataValidator {
    pub(crate) fn validate(
        &self,
        subject: &MetadataValidationSubject,
        context: &WorkspaceContext,
    ) -> MetadataValidationResult;
}
```

Replace the CLI-oriented reporter/batch interface at the reusable core, but
retain a temporary adapter for public `unica.meta.validate` until Task 10. The
core validates UUIDs, mandatory structure, properties, standard attributes,
child elements, uniqueness, links, owner registration, remaining registrations
after remove, and kind-specific rules. It returns domain diagnostics rather
than formatting stdout.

- [ ] **Step 1: Write failing equivalence tests**

Feed the same valid and invalid resource images through the temporary legacy
adapter and `MetadataValidator`; assert equivalent classification. Cover a
well-formed semantic error, malformed XML, invalid owner registration, duplicate
child, invalid reference, and remove post-state without the deleted descriptor.

Run:

```sh
cargo test -p unica-coder native_operations::meta::validation::tests::internal_ -- --test-threads=1
```

Expected RED: only path/args-oriented validation exists.

- [ ] **Step 2: Extract the byte-image validator and map diagnostics**

Malformed XML is a hard `provider_unavailable`/parse failure for `info` because
no local structure can be returned. Well-formed semantic invalidity is
`validation.status="failed"` for `info` and `validation_failed` for mutation
post-images. Keep message text useful, but make code, target, operation index,
and field stable.

- [ ] **Step 3: Run validator and legacy regressions**

```sh
cargo test -p unica-coder native_operations::meta::validation -- --test-threads=1
cargo test -p unica-coder meta_validate -- --test-threads=1
```

Expected GREEN: the legacy public route still works temporarily and delegates
to the same validator used by the new port.

- [ ] **Step 4: Commit the internal service**

```sh
git add crates/unica-coder/src/infrastructure/native_operations/meta crates/unica-coder/src/infrastructure/metadata_operations.rs
git commit -m "refactor(meta): make validation an internal reusable service"
```

---

### Task 6: Implement `meta.add` from the internal 23-kind template catalog

**Files:**

- Modify: `crates/unica-coder/src/infrastructure/native_operations/meta/template_catalog.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/meta/xml_model.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/meta/publisher.rs`
- Modify: `crates/unica-coder/src/infrastructure/metadata_operations.rs`
- Modify: `crates/unica-coder/src/infrastructure/platform_xml_source_targets.rs`
- Modify: `crates/unica-coder/src/infrastructure/format_guard.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/compile_transaction.rs`
- Test: `crates/unica-coder/src/infrastructure/native_operations/meta/template_catalog.rs`
- Test: `crates/unica-coder/tests/platform/meta_surface.rs`
- Create: `crates/unica-coder/tests/platform_meta_surface.rs`

**Interfaces:**

```rust
pub(crate) trait MetadataTemplateCatalog {
    fn minimal_object(
        &self,
        source: &ResolvedSourceSet,
        kind: MetadataKind,
        name: &str,
    ) -> Result<MetadataPostImage, MetaFailure>;
}
```

Adapt the existing compile emitter; do not construct a legacy JSON definition
inside `meta.add`. A template chooses exact XML shape from `MetadataKind`, the
resolved source format/capability, and configuration context. It generates the
UUID, canonical synonym, mandatory properties, `InternalInfo`, standard
attributes, owner registration, and mandatory auxiliary resources.

Use `CompileTransaction` through `meta/publisher.rs` for create/replace/read
guards and rollback. Do not rename the shared transaction merely to make the
Meta wrapper prettier. `already_exists` covers both an existing descriptor and
a partial registration/resource footprint; never repair or overwrite.

- [ ] **Step 1: Write a failing 23-kind preview table**

For every `MetadataKind::ALL`, resolve one Platform XML configuration source,
prepare `MetaAddRequest`, and assert canonical future `metadataPath`, created
resource roles, owner registration, validation success, and no filesystem
write. Include read-capable/write-incapable source and unsupported format cases.

Run:

```sh
cargo test -p unica-coder --test platform_meta_surface meta_add_preview_ -- --test-threads=1
```

Expected RED: the infrastructure port cannot prepare add.

- [ ] **Step 2: Implement minimal templates without the old DSL parser**

Reuse XML emission functions only after changing their inputs to typed values.
No template may call `read_meta_compile_definition_guarded`,
`meta_compile_parse_attr`, `parse_meta_string_type`, or consume a
`serde_json::Map`.

- [ ] **Step 3: Write and pass 23-kind apply/duplicate/partial tests**

For every kind, apply into a fresh fixture, validate emitted XML and owner
registration, then call add again and assert `already_exists` with byte-identical
preimages. Add explicit partially registered descriptor/resource cases, a
concurrent owner change failpoint, rollback, support lock, and cancellation.

Run:

```sh
cargo test -p unica-coder --test platform_meta_surface meta_add_ -- --test-threads=1
cargo test -p unica-coder native_operations::meta::template_catalog -- --test-threads=1
```

Expected GREEN: all 23 kinds preview and apply through the typed template path.

- [ ] **Step 4: Commit typed creation**

```sh
git add crates/unica-coder/src/infrastructure crates/unica-coder/tests/platform_meta_surface.rs crates/unica-coder/tests/platform/meta_surface.rs
git commit -m "feat(meta): create minimal objects from internal templates"
```

---

### Task 7: Implement atomic ordered typed editing

**Files:**

- Modify: `crates/unica-coder/src/infrastructure/native_operations/meta/edit.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/meta/xml_model.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/meta/publisher.rs`
- Modify: `crates/unica-coder/src/infrastructure/metadata_operations.rs`
- Test: `crates/unica-coder/src/infrastructure/native_operations/meta/edit.rs`
- Test: `crates/unica-coder/tests/platform/meta_surface.rs`

**Interfaces:**

```rust
pub(crate) fn prepare_typed_edit(
    request: &MetaEditRequest,
    resolved: ResolvedMetadataObject,
    context: &WorkspaceContext,
) -> Result<Box<dyn PreparedMetadataMutation>, MetaFailure>;
```

Apply each `MetaEditOperation` to one in-memory working tree in array order.
The next operation reads the previous post-image. Record `operationIndex` on
every failure. Build one final transaction only after every operation and the
internal validator succeed. Preserve source byte style and publish nothing for
a semantically identical post-image.

- [ ] **Step 1: Write failing operation/collection matrix tests**

Cover all five operations across all allowed collections:

- `setProperties` for representative common and kind-specific properties;
- `add`, `update`, and `remove` for attributes, tabular sections, dimensions,
  resources, enum values, columns, forms, templates, and commands;
- scoped tabular-section attributes;
- `editRelations` for owners, register records, based-on, and input-by-string
  with add/remove/replace;
- structured string, number, boolean, date, value-storage, reference,
  defined-type, and composite types;
- position before/after;
- add duplicate, update/remove missing target, rename collision, invalid scope,
  invalid property, and no upsert.

Run:

```sh
cargo test -p unica-coder native_operations::meta::edit::tests::typed_ -- --test-threads=1
```

Expected RED: only the legacy inline/definition edit engine exists.

- [ ] **Step 2: Implement typed visitors over the existing byte-preserving XML model**

Translate domain variants directly to XML edits. Share Platform XML insertion,
type emission, fill-value, name, and enum validation; do not serialize domain
operations back into the legacy DSL or call `meta_edit_apply_inline_operation`
or `meta_edit_apply_definition`.

- [ ] **Step 3: Prove order, atomicity, no-op, and rollback**

Add integration tests where operation 2 uses a tabular section created by
operation 1; operation 3 fails and leaves every exact preimage unchanged;
an identical update publishes no bytes/events; a concurrent descriptor/owner
change fails; apply rollback restores all resources; rollback failure returns
the stable hard diagnostic.

Run:

```sh
cargo test -p unica-coder --test platform_meta_surface meta_edit_ -- --test-threads=1
cargo test -p unica-coder native_operations::meta::edit -- --test-threads=1
```

Expected GREEN: typed editing is complete without a public registry switch.

- [ ] **Step 4: Commit typed editing**

```sh
git add crates/unica-coder/src/infrastructure crates/unica-coder/tests/platform/meta_surface.rs
git commit -m "feat(meta): apply ordered typed edit operations atomically"
```

---

### Task 8: Convert remove to the logical selector and common mutation pipeline

**Files:**

- Modify: `crates/unica-coder/src/infrastructure/native_operations/meta/remove.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/meta/publisher.rs`
- Modify: `crates/unica-coder/src/infrastructure/metadata_operations.rs`
- Modify: `crates/unica-coder/src/infrastructure/format_guard.rs`
- Modify: `crates/unica-coder/src/application/operation_descriptors.rs`
- Test: `crates/unica-coder/src/infrastructure/native_operations/meta/remove.rs`
- Create: `crates/unica-coder/src/application/meta_remove_surface_tests.rs`
- Test: `crates/unica-coder/tests/platform_meta_surface.rs`

**Interfaces:**

`prepare_metadata_mutation(MetaMutationCommand::Remove(request))` resolves
`sourceSet + metadataPath` through the same closed target resolver as info/edit,
then plans descriptor/resource deletion, owner deregistration, subsystem
cleanup, and validation of the remaining post-state.

- [ ] **Step 1: Write failing logical-remove tests**

Cover preview/apply, unknown source set, missing object, containment, read-only
capability, format gate, support lock, discovered references, subsystem cleanup,
owner change, exact-preimage conflict, cancellation, rollback, and rollback
failure. Assert physical arguments are not required anywhere in the direct
typed request.

Add table tests for the force gate:

```text
force=false                              -> references block
force=true, confirm=false                -> invalid_arguments
force=true, confirm=true, dryRun=true    -> preview only
force=true, confirm=true, dryRun=false   -> forced apply
```

Run:

```sh
cargo test -p unica-coder application::meta_remove_surface_tests::meta_remove_ -- --test-threads=1
```

Expected RED: the private coordinator route cannot prepare typed remove. Before
Task 10, the external integration target cannot invoke an off-registry handler
without publishing a test backdoor; it proves only that the production registry
has not switched prematurely:

```sh
cargo test -p unica-coder --test platform_meta_surface meta_remove_ -- --test-threads=1
```

The external typed JSON-RPC gate moves atomically with registration in Task 10.

- [ ] **Step 2: Reuse the logical resolver, validator, and publisher**

Keep reference-search logic, but return typed `reference_conflict` diagnostics
with logical targets. Validation subject contains surviving registrations and
affected dependency resources, not the deleted descriptor.

- [ ] **Step 3: Run remove and transaction regressions**

```sh
cargo test -p unica-coder application::meta_remove_surface_tests::meta_remove_ -- --test-threads=1
cargo test -p unica-coder --test platform_meta_surface meta_remove_ -- --test-threads=1
cargo test -p unica-coder native_operations::meta::remove -- --test-threads=1
cargo test -p unica-coder compile_transaction -- --test-threads=1
```

Expected GREEN: the private cross-layer suite exercises parse/coordinator,
validator, publisher, events/cache and filesystem state; the external target
keeps the six-tool legacy registry unchanged; typed logical remove has the same
or stronger safety guarantees.

- [ ] **Step 4: Commit logical removal**

```sh
git add crates/unica-coder/src/application/operation_descriptors.rs crates/unica-coder/src/infrastructure crates/unica-coder/tests/platform/meta_surface.rs
git commit -m "feat(meta): remove objects through logical guarded targets"
```

---

### Task 9: Merge local info, validation, and RLM profile sections

**Files:**

- Modify: `crates/unica-coder/src/infrastructure/native_operations/meta/info.rs`
- Modify: `crates/unica-coder/src/infrastructure/metadata_operations.rs`
- Modify: `crates/unica-coder/src/infrastructure/rlm_navigation.rs`
- Modify: `crates/unica-coder/src/infrastructure/workspace_services.rs`
- Modify: `crates/unica-coder/src/domain/code_intelligence.rs`
- Modify: `crates/unica-coder/src/domain/metadata/results.rs`
- Modify: `crates/unica-coder/src/application/code_intelligence.rs`
- Test: `crates/unica-coder/src/infrastructure/rlm_navigation.rs`
- Test: `crates/unica-coder/tests/platform/meta_surface.rs`

**Interfaces:**

```rust
pub(crate) struct MetaRelatedSection<T> {
    pub(crate) status: RelatedStatus,       // ready|partial|unavailable
    pub(crate) freshness: RelatedFreshness, // current|stale|unknown
    pub(crate) total: usize,
    pub(crate) returned: usize,
    pub(crate) truncated: bool,
    pub(crate) items: Vec<T>,
    pub(crate) diagnostics: Vec<MetaDiagnostic>,
}
```

Move `MetaProfileResult`/`MetaProfileSection` ownership from
`domain::code_intelligence` to `domain::metadata`. Keep the internal fixed RLM
operation if workspace services need it, but remove its public tool-name
knowledge. `read_metadata_related` adapts RLM output into named modules, roles,
subscriptions, functional options, and predefined items.

- [ ] **Step 1: Write failing typed-info integration tests**

Assert local kind/name/synonym/support/properties/owners/collections plus
`validation`. Verify malformed XML is a hard failure; well-formed invalid XML
returns structure with `validation.status="failed"`.

For each related section cover `ready/current`, `partial`, `stale`, and
`unavailable`; default sections omit predefined items; explicit sections and a
per-section limit return `total`, `returned`, and `truncated`; limit never cuts
local XML structure. Simulate RLM timeout/start failure and keep the local
answer.

Run:

```sh
cargo test -p unica-coder --test platform_meta_surface meta_info_ -- --test-threads=1
```

Expected RED: current info has local text/data only and profile is a separate
code-intelligence tool.

- [ ] **Step 2: Implement one local read and soft-failing enrichment adapter**

Resolve and read local bytes once. Reuse that image for parsing and validation.
Treat data observed before a mutation-triggered index refresh as stale unless
the bounded readiness check proves current. Never label stale data current.

- [ ] **Step 3: Remove public-profile types from code-intelligence ownership**

Delete `CodeIntelligenceOperation::ObjectProfile` and the public request/result
routing only after `application::metadata` consumes the moved adapter. Keep
workspace RLM helper code private and named by capability, not by removed MCP
tool.

- [ ] **Step 4: Run info, RLM, and code-intelligence regressions**

```sh
cargo test -p unica-coder --test platform_meta_surface meta_info_ -- --test-threads=1
cargo test -p unica-coder rlm_navigation -- --test-threads=1
cargo test -p unica-coder code_intelligence -- --test-threads=1
```

Expected GREEN: typed info contains profile value without depending on RLM for
local correctness.

- [ ] **Step 5: Commit the combined read**

```sh
git add crates/unica-coder/src/application crates/unica-coder/src/domain crates/unica-coder/src/infrastructure crates/unica-coder/tests/platform/meta_surface.rs
git commit -m "feat(meta): fold validation and profile data into info"
```

---

### Task 10: Switch the public MCP surface exactly once

**Files:**

- Create: `tests/ci/test_meta_surface_contract.py`
- Modify: `crates/unica-coder/src/application/mod.rs`
- Modify: `crates/unica-coder/src/application/tool_contracts.rs`
- Modify: `crates/unica-coder/src/application/operation_descriptors.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/mod.rs`
- Modify: `tests/ci/test_unica_mcp_smoke.py`
- Modify: `scripts/ci/smoke-unica-mcp.py`
- Modify: `spec/architecture/invariants.md`
- Modify: `spec/decisions/0025-meta-surface-with-typed-operations.md`
- Modify: `spec/decisions/README.md`
- Modify: `spec/architecture/building-blocks.md`
- Modify: `spec/architecture/runtime.md`
- Modify: `spec/architecture/risks.md`
- Modify: `spec/acceptance/unica-mcp-validation.md`
- Modify: `spec/acceptance/format-profile-8-3-27.md`

**Contract:**

Add `INV-MCP-META-SURFACE`, owned by ADR-0025 and checked by
`tests/ci/test_meta_surface_contract.py`. The check must inspect the Rust
registry/schema path and assert:

- exactly four `unica.meta.*` tools and exact handlers;
- no retired names registered or accepted at runtime;
- exact argument sets and lower-camel names;
- typed-data/no-stdout contract;
- shallow `operations.items`;
- no physical/DSL argument aliases;
- preview defaults and remove triple gate.

- [ ] **Step 1: Refresh main and resolve the ADR number before the switch**

```sh
git fetch origin main
git log --oneline -- spec/decisions
```

If `0025` now exists on `origin/main`, renumber this branch-local proposed ADR
to the next free number and update the design, invariant owner, index, and plan
references before proceeding.

- [ ] **Step 2: Write the failing exact-surface contract test**

Run:

```sh
python3.12 -m unittest tests.ci.test_meta_surface_contract
```

Expected RED: current `tools()` still exposes compile/profile/validate and has
no add.

- [ ] **Step 3: Replace the six specs with four metadata handlers**

Remove native registrations for `meta-compile` and `meta-validate`, remove the
code-intelligence profile spec, and register:

```text
unica.meta.info   ToolHandler::Metadata { Info }    mutating=false
unica.meta.add    ToolHandler::Metadata { Add }     mutating=true
unica.meta.edit   ToolHandler::Metadata { Edit }    mutating=true
unica.meta.remove ToolHandler::Metadata { Remove }  mutating=true
```

Delete old schema branches, validation exceptions, help strings, path aliases,
and descriptor entries. New metadata descriptors are handler-resolved and may
not advertise native physical path groups. Keep `MetadataChanged` explicit in
the coordinator outcome rather than reintroducing a native fallback event.

- [ ] **Step 4: Mark the architecture contract effective in the same change**

Set ADR-0025 (or its refreshed number) to `accepted`, add the invariant with its
real test, and update current architecture/acceptance descriptions. Do not copy
the full rule into multiple files; refer to the invariant and ADR by ID.

- [ ] **Step 5: Run the exact surface, schema, architecture, and JSON-RPC tests**

```sh
python3.12 -m unittest \
  tests.ci.test_meta_surface_contract \
  tests.ci.test_architecture_registry \
  tests.ci.test_unica_mcp_smoke
cargo test -p unica-coder application::tool_contracts -- --test-threads=1
cargo test -p unica-coder application::tests::lists_unica_orchestrator_scope -- --test-threads=1
```

Expected GREEN: `tools/list` has exactly the final four Meta tools, direct calls
to retired names fail as unknown tools, and all four produce typed data without
stdout.

- [ ] **Step 6: Commit the single public switch**

```sh
git add crates/unica-coder/src/application crates/unica-coder/src/infrastructure/native_operations/mod.rs scripts/ci/smoke-unica-mcp.py tests/ci/test_meta_surface_contract.py tests/ci/test_unica_mcp_smoke.py spec/architecture/invariants.md spec/architecture/building-blocks.md spec/architecture/runtime.md spec/architecture/risks.md spec/acceptance/unica-mcp-validation.md spec/acceptance/format-profile-8-3-27.md spec/decisions/0025-meta-surface-with-typed-operations.md spec/decisions/README.md
git commit -m "feat(meta)!: replace mechanism tools with typed operations"
```

---

### Task 11: Delete the file/string Meta DSL after preserving XML evidence

**Files:**

- Delete: `crates/unica-coder/src/infrastructure/native_operations/meta/legacy_dsl.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/meta/mod.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/meta/edit.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/meta/template_catalog.rs`
- Delete: `plugins/unica/references/specs/meta-dsl-spec.md`
- Delete: `plugins/unica/skills/meta-edit/json-dsl.md`
- Delete: `plugins/unica/skills/meta-edit/child-operations.md`
- Delete: `plugins/unica/skills/meta-edit/properties-reference.md`
- Modify: `plugins/unica/references/specs/1c-config-objects-spec.md`
- Modify: `plugins/unica/references/specs/format-index.md`
- Modify: `plugins/unica/references/specs/README.md`
- Modify: `tests/ci/test_reference_format_profile.py`
- Modify: `tests/ci/test_reference_reachability.py`
- Modify: `tests/ci/test_unica_mcp_script_parity.py`
- Modify: `scripts/ci/refresh-cc-1c-parity.py`

- [ ] **Step 1: Audit unique format facts before deletion**

Compare every normative XML statement and example in the four deleted Markdown
files with `1c-config-objects-spec.md`, indexed format fixtures, and current
emitter tests. Move only facts about Platform XML layout/order/defaults/encoding;
do not preserve JSON keys, file DSL grammar, string operations, or obsolete
workflow prose under another name.

Add failing format-profile tests for each fact that was unique and remains
necessary.

- [ ] **Step 2: Remove public parity cases that only prove retired routes**

Delete `meta-compile`/`meta-validate` public donor scenarios and their generated
reference models only when no current test consumes them. Retain or relocate
specific XML/validator fixtures needed to prove internal templates and
validation. Do not delete the immutable `cc-1c-skills` donor snapshot.

- [ ] **Step 3: Delete legacy code and prove no production caller remains**

Remove definition-file loading, inline operation/value parsing, `;;` batch
parsing, pipe flag parsing, string type parsing, compile stdout formatting, and
the temporary validate adapter. Keep only typed emitters, typed edit visitors,
and internal validator code.

Run a tracked-file search, excluding historical ADR/design/plan and immutable
donor snapshots:

```sh
git grep -n -E 'unica\.meta\.(compile|validate)|DefinitionFile|JsonPath|Operation.*Value|meta-dsl-spec|json-dsl' -- \
  ':!spec/decisions/**' ':!docs/design/**' ':!docs/plans/**' \
  ':!tests/fixtures/unica_mcp_script_parity/cc-1c-skills/**'
```

Expected: no live production, skill, current reference, or public-test match.
If fixture paths differ, enumerate the actual historical exclusions; do not use
a broad exclusion that hides live package content.

- [ ] **Step 4: Run format and native Meta regressions**

```sh
python3.12 -m unittest \
  tests.ci.test_reference_format_profile \
  tests.ci.test_reference_reachability \
  tests.ci.test_unica_mcp_script_parity
cargo test -p unica-coder native_operations::meta -- --test-threads=1
```

Expected GREEN: no public/file DSL remains and the Platform XML facts still have
executable evidence.

- [ ] **Step 5: Commit DSL retirement**

```sh
git add -A crates/unica-coder/src/infrastructure/native_operations/meta plugins/unica/references/specs plugins/unica/skills/meta-edit tests/fixtures/unica_mcp_script_parity/unica_reference_models/meta-compile tests/fixtures/unica_mcp_script_parity/unica_reference_models/meta-validate
git add tests/ci/test_reference_format_profile.py tests/ci/test_reference_reachability.py tests/ci/test_unica_mcp_script_parity.py scripts/ci/refresh-cc-1c-parity.py
git commit -m "refactor(meta)!: remove the external metadata DSL"
```

---

### Task 12: Rewrite skills, cross-skill routing, and provenance

**Files:**

- Delete: `plugins/unica/skills/meta-compile/`
- Delete: `plugins/unica/skills/meta-validate/`
- Create: `plugins/unica/skills/meta-add/SKILL.md`
- Modify: `plugins/unica/skills/meta-info/SKILL.md`
- Modify: `plugins/unica/skills/meta-edit/SKILL.md`
- Modify: `plugins/unica/skills/meta-remove/SKILL.md`
- Modify: `plugins/unica/references/platform/metadata-conventions.md`
- Modify all actual callers found under: `plugins/unica/skills/`
- Modify: `README.md`
- Modify: `spec/provenance/skill-upstreams.json`
- Modify: `tests/ci/test_unica_skills.py`
- Modify: `tests/ci/test_skill_provenance.py`

Known cross-skill callers to inspect include `api-design`, `cf-edit`,
`code-review`, `code-search`, `db-performance`, `integration-implement`,
`query-optimize`, `epf-init`, and `erf-init`; use `rg` to get the authoritative
list before editing.

- [ ] **Step 1: Write failing skill-routing tests**

Assert:

- exactly four Meta skills route MCP-first to the final tools;
- add examples use only `sourceSet/kind/name/dryRun`;
- edit examples use typed `operations[]` and structured types;
- info explains validation and related-section status/freshness/limits;
- remove explains logical selector and the force/confirm/apply triple gate;
- no prompt-visible file refers to retired tools, physical selectors, JSON DSL,
  direct packaged scripts, or string type/operation grammar.

Run:

```sh
python3.12 -m unittest tests.ci.test_unica_skills
```

Expected RED: old skills and routing are still packaged.

- [ ] **Step 2: Rewrite the four skills and all callers**

Keep visible guidance concise and operation-first. Put reusable long reference
tables under the retained skill/reference tree and link them; do not duplicate
the property/kind/operation contract across every caller.

- [ ] **Step 3: Reconcile provenance rather than deleting history**

In the `cc-1c-skills` upstream, rename the local adopted creation entry to
`meta-add` while retaining donor `.claude/skills/meta-compile/**` as upstream
evidence and documenting the deliberate semantic adaptation. Point local and
contract paths at the new skill, typed template implementation, and tests.

Move the old `meta-validate` donor entry from a public skill claim to the
internal validator owner (or another explicit non-prompt-visible adopted
entry supported by the checker schema); retain baseline and explain that only
the public route was removed. Update the `templates-new-object-1c` entry to the
retained `metadata-conventions`/`meta-add` owner and update tests that currently
look up the literal skill name `meta-validate`.

Do not fabricate `primarySource`, delete upstream paths, or discard donor
baseline commits merely to make the checker green.

- [ ] **Step 4: Run skills and provenance checks**

```sh
python3.12 -m unittest \
  tests.ci.test_unica_skills \
  tests.ci.test_skill_provenance
python3.12 scripts/ci/check-skill-upstreams.py --offline
```

Expected GREEN: every shipped skill/reference is reachable, provenance paths
exist, and retired Meta tools are absent from prompt-visible content.

- [ ] **Step 5: Commit package guidance and provenance**

```sh
git add -A plugins/unica/skills/meta-compile plugins/unica/skills/meta-validate plugins/unica/skills/meta-add plugins/unica/skills/meta-info plugins/unica/skills/meta-edit plugins/unica/skills/meta-remove
git add plugins/unica/skills/api-design/SKILL.md plugins/unica/skills/cf-edit/SKILL.md plugins/unica/skills/cf-edit/reference.md plugins/unica/skills/code-review/SKILL.md plugins/unica/skills/code-search/SKILL.md plugins/unica/skills/db-performance/SKILL.md plugins/unica/skills/epf-init/SKILL.md plugins/unica/skills/erf-init/SKILL.md plugins/unica/skills/integration-implement/SKILL.md plugins/unica/skills/query-optimize/SKILL.md plugins/unica/references/platform/metadata-conventions.md README.md spec/provenance/skill-upstreams.json tests/ci/test_unica_skills.py tests/ci/test_skill_provenance.py
git commit -m "docs(meta)!: route skills through the four operation contracts"
```

---

### Task 13: Synchronize tool ledger, package version, and migration note

**Files:**

- Modify: `spec/architecture/tool-surface-review.json`
- Regenerate: `spec/architecture/tool-surface.md`
- Modify: `scripts/ci/release-assessment.py`
- Modify: tests covering release assessment/tool ledger
- Modify: `Cargo.toml`
- Modify: `Cargo.lock`
- Modify: `plugins/unica/.codex-plugin/plugin.json`
- Modify: `plugins/unica/.claude-plugin/plugin.json`
- Modify: `plugins/unica/third-party/tools.lock.json`
- Modify: `tests/ci/test_tool_surface_ledger.py`
- Modify: `tests/ci/test_release_assessment.py`
- Modify: `tests/ci/test_version_contract.py`
- Modify: `tests/ci/test_package_unica_plugin.py`
- Create: `docs/migrations/README.md`
- Create: `docs/migrations/0.12.0-meta-surface.md`
- Modify: `README.md`

This is a pre-1.0 breaking feature increment from 0.11.0 to 0.12.0, not an
authorization to publish a release. Both host manifests and the Rust package
must remain identical in version.

- [ ] **Step 1: Write failing ledger/package/migration tests**

Require exact four-tool ledger membership, typed result declaration, no retired
release-assessment probe, equal 0.12.0 manifests/package, and a migration table:

```text
meta.compile         -> meta.add, then meta.edit
meta.profile         -> meta.info.related
meta.validate        -> meta.info.validation / automatic mutation validation
ObjectPath           -> sourceSet + metadataPath
ConfigDir + Object   -> sourceSet + metadataPath
Operation + Value    -> operations[]
DefinitionFile       -> removed
```

- [ ] **Step 2: Bump all package versions with the repository helper**

```sh
python3.12 scripts/dev/bump-version.py 0.12.0
```

Inspect the diff and reject unrelated generated changes.

- [ ] **Step 3: Regenerate and verify the tool surface from the built binary**

```sh
cargo build -p unica-coder
python3.12 scripts/ci/generate-tool-surface.py --binary target/debug/unica
python3.12 -m unittest tests.ci.test_tool_surface_ledger
```

Expected GREEN: source ledger, generated Markdown, and binary expose the same
four Meta tools.

- [ ] **Step 4: Run package parity tests**

```sh
python3.12 -m unittest \
  tests.ci.test_tool_surface_ledger \
  tests.ci.test_release_assessment \
  tests.ci.test_version_contract \
  tests.ci.test_package_unica_plugin \
  tests.ci.test_unica_mcp_smoke \
  tests.ci.test_unica_skills
```

Expected GREEN: source and packaged plugins expose identical contracts for the
Codex and Claude Code manifests.

- [ ] **Step 5: Commit synchronized delivery metadata**

```sh
git add Cargo.toml Cargo.lock plugins/unica/.codex-plugin/plugin.json plugins/unica/.claude-plugin/plugin.json plugins/unica/third-party/tools.lock.json spec/architecture/tool-surface-review.json spec/architecture/tool-surface.md scripts/ci/release-assessment.py tests/ci/test_tool_surface_ledger.py tests/ci/test_release_assessment.py tests/ci/test_version_contract.py tests/ci/test_package_unica_plugin.py docs/migrations/README.md docs/migrations/0.12.0-meta-surface.md README.md
git commit -m "chore(meta): prepare the 0.12 typed surface migration"
```

---

### Task 14: Run the full reliability and delivery gate

**Files:**

- Modify only defects introduced by this branch, in the task/commit that owns
  them.
- Verify: the complete tracked change set.

- [ ] **Step 1: Format, lint, and run all Rust tests**

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features -- --test-threads=1
```

Expected: all commands exit 0. A failing pre-existing test must be proved
against `origin/main`; do not silently waive it.

- [ ] **Step 2: Run the complete Python CI corpus**

```sh
python3.12 -m unittest discover -s tests/ci -p 'test_*.py'
```

Expected: all contract, package, provenance, documentation, and architecture
checks pass.

- [ ] **Step 3: Run Platform XML verification**

```sh
python3.12 -m unittest tests.dev.test_verify_8_3_27_xml
cargo test -p unica-coder --test format_8_3_27_xml_corpus -- --test-threads=1
```

If the dev verifier requires the private 1Ci corpus, first confirm
`docs-local/1ci/8.3.27/en/manifest.json` is complete; download only through the
repository helper described in `AGENTS.md` when it is absent.

- [ ] **Step 4: Verify architecture synchronization and tracked-file hygiene**

```sh
python3.12 scripts/ci/check-architecture-sync.py --base origin/main
git diff --check origin/main...HEAD
git status --short
```

Repeat the targeted retired-contract `git grep` from Task 11. Manually inspect
any historical-only matches and make exclusions exact.

- [ ] **Step 5: Exercise real MCP JSON-RPC against source and package**

Build/package with the repository’s supported command, run `initialize`,
`tools/list`, and representative preview calls for all four tools. Confirm the
same schema survives the supported Codex and Claude Code package loaders, with
special attention to `operations.items` and the absence of nested composition.

- [ ] **Step 6: Review the final diff against every approved acceptance bullet**

Check all 23 add kinds; five edit operations/nine collections/four relations;
sequential atomicity; read freshness/completeness; remove triple gate; validation;
guards/preimages/locks/cancellation/rollback; event/cache behavior; no stdout;
no aliases/DSL; skills/provenance/package/version/migration.

- [ ] **Step 7: Commit only verification-driven fixes, then request review**

Use focused commit messages for any defects found. Do not create a generic
“fix tests” commit and do not publish, push, or open a PR without the user’s
explicit next instruction.
