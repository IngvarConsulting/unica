# LineNumberLength Support Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let `unica.meta.edit` change a tabular section's `LineNumberLength` without permitting values or owner contexts rejected by 1C:Enterprise 8.3.27.

**Architecture:** Keep `LineNumberLength` in the existing native `modify-ts` path, but carry an explicit owner-derived policy into the tabular-section property target. Resolve that policy only when the requested change contains the property, reuse platform XML owner discovery to read `CompatibilityMode`, validate and canonicalize the integer before replacing the existing XML element, and keep detailed dry-run preview in issue #53.

**Tech Stack:** Rust 2021, `roxmltree`, existing native metadata writer and `CompileTransaction`, Rust unit tests, Markdown skill documentation.

## Global Constraints

- The public boundary remains the single MCP server `unica` and tool `unica.meta.edit`.
- Accepted values are canonical integers from `5` through `9`.
- `Report`, `DataProcessor`, `ExternalReport`, and `ExternalDataProcessor` tabular sections do not support `LineNumberLength`.
- Compatibility modes through `Version8_3_26` keep the property fixed at `5`; `DontUse` and newer modes permit `5..=9`.
- Existing BOM, EOL, XML declaration, UUIDs, sibling properties, self-closing style, and trailing newline must remain unchanged.
- `dryRun=true` remains no-write; detailed payload validation and planned-change output remain owned by #53.

---

### Task 1: Add failing regression coverage

**Files:**
- Modify: `crates/unica-coder/src/infrastructure/native_operations/meta.rs`

**Interfaces:**
- Consumes: existing `edit_meta`, `meta_edit_args`, `sample_meta_object_xml`, and `emit_meta_tabular_section`.
- Produces: regression tests defining the public behavior required from the implementation.

- [ ] **Step 1: Add an apply test that changes only the existing scalar**

Add a test that writes a `Document` containing:

```xml
<TabularSection uuid="22222222-2222-4222-8222-222222222222">
    <Properties>
        <Name>SampleItems</Name>
        <Synonym/>
        <Comment/>
        <ToolTip/>
        <FillChecking>DontCheck</FillChecking>
        <LineNumberLength>5</LineNumberLength>
    </Properties>
    <ChildObjects/>
</TabularSection>
```

Run:

```rust
let outcome = edit_meta(
    &meta_edit_args(
        &object_path,
        "modify-ts",
        "SampleItems: lineNumberLength=9",
    ),
    &context,
);
```

Assert `outcome.ok`, `<LineNumberLength>9</LineNumberLength>`, absence of the old scalar, retained BOM/EOL, and equality with `original.replace("<LineNumberLength>5</LineNumberLength>", "<LineNumberLength>9</LineNumberLength>")`.

- [ ] **Step 2: Add table-driven invalid-value tests**

For `["", "4", "10", "5.5", "text", "-1"]`, run the same operation and assert:

```rust
assert!(!outcome.ok, "{value}: {outcome:?}");
assert!(outcome.errors.iter().any(|error| {
    error.contains("LineNumberLength")
        && error.contains("integer")
        && error.contains("5..=9")
}));
assert_eq!(fs::read(&object_path).unwrap(), before);
```

- [ ] **Step 3: Add owner-context tests**

Cover these cases with unchanged-file assertions:

```text
Report + lineNumberLength=9         -> property is not applicable
DataProcessor + lineNumberLength=9  -> property is not applicable
ExternalReport + lineNumberLength=9 -> property is not applicable
Document + CompatibilityMode=Version8_3_26 -> property is fixed at 5
standalone Document without Configuration.xml -> compatibility mode cannot be determined
```

Also extend the existing serializer test so both `Report` and `DataProcessor` omit `LineNumberLength`.

- [ ] **Step 4: Run the new tests and verify RED**

Run:

```bash
cargo test --package unica-coder line_number_length -- --test-threads=1
```

Expected: the apply test fails with `Unsupported modify property key 'lineNumberLength'`; the serializer test fails because `Report` still emits the property.

### Task 2: Implement the semantic property policy

**Files:**
- Modify: `crates/unica-coder/src/infrastructure/native_operations/meta.rs`

**Interfaces:**
- Consumes: `resolve_platform_xml_owners_with_provenance`, `PlatformXmlOwnerKind`, owner XML snapshots, and existing `MetaEditModifyTarget`.
- Produces: `MetaEditLineNumberLengthPolicy`, owner-policy resolution, canonical key mapping, and numeric validation shared by inline and JSON `modify-ts`.

- [ ] **Step 1: Add the owner-derived policy**

Define:

```rust
#[derive(Clone, Copy)]
pub(crate) enum MetaEditLineNumberLengthPolicy {
    Editable,
    FixedFive,
    NotApplicable,
    UnknownCompatibility,
}
```

Resolve `NotApplicable` for `Report`/`DataProcessor` and their external variants; otherwise inspect the configuration or extension owner returned by `resolve_platform_xml_owners_with_provenance`. Parse `CompatibilityMode` for configurations and `ConfigurationExtensionCompatibilityMode` for extensions. Map `DontUse` and versions newer than `Version8_3_26` to `Editable`, versions through `Version8_3_26` to `FixedFive`, and a standalone or missing property to `UnknownCompatibility`. Do not resolve or guard the owner for unrelated `meta.edit` changes.

- [ ] **Step 2: Carry the policy through both writer inputs**

Change the tabular-section target to:

```rust
TabularSection {
    line_number_length: MetaEditLineNumberLengthPolicy,
}
```

Pass the resolved policy from `edit_meta` through `meta_edit_apply_inline_operation`, `meta_edit_apply_definition`, `meta_edit_apply_definition_modify`, `meta_edit_modify_tabular_sections_from_definition`, and `meta_edit_modify_top_child` into `meta_edit_modify_tabular_section_properties`.

- [ ] **Step 3: Canonicalize and validate the key/value**

Map:

```rust
"linenumberlength" | "line_number_length" | "line-number-length"
```

to `LineNumberLength` only for `Editable`. Return context-specific errors for `FixedFive`, `NotApplicable`, and `UnknownCompatibility`.

In `meta_edit_modify_properties_range`, parse the value as an unsigned integer, require `5..=9`, serialize `parsed.to_string()`, and replace the existing scalar. If the scalar is absent, return a clear error rather than guessing an insertion position or compatibility profile.

- [ ] **Step 4: Correct serializer applicability**

Change the emitter guard to:

```rust
if !matches!(object_type, "DataProcessor" | "Report") {
    lines.push(format!(
        "{indent}\t\t<LineNumberLength>9</LineNumberLength>"
    ));
}
```

- [ ] **Step 5: Run focused tests and verify GREEN**

Run:

```bash
cargo test --package unica-coder line_number_length -- --test-threads=1
cargo test --package unica-coder edit_meta_modifies_tabular_section_properties -- --test-threads=1
```

Expected: all selected tests pass.

### Task 3: Document and verify the public contract

**Files:**
- Modify: `plugins/unica/skills/meta-edit/SKILL.md`
- Modify: `plugins/unica/skills/meta-edit/child-operations.md`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/meta.rs`

**Interfaces:**
- Consumes: the implemented `modify-ts` behavior.
- Produces: MCP-visible documentation and release-quality validation evidence.

- [ ] **Step 1: Document the supported key and restrictions**

Add this example:

```text
-Operation modify-ts -Value "Товары: lineNumberLength=9"
```

Document `5..=9`, stored-object applicability, the `Report`/`DataProcessor` exclusion, the compatibility-mode restriction, and that `add-ts` retains its existing syntax.

- [ ] **Step 2: Run focused and full Rust verification**

Run:

```bash
cargo fmt --all -- --check
cargo clippy --package unica-coder --all-targets --all-features -- -D warnings
cargo test --package unica-coder
```

Expected: exit code `0` for every command.

- [ ] **Step 3: Run public-contract and repository checks**

Run:

```bash
python3.12 -m unittest tests.ci.test_unica_skills tests.ci.test_unica_mcp_script_parity
git diff --check
```

Expected: all tests pass and `git diff --check` produces no output.

- [ ] **Step 4: Commit, push, and open a draft PR**

Stage only:

```text
crates/unica-coder/src/infrastructure/native_operations/meta.rs
plugins/unica/skills/meta-edit/SKILL.md
plugins/unica/skills/meta-edit/child-operations.md
docs/superpowers/plans/2026-07-24-line-number-length.md
```

Commit as `fix: support tabular line number length`, push `codex/issue-178-line-number-length`, and open a draft PR to `main`. The PR body must explain the root cause, platform constraints, tests, the #53 dry-run boundary, and contain `Closes #178`.
