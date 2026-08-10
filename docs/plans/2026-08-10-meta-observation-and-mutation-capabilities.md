# Meta observation and mutation capabilities implementation plan

> **For Codex:** Execute this plan task by task with red-green-refactor. ADR-0042
> owns the observation/mutation boundary. Do not claim the complete read-model
> matrix of proposed ADR-0041 in this change.

**Goal:** Fix #367, #368, and #369 by making `unica.meta.info` a resilient
observer of real 1C storage, validating HTML pages as registered UTF-8 files,
and reporting platform types independently from the public mutation algebra.

**Architecture:** A closed child-storage profile describes the distinct 1C
topologies for forms, templates, and commands. Observation enumerates and reads
those profiles directly and localizes unavailable child evidence; mutation uses
the same topology profile but remains fail-closed and transactional. Public
read results use an observed-type model with an explicit mutation capability,
while edit arguments retain their closed writer type. XML emission remains at
the adapter boundary.

**Tech stack:** Rust, serde/serde_json, roxmltree, existing metadata transaction
planner, colocated Rust tests, Python contract guards, exact 1C 8.3.27 fixtures.

---

## Task 1: Child storage topology and observation independence (#367)

**Files:**

- Modify: `crates/unica-coder/src/infrastructure/native_operations/meta/edit.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/meta/info.rs`
- Test: colocated tests in `edit.rs` and `info.rs`
- Test: `crates/unica-coder/src/application/meta_info_surface_tests.rs`

**Step 1: Write failing tests**

- Observe an inline command whose only physical child file is optional
  `Commands/<Name>/Ext/CommandModule.bsl`; assert no standalone
  `Commands/<Name>.xml` is requested and forms remain visible.
- Assert a missing or malformed resource for one child produces a diagnostic
  scoped to that child without discarding independently observable siblings.
- Pin the storage matrix: forms/templates are owner references plus standalone
  descriptors; commands are inline owner descriptors with no standalone XML.

**Step 2: Observe the focused failures**

Run:

```bash
cargo test -p unica-coder meta_info_command -- --test-threads=1
cargo test -p unica-coder typed_child_storage -- --test-threads=1
```

**Step 3: Implement the minimum shared storage profile and observer**

- Introduce a private closed storage-profile enum for Form, Template, Command.
- Replace the `meta.info` call to `plan_typed_child_resources` with an
  observation-only collector that performs no mutation planning or guards.
- Read standalone descriptors only for forms/templates; use the inline command
  node as its descriptor and read only optional command payload files.
- Localize read/validation failures to the affected child and keep complete
  evidence for unaffected children.

**Step 4: Keep mutation topology honest**

- Make command add/update/remove mutate the inline owner descriptor without
  inventing a standalone command XML file.
- Keep form/template owner entries as references and their standalone
  descriptors as the authoritative child documents.
- Preserve transaction guards and publication effects for every real file.

**Step 5: Run focused tests until green and commit**

```bash
git add crates/unica-coder/src/infrastructure/native_operations/meta/edit.rs \
  crates/unica-coder/src/infrastructure/native_operations/meta/info.rs \
  crates/unica-coder/src/application/meta_info_surface_tests.rs
git commit -m "fix(meta): separate child observation from mutation planning"
```

## Task 2: HTML page resource semantics (#368)

**Files:**

- Modify: `crates/unica-coder/src/infrastructure/native_operations/meta/validation.rs`
- Test: colocated tests in `validation.rs`

**Step 1: Write failing tests**

- Accept a registered UTF-8 HTML page with `<!DOCTYPE html>`, ordinary HTML
  entities, and non-XHTML void elements.
- Reject invalid UTF-8.
- Keep `Ext/Template.xml` under the existing strict XML descriptor contract and
  reject unregistered/missing page topology through footprint validation.

**Step 2: Observe the red test**

Run:

```bash
cargo test -p unica-coder html_template_page -- --test-threads=1
```

**Step 3: Remove XML parsing from HTML-page validation**

- Validate the page bytes as UTF-8 only; registration and path membership stay
  owned by the HTML descriptor/child-footprint checks.
- Do not normalize, parse, rewrite, or strip a DOCTYPE from the page bytes.

**Step 4: Run focused tests until green and commit**

```bash
git add crates/unica-coder/src/infrastructure/native_operations/meta/validation.rs
git commit -m "fix(meta): validate html pages as registered utf8 resources"
```

## Task 3: Observed type model and UUID capability (#369)

**Files:**

- Modify: `crates/unica-coder/src/domain/metadata/types.rs`
- Modify: `crates/unica-coder/src/domain/metadata/results.rs`
- Modify: `crates/unica-coder/src/application/metadata.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/meta/xml_model.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/meta/edit.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/meta/info.rs`
- Test: colocated tests in the modified Rust files
- Test: `crates/unica-coder/src/application/meta_info_surface_tests.rs`

**Step 1: Write failing end-to-end read tests**

- Read the tracked 1C fixture containing `v8:UUID` and assert `ok: true`, an
  observed UUID variant, and an explicit mutation capability.
- Assert supported writer types report `editable`.
- Assert a syntactically valid but unknown platform QName is preserved as a
  localized incomplete/read-only observation with a warning, not a failed
  `meta.info` call.
- Assert malformed type XML still reports an error and cannot masquerade as a
  read-only platform type.

**Step 2: Observe the red failures**

Run:

```bash
cargo test -p unica-coder meta_info_uuid -- --test-threads=1
cargo test -p unica-coder observed_metadata_type -- --test-threads=1
```

**Step 3: Split reader and writer types**

- Keep `MetadataType` as the closed mutation input.
- Add a serializable observed type representation carrying platform variants
  and `mutationCapability` (`editable` or `readOnly`).
- Parse QName syntax with the namespace context of the original document;
  unknown but valid QNames become localized read-only observations.
- Add UUID to the writer algebra only after exact `v8:UUID` emission and
  parse/emit/parse round-trip tests pass; otherwise report it read-only.

**Step 4: Update application schema only for proven writer capabilities**

- If UUID is editable, add the closed `uuid` variant to the argument parser and
  JSON schema with no unrelated fields.
- Keep the five operation tags and all other writer contracts unchanged.

**Step 5: Run focused tests until green and commit**

```bash
git add crates/unica-coder/src/domain/metadata/types.rs \
  crates/unica-coder/src/domain/metadata/results.rs \
  crates/unica-coder/src/application/metadata.rs \
  crates/unica-coder/src/infrastructure/native_operations/meta/xml_model.rs \
  crates/unica-coder/src/infrastructure/native_operations/meta/edit.rs \
  crates/unica-coder/src/infrastructure/native_operations/meta/info.rs \
  crates/unica-coder/src/application/meta_info_surface_tests.rs
git commit -m "fix(meta): separate observed types from mutation types"
```

## Task 4: Normative contract and public guidance

**Files:**

- Modify: `spec/decisions/0042-meta-observation-does-not-depend-on-mutation.md`
- Modify: `spec/decisions/0028-typed-metadata-operations.md`
- Modify: `spec/architecture/invariants.md`
- Modify: `spec/architecture/tool-surface.md`
- Modify: `plugins/unica/references/specs/1c-config-objects-spec.md`
- Modify: relevant `plugins/unica/skills/**/SKILL.md` files discovered by `rg`
- Test: `tests/ci/test_architecture_sync_guard.py`
- Test: relevant contract tests discovered by `rg`

**Step 1: Write failing contract tests for the new invariant/surface**

- Pin the observer/mutator separation and the child-storage matrix at public
  seams without source-text implementation assertions.
- Pin the observed `mutationCapability` payload and any newly editable UUID
  input schema.

**Step 2: Promote the implemented decision**

- Move ADR-0042 from `proposed` to `accepted` without rewriting its decision.
- Mark ADR-0028 superseded by ADR-0042 using the repository lifecycle format.
- Add the derived invariant and update the tool surface/spec/skill wording.
- Keep ADR-0041 proposed: this PR implements only its compatible vertical
  slice, not the full `details` matrix.

**Step 3: Run architecture and contract guards until green and commit**

```bash
python3.12 scripts/ci/check-architecture-sync.py --base origin/main --strict
python3.12 -m unittest tests.ci.test_architecture_sync_guard
git add spec plugins tests/ci
git commit -m "docs(meta): accept observation mutation boundary"
```

## Task 5: Complete verification, publication, review, and merge

**Step 1: Run fresh verification**

```bash
cargo fmt --all -- --check
cargo test --workspace --all-targets
python3.12 -m unittest discover -s tests/ci -p 'test_*.py'
python3.12 -m unittest discover -s tests/dev -p 'test_*.py'
python3.12 scripts/ci/check-architecture-sync.py --base origin/main --strict
git diff --check origin/main...HEAD
```

**Step 2: Review the final diff against #367, #368, #369 and ADR-0042**

- Confirm no fake command descriptor is read or emitted.
- Confirm HTML page bytes are never XML-normalized.
- Confirm observation remains broader than mutation and diagnostics are local.
- Confirm no claim accepts the unimplemented remainder of ADR-0041.

**Step 3: Push and open one ready implementation PR against `main`**

The body must include `Fixes #367`, `Fixes #368`, and `Fixes #369`, exact test
evidence, the ADR-0042 boundary, and the explicit non-goal for ADR-0041.

**Step 4: Resolve all actionable review threads and CI failures in the same
head branch, rerun verification, and merge only when every required check is
green and no unresolved actionable thread remains.**
