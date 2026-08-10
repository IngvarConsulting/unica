# EventSubscription logical binding implementation plan

> **For Codex:** Execute this plan task by task with red-green-refactor. Do not
> move compatibility rules into `xml_model.rs`; ADR-0039 and
> `INV-MCP-EVENT-BINDING` own the boundary.

**Goal:** Make `unica.meta.add`, `unica.meta.edit`, and `unica.meta.info` model
and validate one atomic `EventSubscription` binding from logical sources through
the canonical event to an exact common-module procedure.

**Architecture:** A new pure domain module owns the 8.3.27 source/event/signature
catalog and binding predicates. Application owns the closed JSON algebra and
final-state orchestration. Infrastructure resolves metadata/XML/BSL evidence
into domain facts and binds every byte used by validation to the publication
transaction. `xml_model.rs` remains a lossless profile adapter.

**Tech stack:** Rust, serde/serde_json, roxmltree, bsl-parser AST, existing
metadata transaction planner, Python unittest CI guards, exact 1C 8.3.27 XML
fixtures.

---

## Task 1: Public source and scalar-property contract

**Files:**

- Modify: `crates/unica-coder/src/domain/metadata/types.rs`
- Modify: `crates/unica-coder/src/domain/metadata/properties.rs`
- Modify: `crates/unica-coder/src/domain/metadata/operations.rs`
- Modify: `crates/unica-coder/src/application/metadata.rs`
- Test: tests colocated in those four Rust files
- Test: `tests/ci/test_meta_surface_contract.py`

**Step 1: Write failing tests**

- Assert the source union accepts only `object`, `manager`, `recordSet`,
  `definedType`, and `family`.
- Assert primitive, `valueStorage`, `reference`, unknown fields, unknown
  `sourceClass`, add/remove mode, and empty final source are rejected.
- Assert `Event` and `Handler` are string properties only of
  `EventSubscription`.
- Assert `Handler` requires `CommonModule.<Module>.<Procedure>` and `Event`
  requires a non-empty canonical candidate string.
- Assert the five `op` tags are unchanged.

**Step 2: Run the focused tests and observe the intended failures**

Run:

```bash
cargo test -p unica-coder event_source -- --test-threads=1
cargo test -p unica-coder event_subscription_properties -- --test-threads=1
python3.12 -m unittest tests.ci.test_meta_surface_contract
```

**Step 3: Implement the minimal domain types, parser, and schema**

- Replace format-shaped source variants with the logical variants.
- Add a closed serializable `EventSourceClass` identifier for `family`.
- Add `Event` and `Handler` to the property registry with owner-specific rules.
- Keep final binding compatibility out of argument parsing.

**Step 4: Run focused tests until green**

**Step 5: Commit**

```bash
git add crates/unica-coder/src/domain/metadata/types.rs \
  crates/unica-coder/src/domain/metadata/properties.rs \
  crates/unica-coder/src/domain/metadata/operations.rs \
  crates/unica-coder/src/application/metadata.rs \
  tests/ci/test_meta_surface_contract.py
git commit -m "feat(meta): expose logical event subscription binding"
```

## Task 2: Domain catalog and pure binding rules

**Files:**

- Create: `crates/unica-coder/src/domain/metadata/event_subscription.rs`
- Modify: `crates/unica-coder/src/domain/metadata/mod.rs`
- Test: colocated tests in `event_subscription.rs`

**Step 1: Write failing table tests**

- Assert the profile contains exactly 41 source classes and 183
  source/event/signature rows established from 8.3.27.2074.
- Pin representative object, manager, register record-set, sequence,
  recalculation, settings/filter and external-data-source rows.
- Assert event names are exact and case-sensitive.
- Assert event intersection requires presence on every source.
- Assert the same event name with different signatures is rejected.
- Assert handler arity is event arity plus one.

**Step 2: Observe the red tests**

Run:

```bash
cargo test -p unica-coder event_subscription_binding -- --test-threads=1
```

**Step 3: Implement the minimum profile**

- Encode logical class IDs, canonical events and signature IDs/arity in a
  profile-specific static catalog.
- Expose pure lookup, intersection, signature-unification and final binding
  validation functions.
- Keep QName, XML tag and file path data out of the module.

**Step 4: Run focused tests until green and refactor duplicate rows only after
the literal evidence assertions pass**

**Step 5: Commit**

```bash
git add crates/unica-coder/src/domain/metadata/event_subscription.rs \
  crates/unica-coder/src/domain/metadata/mod.rs
git commit -m "feat(meta): add 8.3.27 event binding domain"
```

## Task 3: Wire mapping and logical dependency resolution

**Files:**

- Modify: `crates/unica-coder/src/infrastructure/native_operations/meta/xml_model.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/meta/validation_context.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/meta/edit.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/meta/info.rs`
- Modify: `crates/unica-coder/src/infrastructure/metadata_kinds.rs`
- Modify: `crates/unica-coder/src/domain/source_target.rs`
- Modify: `crates/unica-coder/src/infrastructure/platform_xml_source_targets.rs`
- Test: colocated tests in the modified Rust files

**Step 1: Write failing resolver tests**

- Map concrete object, manager, register record-set, sequence record-set and
  family TypeSet wire forms to logical classes.
- Parse and resolve
  `CalculationRegister.<Register>.Recalculation.<Name>` without accepting a raw
  `RecalculationRecordSet` selector.
- Reject a concrete `ExternalDataSource` path until its root topology exists.
- Preserve syntactically readable unsupported wire data as a diagnostic, not a
  legal mutation target.
- Bind exact owner/registration/descriptor bytes for every concrete source.

**Step 2: Run focused tests and verify the failures**

Run:

```bash
cargo test -p unica-coder event_source -- --test-threads=1
cargo test -p unica-coder recalculation_address -- --test-threads=1
```

**Step 3: Implement mapping and resolver changes**

- Introduce a private wire-source representation if needed to keep parsing
  broader than mutation.
- Map logical variants to exact profile QName only at the adapter edge.
- Extend nested logical addressing narrowly for `Recalculation`.
- Reuse the complete physical metadata registry rather than widening the
  public create-kind enum.

**Step 4: Run focused tests until green**

**Step 5: Commit**

```bash
git add crates/unica-coder/src/domain/source_target.rs \
  crates/unica-coder/src/infrastructure/metadata_kinds.rs \
  crates/unica-coder/src/infrastructure/platform_xml_source_targets.rs \
  crates/unica-coder/src/infrastructure/native_operations/meta/xml_model.rs \
  crates/unica-coder/src/infrastructure/native_operations/meta/validation_context.rs \
  crates/unica-coder/src/infrastructure/native_operations/meta/edit.rs \
  crates/unica-coder/src/infrastructure/native_operations/meta/info.rs
git commit -m "feat(meta): resolve logical event source classes"
```

## Task 4: DefinedType expansion

**Files:**

- Modify: `crates/unica-coder/src/infrastructure/native_operations/meta/edit.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/meta/info.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/meta/validation.rs`
- Test: colocated tests plus metadata integration fixtures

**Step 1: Write failing tests**

- Expand a DefinedType containing object classes.
- Expand nested DefinedType members.
- Reject empty, cyclic, primitive, reference, missing and foreign-owner members.
- Assert every registration and descriptor participates in concurrent-change
  rejection for dry-run/apply parity.

**Step 2: Observe failures**

Run:

```bash
cargo test -p unica-coder defined_type_event_source -- --test-threads=1
```

**Step 3: Implement recursive evidence expansion with a visited-address set**

**Step 4: Run focused tests until green**

**Step 5: Commit**

```bash
git add crates/unica-coder/src/infrastructure/native_operations/meta/edit.rs \
  crates/unica-coder/src/infrastructure/native_operations/meta/info.rs \
  crates/unica-coder/src/infrastructure/native_operations/meta/validation.rs
git commit -m "feat(meta): expand defined event source types"
```

## Task 5: Exact handler AST and CommonModule capability

**Files:**

- Modify: `crates/unica-coder/src/infrastructure/bsl_outline.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/meta/validation.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/meta/edit.rs`
- Test: colocated tests

**Step 1: Write failing tests**

- Reject substring-only, comment-only, function, non-export and wrong-arity
  matches.
- Accept the exact case-insensitive BSL procedure identifier with exact arity.
- Reject `Global=true` and `Server=false`; ignore unrelated client flags.
- Keep ScheduledJob validation behavior stable unless its existing contract
  explicitly shares the exact helper.

**Step 2: Run focused tests and observe the failures**

Run:

```bash
cargo test -p unica-coder event_handler -- --test-threads=1
```

**Step 3: Extract a small reusable AST method lookup from `bsl_outline.rs` and
replace the text search only for the shared validated path**

**Step 4: Run focused tests until green**

**Step 5: Commit**

```bash
git add crates/unica-coder/src/infrastructure/bsl_outline.rs \
  crates/unica-coder/src/infrastructure/native_operations/meta/validation.rs \
  crates/unica-coder/src/infrastructure/native_operations/meta/edit.rs
git commit -m "fix(meta): validate event handlers through BSL AST"
```

## Task 6: Final-state binding orchestration

**Files:**

- Modify: `crates/unica-coder/src/application/ports.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/meta/edit.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/meta/info.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/meta/validation.rs`
- Test: `crates/unica-coder/src/application/meta_add_surface_tests.rs`
- Test: `crates/unica-coder/src/application/meta_info_surface_tests.rs`
- Test: metadata integration tests colocated under native operations

**Step 1: Write failing integration tests**

- Change Source, Event and Handler in one request and accept only the final
  compatible image independent of operation order.
- Reject incompatible source/event, mixed signatures, empty source/event/
  handler, invalid module flags and wrong procedure arity.
- Assert no write/event on failure, dry-run/apply effect parity, no-op byte
  preservation and concurrent dependency rejection.
- Assert `meta.info` exposes properties and source while reporting invalid
  existing bindings without promoting them to legal mutation targets.

**Step 2: Observe the red integration tests**

Run:

```bash
cargo test -p unica-coder event_subscription_binding -- --test-threads=1
cargo test -p unica-coder meta_add_event_subscription -- --test-threads=1
cargo test -p unica-coder meta_info_event_subscription -- --test-threads=1
```

**Step 3: Implement one final-image evidence object and invoke the pure domain
validator after dependency collection and under the authoritative guard**

**Step 4: Run focused tests until green**

**Step 5: Commit**

```bash
git add crates/unica-coder/src/application/ports.rs \
  crates/unica-coder/src/application/meta_add_surface_tests.rs \
  crates/unica-coder/src/application/meta_info_surface_tests.rs \
  crates/unica-coder/src/infrastructure/native_operations/meta/edit.rs \
  crates/unica-coder/src/infrastructure/native_operations/meta/info.rs \
  crates/unica-coder/src/infrastructure/native_operations/meta/validation.rs
git commit -m "feat(meta): validate complete event subscription bindings"
```

## Task 7: Public documentation and exact platform fixtures

**Files:**

- Modify: `plugins/unica/references/specs/1c-config-objects-spec.md`
- Modify: `plugins/unica/skills/meta-add/SKILL.md`
- Modify: `plugins/unica/skills/meta-edit/SKILL.md`
- Modify: `plugins/unica/skills/meta-info/SKILL.md`
- Modify: `spec/architecture/tool-surface.md`
- Modify: `spec/architecture/change-checklist.md` if its capability row changes
- Modify: `crates/unica-coder/tests/format_8_3_27_xml_corpus.rs`
- Modify: exact EventSubscription corpus fixtures
- Test: `tests/ci/test_unica_skills.py`
- Test: `tests/ci/test_meta_surface_contract.py`

**Step 1: Write failing documentation/fixture assertions**

- Pin the atomic JSON example and remove primitive/reference claims.
- Keep only wire syntax in the XML format specification; point logical
  compatibility to ADR-0039/INV-MCP-EVENT-BINDING.
- Correct the information-register `BeforeWrite` fixture to
  `(Source, Cancel, Replace)`.
- Add manager/event and mismatch cases while documenting that importer success
  is not a signature oracle.

**Step 2: Run guards and observe failures before edits**

Run:

```bash
python3.12 -m unittest tests.ci.test_unica_skills tests.ci.test_meta_surface_contract
cargo test -p unica-coder --test format_8_3_27_xml_corpus
```

**Step 3: Synchronize all public evidence**

**Step 4: Run the same checks until green**

**Step 5: Commit**

```bash
git add plugins/unica/references/specs/1c-config-objects-spec.md \
  plugins/unica/skills/meta-add/SKILL.md \
  plugins/unica/skills/meta-edit/SKILL.md \
  plugins/unica/skills/meta-info/SKILL.md \
  spec/architecture/tool-surface.md spec/architecture/change-checklist.md \
  crates/unica-coder/tests/format_8_3_27_xml_corpus.rs \
  tests/ci/test_unica_skills.py tests/ci/test_meta_surface_contract.py
git commit -m "docs(meta): document event subscription binding"
```

## Task 8: Full verification and review

**Step 1: Format and run focused gates**

```bash
cargo fmt --all -- --check
cargo test -p unica-coder event_subscription_binding -- --test-threads=1
python3.12 -m unittest tests.ci.test_architecture_registry \
  tests.ci.test_design_documents tests.ci.test_meta_surface_contract \
  tests.ci.test_unica_skills
```

**Step 2: Run workspace verification**

```bash
cargo test --workspace
```

**Step 3: Inspect final diff and contract synchronization**

```bash
git diff origin/main...HEAD --check
git status --short
```

**Step 4: Run the exact 8.3.27 platform gate when the established local
installation is available; otherwise report it separately rather than treating
Rust fixture success as platform proof**

**Step 5: Commit any verification-only fixture corrections and prepare the
branch for push/PR only after all required checks are green**
