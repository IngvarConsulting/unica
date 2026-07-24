# PR 184 Owner-Aware Metadata Validation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `unica.meta.validate` resolve every non-external object through its registered `Configuration.xml`, declare the exact PR 188 platform-XML read-set, and apply the 38-character rule only to platform types with `ListPresentation`.

**Architecture:** Integrate the PR branch with current `main`, then replace the hidden `InternalLocalOwnerOnly` argument with explicit private owner-shape validation entry points. A shared owner/read inspection supplies both the format guard and the public validator; Rust and the Python parity oracle use the same owner, language, type, and command-text rules. External reports and processors remain self-owned roots.

**Tech Stack:** Rust 2021, `roxmltree`, platform-owner/source-set resolution, Python 3.12, `lxml`, `unittest`, Git.

## Global Constraints

- Keep the public MCP boundary as one server named `unica` with tool `unica.meta.validate`.
- Base the implementation on current `main`, including PR 188 format guards.
- Remove every native argument-map/parser use of `InternalLocalOwnerOnly` and
  `internalLocalOwnerOnly`.
- Public `unica.meta.validate` always performs owner-aware validation.
- Every non-external object requires an owning `Configuration.xml` and matching `Configuration/ChildObjects` registration.
- `ExternalReport` and `ExternalDataProcessor` are self-owned and never inherit a neighboring configuration.
- Read registered language files only for types that support `ListPresentation`.
- Resolve language codes only from owner-registered `Languages/<Name>.xml`; do not use observed `v8:lang` or language-neutral fallback.
- Use non-empty `ListPresentation`, otherwise non-empty `Synonym`, independently for each registered language.
- Missing object translations remain valid; missing owner/profile infrastructure is an error.
- Keep Rust and Python oracle diagnostics equivalent after path normalization.
- Preserve private transactional owner-shape validation without turning it into full workspace validation.
- Fix causes rather than suppressing format, owner, or parity failures.
- Treat the exact read inspection as PR 188's platform-XML format set; BSL
  source reads and existence/directory-membership probes used by semantic
  checks are not platform-format inputs.

## File Map

- `crates/unica-coder/src/infrastructure/native_operations/meta.rs`
  - Public metadata validation, property checks, private post-write owner-shape validation.
- `crates/unica-coder/src/infrastructure/native_operations/meta_validation_context.rs`
  - New focused owner classification, registration, language profile, registrar scan, and deterministic read inspection.
- `crates/unica-coder/src/infrastructure/native_operations.rs`
  - Registers the new internal module.
- `crates/unica-coder/src/infrastructure/native_operations/cf.rs`
  - Separates full public configuration validation from private owner-only validation.
- `crates/unica-coder/src/infrastructure/native_operations/form.rs`
  - Switches form post-write checks to the typed metadata-owner validator.
- `crates/unica-coder/src/infrastructure/native_operations/help.rs`
  - Switches help post-write checks to the typed metadata-owner validator.
- `crates/unica-coder/src/infrastructure/native_operations/interface.rs`
  - Switches interface post-write checks to the typed metadata-owner validator.
- `crates/unica-coder/src/infrastructure/native_operations/role.rs`
  - Calls the typed configuration-owner validator after role writes.
- `crates/unica-coder/src/infrastructure/native_operations/subsystem.rs`
  - Exposes typed subsystem/configuration-owner post-write validators.
- `crates/unica-coder/src/infrastructure/native_operations/template.rs`
  - Switches template post-write checks to the typed metadata-owner validator.
- `crates/unica-coder/src/infrastructure/format_guard.rs`
  - Consumes the exact metadata read inspection and tests PR 188 coverage.
- `tests/ci/test_product_contracts.py`
  - Prevents reintroduction of the hidden argument-map switch.
- `tests/fixtures/unica_mcp_script_parity/reference_skills/meta-validate/scripts/meta-validate.py`
  - Mirrors the owner-aware algorithm and targeted errors.
- `tests/ci/test_unica_mcp_script_parity.py`
  - Points the parity scenario at the valid `Enum` fixture.
- `tests/fixtures/unica_mcp_script_parity/meta-validate-language-aware/**`
  - Replaces the impossible `CommonModule.ListPresentation` fixture with a registered `Enum`.
- `tests/fixtures/unica_mcp_script_parity/meta-validate-parity-owner/**`
  - Supplies a validated owner/language profile before the meta-compile parity setup.
- `tests/fixtures/unica_mcp_script_parity/cf-validate/Configuration.xml`
  - Read-only validated configuration envelope for the meta-compile owner fixture.
- `tests/fixtures/unica_mcp_script_parity/bsp/meta/Languages/Русский.xml`
  - Supplies the registered BSP language descriptor required by BSP meta-validate scenarios.
- `tests/fixtures/unica_mcp_script_parity/bsp/meta/Enums/ВажностьПроблемыУчета.xml`
  - Read-only Designer-exported Enum envelope used as the valid fixture template.
- `plugins/unica/references/platform/metadata-conventions.md`
  - Documents type scope, owner requirements, and command-text precedence.
- `plugins/unica/references/README.md`
  - Fixes the relative reference link.
- `tests/ci/test_reference_metadata_conventions.py`
  - New focused reader-facing documentation contract.

---

### Task 1: Integrate PR 188 And Establish A Baseline

**Files:**
- Merge: `origin/main` into `codex/pr-184-language-aware-fixes`
- Verify: no source edits in this task

**Interfaces:**
- Consumes: PR 184 head plus approved design commit `40a7f66`.
- Produces: a branch containing PR 188 `0fd0e28308eee228c82d562b55f985a636d4e091` and the current `origin/main`.

- [ ] **Step 1: Confirm the worktree is clean and refresh refs**

```bash
git status --short --branch
git fetch origin
```

Expected: the first command shows no tracked or untracked changes; fetch exits
zero.

- [ ] **Step 2: Merge current main without rewriting PR history**

```bash
git merge --no-edit origin/main
```

Expected: a merge commit is created without conflicts. Do not rebase or force
rewrite the contributor's PR history.

- [ ] **Step 3: Prove that PR 188 is present**

```bash
git merge-base --is-ancestor 0fd0e28308eee228c82d562b55f985a636d4e091 HEAD
git log --oneline --first-parent -6
```

Expected: the ancestry command exits zero and the log includes the new merge.

- [ ] **Step 4: Run the pre-change focused baseline**

```bash
cargo test -p unica-coder validate_meta_ -- --test-threads=1
cargo test -p unica-coder format_guard::tests::meta_validate_ -- --test-threads=1
python3.12 tests/ci/test_unica_mcp_script_parity.py -k meta_validate_language_aware
```

Expected: all existing tests pass. Record counts before adding new failing
tests.

---

### Task 2: Remove The Hidden Validation Switch

**Files:**
- Modify: `crates/unica-coder/src/infrastructure/native_operations/meta.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/cf.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/form.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/help.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/interface.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/role.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/subsystem.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/template.rs`
- Modify: `crates/unica-coder/src/infrastructure/format_guard.rs`
- Modify: `tests/ci/test_product_contracts.py`

**Interfaces:**
- Produces: `pub(crate) fn validate_cf_owner_path(path: &Path, context: &WorkspaceContext) -> Result<(), String>`.
- Produces: `pub(crate) fn validate_subsystem_owner_path(path: &Path, context: &WorkspaceContext) -> Result<(), String>`.
- Produces: `pub(crate) fn validate_metadata_owner_shape_8_3_27(path: &Path, context: &WorkspaceContext, operation: &str) -> Result<(), String>`.
- Removes: `MetaValidationOptions::follow_metadata_references`.
- Removes: parsing or insertion of `InternalLocalOwnerOnly` /
  `internalLocalOwnerOnly` in native argument maps.

- [ ] **Step 1: Add the failing source-contract test**

Add this method to `ProductContractTests`:

```python
def test_native_validators_do_not_expose_internal_local_owner_only_switch(self) -> None:
    repo_root = Path(__file__).resolve().parents[2]
    rust_root = repo_root / "crates" / "unica-coder" / "src"
    offenders = []
    for path in sorted(rust_root.rglob("*.rs")):
        text = path.read_text(encoding="utf-8")
        for marker in ("InternalLocalOwnerOnly", "internalLocalOwnerOnly"):
            if marker in text:
                offenders.append(
                    f"{path.relative_to(repo_root).as_posix()}: {marker}"
                )
    self.assertEqual(offenders, [])
```

- [ ] **Step 2: Run the contract test and verify RED**

```bash
python3.12 -m unittest \
  tests.ci.test_product_contracts.ProductContractTests.test_native_validators_do_not_expose_internal_local_owner_only_switch
```

Expected: FAIL listing `cf.rs`, `meta.rs`, `role.rs`, `subsystem.rs`, and the
format-guard test that still uses the marker.

- [ ] **Step 3: Split configuration validation into typed entry points**

In `cf.rs`, replace argument-map branching with an internal scope used only by
Rust callers:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CfValidationScope {
    Full,
    OwnerShape,
}

pub(crate) fn validate_cf(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> AdapterOutcome {
    validate_cf_with_scope(args, context, CfValidationScope::Full)
}

pub(crate) fn validate_cf_owner_path(
    path: &Path,
    context: &WorkspaceContext,
) -> Result<(), String> {
    let args = Map::from_iter([(
        "ConfigPath".to_string(),
        Value::String(path.display().to_string()),
    )]);
    let outcome = validate_cf_with_scope(&args, context, CfValidationScope::OwnerShape);
    if outcome.ok {
        Ok(())
    } else if outcome.errors.is_empty() {
        Err(outcome.summary)
    } else {
        Err(outcome.errors.join("; "))
    }
}
```

Move the existing `local_owner_only` behavior behind
`scope == CfValidationScope::OwnerShape`. Do not inspect an argument with that
name.

- [ ] **Step 4: Split metadata post-write validation from the public validator**

Remove `follow_metadata_references` from `MetaValidationOptions`. Delete
`require_metadata_8_3_27_validation` and replace it with one typed private
entry point. Keep scope out of argument maps:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MetaValidationScope {
    PublicOwnerAware,
    PostWriteLocal,
}

pub(crate) fn validate_metadata_owner_shape_8_3_27(
    object_path: &Path,
    workspace: &WorkspaceContext,
    operation: &str,
) -> Result<(), String> {
    let xml_text = read_utf8_sig(object_path)?;
    let document = Document::parse(xml_text.trim_start_matches('\u{feff}'))
        .map_err(|error| format!("XML parse error: {error}"))?;
    let root_object = meta_edit_object_node(&document)?;

    match root_object.tag_name().name() {
        "Configuration" => return validate_cf_owner_path(object_path, workspace),
        "Subsystem" => return validate_subsystem_owner_path(object_path, workspace),
        _ => {}
    }

    validate_metadata_8_3_27_boolean_contract(&xml_text, operation)?;
    validate_metadata_8_3_27_enum_contract(&xml_text, operation)?;
    let options = MetaValidationOptions {
        detailed: true,
        max_errors: 30,
        out_file_label: None,
        out_file: None,
    };
    let run = meta_validate_one_with_scope(
        object_path.to_path_buf(),
        &options,
        workspace,
        MetaValidationScope::PostWriteLocal,
    )?;
    if run.ok {
        Ok(())
    } else {
        Err(format!(
            "{operation} owner metadata validation failed for {}: {}",
            object_path.display(),
            run.errors.join("; ")
        ))
    }
}
```

Introduce `meta_validate_one_with_scope`; keep `meta_validate_one` as the
public-only wrapper:

```rust
pub(crate) fn meta_validate_one(
    path: PathBuf,
    options: &MetaValidationOptions,
    context: &WorkspaceContext,
) -> Result<MetaValidationRun, String> {
    meta_validate_one_with_scope(
        path,
        options,
        context,
        MetaValidationScope::PublicOwnerAware,
    )
}
```

Rename the existing implementation body to
`meta_validate_one_with_scope(path, options, context, scope)`. Do not duplicate
the body in two validators. Calculate every cross-object input in one scope
branch:

```rust
struct MetaValidationReferenceInputs {
    config_dir: Option<PathBuf>,
    language_codes: Vec<String>,
}

let reference_inputs = match scope {
    MetaValidationScope::PublicOwnerAware => {
        let config_dir = meta_validate_config_dir(&resolved_path);
        let language_codes = meta_validate_language_codes(config_dir.as_deref());
        MetaValidationReferenceInputs {
            config_dir,
            language_codes,
        }
    }
    MetaValidationScope::PostWriteLocal => MetaValidationReferenceInputs {
        config_dir: None,
        language_codes: Vec::new(),
    },
};
```

Pass these inputs to `meta_validate_check_properties`,
`meta_validate_check_cross_properties`, and
`meta_validate_check_method_reference`; do not rediscover a configuration
inside any checker. With `config_dir: None`, registrar scanning is also
disabled. Task 3 replaces the `PublicOwnerAware` branch with the exact owner
inspection.

Add `validate_subsystem_owner_path` as the same typed adapter around
`validate_subsystem`:

```rust
pub(crate) fn validate_subsystem_owner_path(
    path: &Path,
    context: &WorkspaceContext,
) -> Result<(), String> {
    let outcome = validate_subsystem(&subsystem_validation_args(path), context);
    require_subsystem_validation(&outcome).map_err(|error| {
        format!(
            "subsystem owner validation failed for {}: {error}",
            path.display()
        )
    })
}
```

This preserves the current local root, property, Boolean, enum, child,
uniqueness, and specialized Configuration/Subsystem checks. It deliberately
removes all owner/language/registrar reads from post-write validation. Public
`validate_meta` is always `PublicOwnerAware`.

Add a regression proving the private path stays local:

```rust
#[test]
fn post_write_metadata_owner_shape_does_not_require_workspace_owner() {
    let context = temp_context("post-write-local");
    let object = context.cwd.join("CommonModules/Local.xml");
    write_file(
        &object,
        &sample_meta_object_xml(
            "CommonModule",
            "Local",
            "",
            "\t\t<ChildObjects/>",
        ),
    );
    write_file(
        &context.cwd.join("Configuration.xml"),
        "<malformed-neighbor",
    );

    validate_metadata_owner_shape_8_3_27(&object, &context, "test")
        .expect("post-write validation must not read a neighboring owner");
}
```

- [ ] **Step 5: Replace configuration-owner argument maps**

Import and call `validate_cf_owner_path` directly from `meta.rs`, `role.rs`,
and `subsystem.rs`:

```rust
validate_cf_owner_path(config_path, context).map_err(|detail| {
    format!(
        "{operation} Configuration owner validation failed for {}: {detail}",
        config_path.display()
    )
})
```

Delete the internal-local format-guard test. It tests a mode that no longer
exists; later tasks replace it with full owner/read-set assertions.

Rename every import and call of `require_metadata_8_3_27_validation` in
`form.rs`, `help.rs`, `interface.rs`, `meta.rs`, and `template.rs` to
`validate_metadata_owner_shape_8_3_27`. In `subsystem.rs`, replace its
Configuration argument map with `validate_cf_owner_path`.

- [ ] **Step 6: Run focused tests and verify GREEN**

```bash
python3.12 -m unittest \
  tests.ci.test_product_contracts.ProductContractTests.test_native_validators_do_not_expose_internal_local_owner_only_switch
cargo test -p unica-coder native_operations::cf -- --test-threads=1
cargo test -p unica-coder native_operations::form -- --test-threads=1
cargo test -p unica-coder native_operations::help -- --test-threads=1
cargo test -p unica-coder native_operations::interface -- --test-threads=1
cargo test -p unica-coder native_operations::meta -- --test-threads=1
cargo test -p unica-coder post_write_metadata_owner_shape_does_not_require_workspace_owner -- --test-threads=1
cargo test -p unica-coder native_operations::role -- --test-threads=1
cargo test -p unica-coder native_operations::subsystem -- --test-threads=1
cargo test -p unica-coder native_operations::template -- --test-threads=1
```

Expected: all commands exit zero.

- [ ] **Step 7: Commit the validation entry-point cleanup**

```bash
git add \
  crates/unica-coder/src/infrastructure/native_operations/cf.rs \
  crates/unica-coder/src/infrastructure/native_operations/form.rs \
  crates/unica-coder/src/infrastructure/native_operations/help.rs \
  crates/unica-coder/src/infrastructure/native_operations/interface.rs \
  crates/unica-coder/src/infrastructure/native_operations/meta.rs \
  crates/unica-coder/src/infrastructure/native_operations/role.rs \
  crates/unica-coder/src/infrastructure/native_operations/subsystem.rs \
  crates/unica-coder/src/infrastructure/native_operations/template.rs \
  crates/unica-coder/src/infrastructure/format_guard.rs \
  tests/ci/test_product_contracts.py
git -c commit.gpgsign=false commit -m "Убрать скрытый режим локальной валидации"
```

---

### Task 3: Build The Owner-Aware Exact Read Inspection

**Files:**
- Create: `crates/unica-coder/src/infrastructure/native_operations/meta_validation_context.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/meta.rs`
- Modify: `crates/unica-coder/src/infrastructure/format_guard.rs`

**Interfaces:**
- Produces: `MetaValidationOwnerKind::{Configuration, Extension, External}`.
- Produces: `MetaValidationOwnerContext { object_type, object_name, owner_path, language_codes }`.
- Produces: `MetaValidationReadInspection { paths, context }`.
- Produces: `inspect_meta_validation_reads(path: &Path, workspace: &WorkspaceContext) -> MetaValidationReadInspection`.
- Produces: `meta_validate_types_with_list_presentation() -> &'static [&'static str]`.
- Consumes: PR 188 `resolve_platform_xml_owners` and project source-set classification.

- [ ] **Step 1: Add failing native owner/read-set tests**

In the existing `meta.rs` test module, add a fixture helper that writes a
registered object owner. Extend its imports to
`use serde_json::{json, Map, Value};` and
`use std::path::{Path, PathBuf};`.

```rust
const TEST_MD_NS: &str = "http://v8.1c.ru/8.3/MDClasses";
const TEST_V8_NS: &str = "http://v8.1c.ru/8.1/data/core";
const TEST_XR_NS: &str = "http://v8.1c.ru/8.3/xcf/readable";

fn write_owner(
    source_dir: &Path,
    object_type: &str,
    object_name: &str,
    languages: &[&str],
) -> PathBuf {
    fs::create_dir_all(source_dir.join("Languages")).unwrap();
    let language_nodes = languages
        .iter()
        .map(|name| format!("<Language>{name}</Language>"))
        .collect::<String>();
    let configuration = format!(
        r#"<MetaDataObject xmlns="{TEST_MD_NS}" version="2.20">
<Configuration uuid="11111111-1111-4111-8111-111111111111">
<Properties><Name>Owner</Name></Properties>
<ChildObjects>{language_nodes}<{object_type}>{object_name}</{object_type}></ChildObjects>
</Configuration></MetaDataObject>"#
    );
    fs::write(source_dir.join("Configuration.xml"), configuration).unwrap();
    source_dir.to_path_buf()
}

fn meta_validate_args(path: &Path) -> Map<String, Value> {
    Map::from_iter([
        (
            "ObjectPath".to_string(),
            Value::String(path.display().to_string()),
        ),
        ("Detailed".to_string(), Value::Bool(true)),
    ])
}

fn sample_meta_named(object_type: &str, object_name: &str) -> String {
    sample_meta_object_xml(
        object_type,
        object_name,
        "",
        "\t\t<ChildObjects/>",
    )
}
```

Add these tests:

```rust
#[test]
fn validate_meta_rejects_non_external_object_without_owner() {
    let context = temp_context("missing-owner");
    let object = context.cwd.join("Enums/Detached.xml");
    write_file(&object, &sample_meta_named("Enum", "Detached"));
    let outcome = validate_meta(&meta_validate_args(&object), &context);
    assert!(!outcome.ok, "{outcome:?}");
    assert!(
        outcome.errors.join("\n").contains("Configuration.xml"),
        "{outcome:?}"
    );
}

#[test]
fn validate_meta_rejects_object_missing_from_owner_registration() {
    let context = temp_context("missing-registration");
    let src = write_owner(
        &context.cwd.join("src"),
        "Enum",
        "Other",
        &["Русский"],
    );
    let object = src.join("Enums/Detached.xml");
    write_file(&object, &sample_meta_named("Enum", "Detached"));
    let outcome = validate_meta(&meta_validate_args(&object), &context);
    assert!(!outcome.ok, "{outcome:?}");
    assert!(
        outcome.errors.join("\n").contains("not registered"),
        "{outcome:?}"
    );
}

#[test]
fn validate_meta_external_descriptor_ignores_neighbor_configuration() {
    let context = temp_context("external-owner");
    write_file(
        &context.cwd.join("Configuration.xml"),
        r#"<broken-neighbor version="2.21">"#,
    );
    let object = context.cwd.join("tools/Standalone.xml");
    write_file(
        &object,
        &sample_meta_named("ExternalDataProcessor", "Standalone"),
    );
    let outcome = validate_meta(&meta_validate_args(&object), &context);
    assert!(outcome.ok, "{outcome:?}");
    assert!(!outcome.stdout.unwrap_or_default().contains("broken-neighbor"));
}

#[test]
fn meta_validation_context_classifies_registered_extension_owner() {
    let context = temp_context("extension-owner");
    write_file(
        &context.cwd.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: extension\n    type: EXTENSION\n    path: extension\n",
    );
    let source_dir = write_owner(
        &context.cwd.join("extension"),
        "CommonModule",
        "ExtensionModule",
        &[],
    );
    let object = source_dir.join("CommonModules/ExtensionModule.xml");
    write_file(
        &object,
        &sample_meta_named("CommonModule", "ExtensionModule"),
    );

    let inspection = inspect_meta_validation_reads(&object, &context);
    let owner = inspection.context.expect("registered extension owner");
    assert_eq!(owner.owner_kind, MetaValidationOwnerKind::Extension);
    assert_eq!(
        inspection.paths,
        vec![object, source_dir.join("Configuration.xml")]
    );
}
```

In `format_guard.rs`, add an exact-path assertion:

```rust
assert_eq!(
    effective_format_paths(descriptor, &args, &context(&root)).unwrap(),
    vec![
        object.clone(),
        configuration.clone(),
        russian.clone(),
        english.clone(),
    ]
);
```

Add a `meta_validate_owner` fixture helper in `format_guard.rs` that writes a
registered owner plus each declared language descriptor at a caller-supplied
format version. Update both PR 188 registrar tests to register
`AccumulationRegister.Sales` and `Language.Russian`, with all new owner/language
files at `2.20`. Their existing registrar assertions must continue to pass,
and their dependency vectors now include `Configuration.xml` and
`Languages/Russian.xml` before document-prefix entries.

- [ ] **Step 2: Run the new tests and verify RED**

```bash
cargo test -p unica-coder validate_meta_rejects_non_external_object_without_owner -- --nocapture
cargo test -p unica-coder validate_meta_rejects_object_missing_from_owner_registration -- --nocapture
cargo test -p unica-coder validate_meta_external_descriptor_ignores_neighbor_configuration -- --nocapture
cargo test -p unica-coder meta_validation_context_classifies_registered_extension_owner -- --nocapture
cargo test -p unica-coder meta_validate_dependencies_include_owner_and_registered_languages -- --nocapture
```

Expected: the first tests fail because standalone/nearest-ancestor fallback is
still accepted; the dependency test fails because language XML files are absent
from the PR 188 read-set.

- [ ] **Step 3: Add the focused context module**

Register the module in `native_operations.rs`:

```rust
pub(crate) mod meta_validation_context;
```

Start `meta_validation_context.rs` with:

```rust
use super::common::read_utf8_sig;
use super::meta::{
    meta_info_child,
    meta_info_inner_text,
    meta_validate_valid_types,
};
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::platform_xml_owner::{
    resolve_platform_xml_owners,
    PlatformXmlOwnerKind,
};
use roxmltree::Document;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

const MD_CLASSES_NS: &str = "http://v8.1c.ru/8.3/MDClasses";
```

Define the data boundary:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MetaValidationOwnerKind {
    Configuration,
    Extension,
    External,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ObjectIdentity {
    object_type: String,
    object_name: String,
    registrar_reference: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OwnerCandidate {
    kind: MetaValidationOwnerKind,
    path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OwnerCandidateError {
    attempted_path: Option<PathBuf>,
    message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ConfigurationOwner {
    kind: MetaValidationOwnerKind,
    path: PathBuf,
    registrations: Vec<(String, String)>,
    registered_languages: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MetaValidationOwnerContext {
    pub object_type: String,
    pub object_name: String,
    pub owner_kind: MetaValidationOwnerKind,
    pub owner_path: PathBuf,
    pub language_codes: Vec<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct MetaValidationReadInspection {
    pub paths: Vec<PathBuf>,
    pub context: Result<MetaValidationOwnerContext, String>,
}

pub(crate) fn meta_validate_types_with_list_presentation() -> &'static [&'static str] {
    &[
        "ExchangePlan",
        "FilterCriterion",
        "Catalog",
        "Document",
        "DocumentJournal",
        "Enum",
        "ChartOfCharacteristicTypes",
        "ChartOfAccounts",
        "ChartOfCalculationTypes",
        "InformationRegister",
        "AccumulationRegister",
        "AccountingRegister",
        "CalculationRegister",
        "BusinessProcess",
        "Task",
    ]
}

```

- [ ] **Step 4: Implement deterministic owner and registration resolution**

Implement `inspect_meta_validation_reads` with these exact states:

```rust
pub(crate) fn inspect_meta_validation_reads(
    object_path: &Path,
    workspace: &WorkspaceContext,
) -> MetaValidationReadInspection {
    let mut paths = vec![object_path.to_path_buf()];
    let identity = match read_object_identity(object_path) {
        Ok(identity) => identity,
        Err(error) => return inspection_error(paths, error),
    };

    if matches!(
        identity.object_type.as_str(),
        "ExternalReport" | "ExternalDataProcessor"
    ) {
        return inspection_ok(
            paths,
            MetaValidationOwnerContext {
                object_type: identity.object_type,
                object_name: identity.object_name,
                owner_kind: MetaValidationOwnerKind::External,
                owner_path: object_path.to_path_buf(),
                language_codes: Vec::new(),
            },
        );
    }

    let candidate = match resolve_configuration_owner_candidate(object_path, workspace) {
        Ok(candidate) => candidate,
        Err(error) => {
            if let Some(path) = error.attempted_path {
                stable_push(&mut paths, path);
            }
            let message = if error.message == "Configuration.xml owner not found" {
                format!(
                    "Configuration.xml owner not found for {}.{}",
                    identity.object_type, identity.object_name
                )
            } else {
                error.message
            };
            return inspection_error(paths, message);
        }
    };
    stable_push(&mut paths, candidate.path.clone());
    let owner = match read_configuration_owner(candidate.path, candidate.kind) {
        Ok(owner) => owner,
        Err(error) => return inspection_error(paths, error),
    };
    if !owner.registrations.iter().any(|(object_type, object_name)| {
        object_type == &identity.object_type && object_name == &identity.object_name
    }) {
        return inspection_error(
            paths,
            format!(
                "{}.{} is not registered in {}",
                identity.object_type,
                identity.object_name,
                owner.path.display()
            ),
        );
    }
    inspection_ok(
        paths,
        MetaValidationOwnerContext {
            object_type: identity.object_type,
            object_name: identity.object_name,
            owner_kind: owner.kind,
            owner_path: owner.path,
            language_codes: Vec::new(),
        },
    )
}
```

`read_object_identity` requires `MetaDataObject` plus exactly one recognized
descriptor child and reads its `Properties/Name`.
`read_configuration_owner` requires a `Configuration` descriptor, classifies
an extension from the PR 188 source-set kind or, for an ancestry candidate,
from `Properties/ConfigurationExtensionPurpose`, and extracts `ChildObjects`
registrations and `Language` names in declaration order.

Use PR 188 source-set ownership first. Convert its selected
configuration/extension owner to `OwnerCandidate`, and preserve
`PlatformXmlOwnerError.path` as `OwnerCandidateError.attempted_path`, including
a malformed or configured-but-missing `Configuration.xml`. When no source-set
owns the object, search from the object directory through
`workspace.workspace_root` inclusive and select the nearest existing ancestor
`Configuration.xml` as the candidate; never inspect ancestors outside the
workspace. Append the candidate path before `read_configuration_owner`.
Require its `ChildObjects` to register the exact type/name pair. A registration
mismatch is an error; do not continue to a higher, unrelated configuration.

Implement the candidate selection as:

```rust
fn resolve_configuration_owner_candidate(
    object_path: &Path,
    workspace: &WorkspaceContext,
) -> Result<OwnerCandidate, OwnerCandidateError> {
    match resolve_platform_xml_owners(object_path, workspace) {
        Ok(owners) => {
            if let Some(owner) = owners.into_iter().find(|owner| {
                matches!(
                    owner.kind,
                    PlatformXmlOwnerKind::Configuration
                        | PlatformXmlOwnerKind::Extension
                )
            }) {
                let kind = match owner.kind {
                    PlatformXmlOwnerKind::Extension => {
                        MetaValidationOwnerKind::Extension
                    }
                    PlatformXmlOwnerKind::Configuration => {
                        MetaValidationOwnerKind::Configuration
                    }
                    _ => unreachable!("filtered above"),
                };
                return Ok(OwnerCandidate {
                    kind,
                    path: owner.path,
                });
            }
        }
        Err(error) => {
            return Err(OwnerCandidateError {
                attempted_path: Some(error.path),
                message: error.message,
            });
        }
    }

    let workspace_root = &workspace.workspace_root;
    let mut directory = object_path.parent();
    while let Some(current) = directory {
        if !current.starts_with(workspace_root) {
            break;
        }
        let candidate = current.join("Configuration.xml");
        if candidate.is_file() {
            return match resolve_platform_xml_owners(&candidate, workspace) {
                Ok(owners) => {
                    let owner = owners
                        .into_iter()
                        .find(|owner| {
                            matches!(
                                owner.kind,
                                PlatformXmlOwnerKind::Configuration
                                    | PlatformXmlOwnerKind::Extension
                            )
                        })
                        .ok_or_else(|| OwnerCandidateError {
                            attempted_path: Some(candidate.clone()),
                            message: format!(
                                "{} is not a configuration owner",
                                candidate.display()
                            ),
                        })?;
                    let kind = if owner.kind == PlatformXmlOwnerKind::Extension {
                        MetaValidationOwnerKind::Extension
                    } else {
                        MetaValidationOwnerKind::Configuration
                    };
                    Ok(OwnerCandidate {
                        kind,
                        path: owner.path,
                    })
                }
                Err(error) => Err(OwnerCandidateError {
                    attempted_path: Some(error.path),
                    message: error.message,
                }),
            };
        }
        if current == workspace_root {
            break;
        }
        directory = current.parent();
    }

    Err(OwnerCandidateError {
        attempted_path: None,
        message: "Configuration.xml owner not found".to_string(),
    })
}
```

Parse object and owner descriptors with complete helpers:

```rust
fn read_object_identity(path: &Path) -> Result<ObjectIdentity, String> {
    let text = read_utf8_sig(path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    let document = Document::parse(text.trim_start_matches('\u{feff}'))
        .map_err(|error| format!("failed to parse {}: {error}", path.display()))?;
    let root = document.root_element();
    if root.tag_name().namespace() != Some(MD_CLASSES_NS)
        || root.tag_name().name() != "MetaDataObject"
    {
        return Err(format!(
            "{} is not an MDClasses MetaDataObject",
            path.display()
        ));
    }
    let artifacts = root
        .children()
        .filter(|node| {
            node.is_element()
                && node.tag_name().namespace() == Some(MD_CLASSES_NS)
        })
        .collect::<Vec<_>>();
    let [artifact] = artifacts.as_slice() else {
        return Err(format!(
            "{} must contain exactly one metadata descriptor",
            path.display()
        ));
    };
    let object_type = artifact.tag_name().name();
    if !meta_validate_valid_types().contains(&object_type) {
        return Err(format!("unrecognized metadata type: {object_type}"));
    }
    let object_name = meta_info_child(*artifact, "Properties")
        .and_then(|properties| meta_info_child(properties, "Name"))
        .map(meta_info_inner_text)
        .filter(|name| !name.is_empty())
        .ok_or_else(|| format!("{object_type} Name is missing in {}", path.display()))?;
    let properties = meta_info_child(*artifact, "Properties");
    let reads_registrars = matches!(
        object_type,
        "AccumulationRegister" | "AccountingRegister" | "CalculationRegister"
    ) || (object_type == "InformationRegister"
        && properties
            .and_then(|properties| meta_info_child(properties, "WriteMode"))
            .map(meta_info_inner_text)
            .as_deref()
            == Some("RecorderSubordinate"));
    let registrar_reference =
        reads_registrars.then(|| format!("{object_type}.{object_name}"));
    Ok(ObjectIdentity {
        object_type: object_type.to_string(),
        object_name,
        registrar_reference,
    })
}

fn read_configuration_owner(
    path: PathBuf,
    candidate_kind: MetaValidationOwnerKind,
) -> Result<ConfigurationOwner, String> {
    let text = read_utf8_sig(&path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    let document = Document::parse(text.trim_start_matches('\u{feff}'))
        .map_err(|error| format!("failed to parse {}: {error}", path.display()))?;
    let root = document.root_element();
    if root.tag_name().namespace() != Some(MD_CLASSES_NS)
        || root.tag_name().name() != "MetaDataObject"
    {
        return Err(format!(
            "{} is not an MDClasses MetaDataObject",
            path.display()
        ));
    }
    let artifacts = root
        .children()
        .filter(|node| {
            node.is_element()
                && node.tag_name().namespace() == Some(MD_CLASSES_NS)
        })
        .collect::<Vec<_>>();
    let [configuration] = artifacts.as_slice() else {
        return Err(format!(
            "{} must contain exactly one Configuration descriptor",
            path.display()
        ));
    };
    if configuration.tag_name().name() != "Configuration" {
        return Err(format!(
            "{} does not contain Configuration",
            path.display()
        ));
    }
    let properties = meta_info_child(*configuration, "Properties");
    let is_extension = properties.is_some_and(|properties| {
        meta_info_child(properties, "ConfigurationExtensionPurpose").is_some()
    });
    let kind = if candidate_kind == MetaValidationOwnerKind::Extension || is_extension {
        MetaValidationOwnerKind::Extension
    } else {
        MetaValidationOwnerKind::Configuration
    };
    let mut registrations = Vec::new();
    let mut registered_languages = Vec::new();
    if let Some(children) = meta_info_child(*configuration, "ChildObjects") {
        for child in children.children().filter(roxmltree::Node::is_element) {
            if child.tag_name().namespace() != Some(MD_CLASSES_NS) {
                continue;
            }
            let object_type = child.tag_name().name();
            let object_name = meta_info_inner_text(child).trim().to_string();
            if object_name.is_empty() {
                continue;
            }
            if object_type == "Language" {
                registered_languages.push(object_name);
            } else {
                registrations.push((object_type.to_string(), object_name));
            }
        }
    }
    Ok(ConfigurationOwner {
        kind,
        path,
        registrations,
        registered_languages,
    })
}
```

Define the result constructors directly so every error retains the paths read
before failure:

```rust
fn inspection_ok(
    paths: Vec<PathBuf>,
    context: MetaValidationOwnerContext,
) -> MetaValidationReadInspection {
    MetaValidationReadInspection {
        paths,
        context: Ok(context),
    }
}

fn inspection_error(
    paths: Vec<PathBuf>,
    error: impl Into<String>,
) -> MetaValidationReadInspection {
    MetaValidationReadInspection {
        paths,
        context: Err(error.into()),
    }
}
```

- [ ] **Step 5: Resolve a complete registered language profile**

Replace the final `inspection_ok` in Step 4 with the following. For types in
`meta_validate_types_with_list_presentation`, append registered language paths
before reading them. Use `match`, not `?`, so an attempted missing/malformed
language path remains in the returned inspection:

```rust
let mut language_codes = Vec::new();
let mut seen_codes = HashSet::new();
if meta_validate_types_with_list_presentation()
    .contains(&identity.object_type.as_str())
{
    for language_name in &owner.registered_languages {
        let language_path = owner
            .path
            .parent()
            .expect("Configuration.xml has a parent")
            .join("Languages")
            .join(format!("{language_name}.xml"));
        stable_push(&mut paths, language_path.clone());
        let code = match read_required_language_code(&language_path) {
            Ok(code) => code,
            Err(error) => return inspection_error(paths, error),
        };
        if seen_codes.insert(code.clone()) {
            language_codes.push(code);
        }
    }
    if language_codes.is_empty() {
        return inspection_error(
            paths,
            format!(
                "{} has no registered language profile",
                owner.path.display()
            ),
        );
    }
}

inspection_ok(
    paths,
    MetaValidationOwnerContext {
        object_type: identity.object_type,
        object_name: identity.object_name,
        owner_kind: owner.kind,
        owner_path: owner.path,
        language_codes,
    },
)
```

`read_required_language_code` returns a path-specific error for missing files,
malformed XML, a non-`Language` artifact, or empty `LanguageCode`. Do not return
an empty list and do not inspect object-local `v8:lang` as a fallback.

```rust
fn read_required_language_code(path: &Path) -> Result<String, String> {
    if !path.is_file() {
        return Err(format!(
            "registered language file not found: {}",
            path.display()
        ));
    }
    let text = read_utf8_sig(path)
        .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    let document = Document::parse(text.trim_start_matches('\u{feff}'))
        .map_err(|error| format!("failed to parse {}: {error}", path.display()))?;
    let root = document.root_element();
    if root.tag_name().namespace() != Some(MD_CLASSES_NS)
        || root.tag_name().name() != "MetaDataObject"
    {
        return Err(format!(
            "registered language descriptor is not Language: {}",
            path.display()
        ));
    }
    let artifacts = root
        .children()
        .filter(|node| {
            node.is_element()
                && node.tag_name().namespace() == Some(MD_CLASSES_NS)
        })
        .collect::<Vec<_>>();
    let [language] = artifacts.as_slice() else {
        return Err(format!(
            "registered language descriptor is not Language: {}",
            path.display()
        ));
    };
    if language.tag_name().name() != "Language" {
        return Err(format!(
            "registered language descriptor is not Language: {}",
            path.display()
        ));
    }
    let code = meta_info_child(*language, "Properties")
        .and_then(|properties| meta_info_child(properties, "LanguageCode"))
        .map(meta_info_inner_text)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("empty LanguageCode in {}", path.display()))?;
    Ok(code)
}
```

Use these canonical diagnostics in Rust and Python:

| State | Diagnostic |
|---|---|
| no owner | `Configuration.xml owner not found for <Type>.<Name>` |
| registration mismatch | `<Type>.<Name> is not registered in <path>` |
| no languages | `<path> has no registered language profile` |
| missing language file | `registered language file not found: <path>` |
| wrong language descriptor | `registered language descriptor is not Language: <path>` |
| empty language code | `empty LanguageCode in <path>` |

Malformed XML diagnostics start with `failed to parse <path>:`; parser-specific
tails may differ and are asserted only by prefix in unit tests.

- [ ] **Step 6: Compose registrar reads and stable deduplication**

Move or call the existing sorted registrar-prefix scan from the context module.
Insert its block immediately before Step 5's final `inspection_ok`, so document
paths follow owner/language paths. Use stable insertion:

```rust
fn stable_push(paths: &mut Vec<PathBuf>, candidate: PathBuf) {
    if !paths.contains(&candidate) {
        paths.push(candidate);
    }
}

if let Some(register_reference) = &identity.registrar_reference {
    let documents_dir = owner
        .path
        .parent()
        .expect("Configuration.xml has a parent")
        .join("Documents");
    if documents_dir.is_dir() {
        let registrar_paths = match meta_validate_registrar_document_scan(
                &documents_dir,
                register_reference,
            ) {
            Ok((registrar_paths, _)) => registrar_paths,
            Err(error) => return inspection_error(paths, error),
        };
        for registrar_path in registrar_paths {
            stable_push(&mut paths, registrar_path);
        }
    }
}

```

Move `meta_validate_registrar_document_scan` from `meta.rs` into the context
module unchanged: it sorts directory entries by file name, records every XML
file read, and stops after the first content match. Re-export it
`pub(crate)` for the existing semantic cross-property check. Do not call
`sort()` on the final combined list.

- [ ] **Step 7: Wire the inspection into guard and handler**

Replace `meta_validate_format_dependency_paths` with:

```rust
pub(crate) fn meta_validate_format_dependency_paths(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<Vec<PathBuf>, String> {
    let raw_path = required_path(
        args,
        &["objectPath", "ObjectPath", "path", "Path"],
        "ObjectPath",
    )?;
    let raw_path_text = raw_path.to_string_lossy();
    let mut dependencies = Vec::new();
    for raw in raw_path_text
        .split('|')
        .map(str::trim)
        .filter(|path| !path.is_empty())
    {
        let candidate = absolutize(PathBuf::from(raw), &context.cwd);
        let object_path =
            resolve_meta_info_path(candidate.clone()).unwrap_or(candidate);
        let inspection = inspect_meta_validation_reads(&object_path, context);
        for path in inspection.paths {
            if !dependencies.contains(&path) {
                dependencies.push(path);
            }
        }
    }
    Ok(dependencies)
}
```

Inside `meta_validate_one`, parse and identify the object first, set
`report.md_type` and `report.obj_name`, then consume `inspection.context`
before emitting the successful root-structure line or running checks 2+.
Create the inspection immediately after `resolved_path`:

```rust
let inspection = inspect_meta_validation_reads(&resolved_path, context);
```

Keep the existing object parse/root/type/name checks between that line and the
context match below. For a context error, the match reports the canonical
message and finishes the normal validation report.

Replace the `PublicOwnerAware` reference-input branch from Task 2 with:

```rust
let owner_context = match inspection.context {
    Ok(owner_context) => owner_context,
    Err(error) => {
        report.error(format!("1. Owner context: {error}"));
        return meta_validate_finish(
            report,
            options.out_file.clone(),
            options.out_file_label.clone(),
            resolved_path,
        );
    }
};
let config_dir = match owner_context.owner_kind {
    MetaValidationOwnerKind::Configuration | MetaValidationOwnerKind::Extension => {
        owner_context.owner_path.parent().map(Path::to_path_buf)
    }
    MetaValidationOwnerKind::External => None,
};
let reference_inputs = MetaValidationReferenceInputs {
    config_dir,
    language_codes: owner_context.language_codes,
};
```

The `PostWriteLocal` branch remains `config_dir: None` plus an empty language
list. Pass `reference_inputs.language_codes` into property validation and
`reference_inputs.config_dir` into cross-property/method-reference checks.
Remove `meta_validate_config_dir` and `meta_validate_language_codes` after all
callers use the shared inspection; registrar scanning derives its `Documents`
directory from the resolved owner path, not from a second ancestry walk.

- [ ] **Step 8: Add format-version and missing-profile regressions**

Add:

```rust
#[test]
fn meta_validate_warns_for_newer_owner_it_reads() {
    // Object is 2.20; its registered Configuration.xml is 2.21.
    let check =
        evaluate_format_guard(spec("unica.meta.validate"), &args, &context(&root)).unwrap();
    let FormatGuardCheck::Warn { diagnostic, .. } = check else {
        panic!("metadata owner must participate in format preflight");
    };
    assert_eq!(diagnostic["actualFormat"], "2.21");
    assert_eq!(
        normalized_path(Path::new(diagnostic["root"].as_str().unwrap())),
        normalized_path(&configuration)
    );
}

#[test]
fn meta_validate_warns_for_newer_registered_language_it_reads() {
    // Object and Configuration.xml are 2.20; registered English.xml is 2.21.
    let check =
        evaluate_format_guard(spec("unica.meta.validate"), &args, &context(&root)).unwrap();
    let FormatGuardCheck::Warn { diagnostic, .. } = check else {
        panic!("registered language must participate in format preflight");
    };
    assert_eq!(diagnostic["actualFormat"], "2.21");
    assert_eq!(
        normalized_path(Path::new(diagnostic["root"].as_str().unwrap())),
        normalized_path(&english)
    );
}
```

Also assert that an unregistered `Languages/Unused.xml` and document files
after the registrar match are absent from `effective_format_paths`.

Add the following semantic tests with one assertion per failure:

- `validate_meta_rejects_list_type_without_registered_languages`: register an
  `Enum` but no `Language`; assert `outcome.ok == false` and
  `"has no registered language profile"`.
- `meta_validation_reads_missing_registered_language_before_reporting_error`:
  register `Language/Russian` without creating `Languages/Russian.xml`; assert
  the missing path is the third inspection path and the context error names
  that path.
- `validate_meta_rejects_malformed_registered_language`: write malformed
  `Russian.xml`; assert the error names the file and XML parse failure.
- `validate_meta_rejects_empty_registered_language_code`: write a valid
  `Language` descriptor with `<LanguageCode/>`; assert the error contains
  `"empty LanguageCode"` and the path.
- `meta_validation_deduplicates_language_codes_in_registration_order`: register
  `RussianOne(ru)`, `English(en)`, `RussianTwo(ru)`; assert the context codes
  are exactly `["ru", "en"]`, while all three language files remain in
  `inspection.paths` in registration order.
- `meta_validate_batch_read_set_stably_deduplicates_shared_owner`: validate two
  registered Enum paths in caller order against one owner and one language;
  assert the full dependency vector is exactly
  `[first_object, Configuration.xml, Russian.xml, second_object]`.

- [ ] **Step 9: Run the context and format-guard tests**

```bash
cargo test -p unica-coder meta_validation_context -- --test-threads=1
cargo test -p unica-coder validate_meta_rejects_ -- --test-threads=1
cargo test -p unica-coder format_guard::tests::meta_validate_ -- --test-threads=1
```

Expected: all tests pass with zero failures.

- [ ] **Step 10: Commit the owner/read inspection**

```bash
git add \
  crates/unica-coder/src/infrastructure/native_operations.rs \
  crates/unica-coder/src/infrastructure/native_operations/meta.rs \
  crates/unica-coder/src/infrastructure/native_operations/meta_validation_context.rs \
  crates/unica-coder/src/infrastructure/format_guard.rs
git -c commit.gpgsign=false commit -m "Добавить точный owner-aware read-set"
```

---

### Task 4: Restrict And Simplify Command-Text Validation

**Files:**
- Modify: `crates/unica-coder/src/infrastructure/native_operations/meta.rs`
- Test: unit tests in the same file

**Interfaces:**
- Consumes: `meta_validate_types_with_list_presentation`.
- Changes: `meta_validate_check_properties(report, md_type, props_node, name_node, obj_name, language_codes)`.
- Removes: `meta_validate_observed_language_codes`.
- Produces: one per-language `ListPresentation -> Synonym` selection path.

- [ ] **Step 1: Add failing applicability and precedence tests**

First update the existing validation fixtures so success tests obey the owner
contract introduced in Task 3:

- add `version="2.20"` to `write_language_fixture`;
- make `validate_stdout_with_synonym` always write a `Configuration.xml` that
  registers `Document.SampleShipment` plus `Language.Русский`, and write
  `Languages/Русский.xml` with code `ru`;
- make `validate_stdout_with_presentations` always write
  `<Document>SampleShipment</Document>` after its declared `Language` entries;
- require `configured_languages` to be non-empty in
  `validate_stdout_with_presentations`; the only empty-language caller is the
  obsolete neutral-fallback test removed in Step 4.

Use this exact `ChildObjects` construction in both helpers:

```rust
// validate_stdout_with_synonym defines:
let configured_languages = [("Русский", "ru")];

let language_children = configured_languages
    .iter()
    .map(|(name, _)| format!("<Language>{name}</Language>"))
    .collect::<String>();
let child_objects = format!(
    "{language_children}<Document>SampleShipment</Document>"
);
```

Add two fixture helpers with exact ownership semantics:

```rust
fn validate_registered_object(
    object_type: &str,
    object_name: &str,
    object_xml: &str,
    languages: &[(&str, &str)],
) -> AdapterOutcome {
    let context = temp_context(&format!("registered-{object_type}-{object_name}"));
    let language_names = languages.iter().map(|(name, _)| *name).collect::<Vec<_>>();
    let src = write_owner(
        &context.cwd.join("src"),
        object_type,
        object_name,
        &language_names,
    );
    for (name, code) in languages {
        write_file(
            &src.join("Languages").join(format!("{name}.xml")),
            &sample_language_named(name, code),
        );
    }
    let object = src
        .join(format!("{object_type}s"))
        .join(format!("{object_name}.xml"));
    write_file(&object, object_xml);
    validate_meta(&meta_validate_args(&object), &context)
}

fn outcome_text(outcome: &AdapterOutcome) -> String {
    format!(
        "{}\n{}\n{}",
        outcome.stdout.clone().unwrap_or_default(),
        outcome.warnings.join("\n"),
        outcome.errors.join("\n")
    )
}

fn localized_property(name: &str, values: &[(&str, &str)]) -> String {
    let items = values
        .iter()
        .map(|(language, content)| {
            format!(
                "<v8:item><v8:lang>{language}</v8:lang>\
                 <v8:content>{content}</v8:content></v8:item>"
            )
        })
        .collect::<String>();
    format!("<{name}>{items}</{name}>")
}

fn sample_language_named(name: &str, code: &str) -> String {
    format!(
        r#"<MetaDataObject xmlns="{TEST_MD_NS}" version="2.20">
<Language uuid="22222222-2222-4222-8222-222222222222">
<Properties><Name>{name}</Name><Synonym/><Comment/>
<LanguageCode>{code}</LanguageCode></Properties>
</Language></MetaDataObject>"#
    )
}

fn sample_common_module_named(name: &str, synonyms: &[(&str, &str)]) -> String {
    let synonym = localized_property("Synonym", synonyms);
    format!(
        r#"<MetaDataObject xmlns="{TEST_MD_NS}" xmlns:v8="{TEST_V8_NS}" version="2.20">
<CommonModule uuid="33333333-3333-4333-8333-333333333333">
<Properties><Name>{name}</Name>{synonym}<Comment/></Properties>
<ChildObjects/>
</CommonModule></MetaDataObject>"#
    )
}

fn sample_enum_with_presentations(
    name: &str,
    synonyms: &[(&str, &str)],
    list_presentations: &[(&str, &str)],
) -> String {
    let synonym = localized_property("Synonym", synonyms);
    let list_presentation =
        localized_property("ListPresentation", list_presentations);
    format!(
        r#"<MetaDataObject xmlns="{TEST_MD_NS}" xmlns:v8="{TEST_V8_NS}"
 xmlns:xr="{TEST_XR_NS}" version="2.20">
<Enum uuid="44444444-4444-4444-8444-444444444444">
<InternalInfo>
<xr:GeneratedType name="EnumRef.{name}" category="Ref">
<xr:TypeId>55555555-5555-4555-8555-555555555551</xr:TypeId>
<xr:ValueId>55555555-5555-4555-8555-555555555552</xr:ValueId>
</xr:GeneratedType>
<xr:GeneratedType name="EnumManager.{name}" category="Manager">
<xr:TypeId>55555555-5555-4555-8555-555555555553</xr:TypeId>
<xr:ValueId>55555555-5555-4555-8555-555555555554</xr:ValueId>
</xr:GeneratedType>
<xr:GeneratedType name="EnumList.{name}" category="List">
<xr:TypeId>55555555-5555-4555-8555-555555555555</xr:TypeId>
<xr:ValueId>55555555-5555-4555-8555-555555555556</xr:ValueId>
</xr:GeneratedType>
</InternalInfo>
<Properties><Name>{name}</Name>{synonym}<Comment/>
{list_presentation}</Properties>
<ChildObjects/>
</Enum></MetaDataObject>"#
    )
}
```

Add:

```rust
#[test]
fn validate_meta_does_not_apply_list_command_limit_to_common_module() {
    let outcome = validate_registered_object(
        "CommonModule",
        "LongModule",
        &sample_common_module_named(
            "LongModule",
            &[(
                "ru",
                "Очень длинный синоним общего модуля для проверки ограничения",
            )],
        ),
        &[],
    );
    let stdout = outcome_text(&outcome);
    assert!(outcome.ok, "{outcome:?}");
    assert!(!stdout.contains("longer than 38 characters"), "{stdout}");
}

#[test]
fn validate_meta_prefers_list_presentation_per_registered_language() {
    let outcome = validate_registered_object(
        "Enum",
        "Status",
        &sample_enum_with_presentations(
            "Status",
            &[
            ("ru", "Очень длинный синоним для командного интерфейса перечисления"),
            ("en", "Status"),
            ],
            &[("ru", "Статусы")],
        ),
        &[("Русский", "ru"), ("English", "en")],
    );
    let stdout = outcome_text(&outcome);
    assert!(outcome.ok, "{outcome:?}");
    assert!(!stdout.contains("language 'ru'"), "{stdout}");
}

#[test]
fn validate_meta_uses_synonym_when_registered_language_has_no_list_presentation() {
    let outcome = validate_registered_object(
        "Enum",
        "Status",
        &sample_enum_with_presentations(
            "Status",
            &[(
                "en",
                "A very long status title intended for the command interface",
            )],
            &[("ru", "Статусы")],
        ),
        &[("Русский", "ru"), ("English", "en")],
    );
    let stdout = outcome_text(&outcome);
    assert!(outcome.ok, "{outcome:?}");
    assert!(stdout.contains("Synonym"), "{stdout}");
    assert!(stdout.contains("language 'en'"), "{stdout}");
}

#[test]
fn validate_meta_skips_missing_or_empty_text_for_registered_language() {
    let outcome = validate_registered_object(
        "Enum",
        "Status",
        &sample_enum_with_presentations(
            "Status",
            &[("ru", "Статус"), ("en", "")],
            &[("ru", "Статусы")],
        ),
        &[("Русский", "ru"), ("English", "en")],
    );
    let stdout = outcome_text(&outcome);
    assert!(outcome.ok, "{outcome:?}");
    assert!(!stdout.contains("language 'en'"), "{stdout}");
}
```

Keep these fixtures at export format `2.20`; do not reuse the invalid
`CommonModule.ListPresentation` shape.

- [ ] **Step 2: Run the tests and verify RED**

```bash
cargo test -p unica-coder validate_meta_does_not_apply_list_command_limit_to_common_module -- --nocapture
cargo test -p unica-coder validate_meta_prefers_list_presentation_per_registered_language -- --nocapture
```

Expected: the CommonModule test fails because the current implementation
applies the rule to every accepted type.

- [ ] **Step 3: Gate the rule by platform capability**

Pass `md_type` into `meta_validate_check_properties` and isolate the
command-text block:

```rust
if meta_validate_types_with_list_presentation().contains(&md_type) {
    meta_validate_check_command_texts(
        report,
        props_node,
        configured_language_codes,
    );
}
```

Keep the `Name` and `Synonym present` summary checks for every metadata type.

- [ ] **Step 4: Keep one deterministic selection algorithm**

Implement:

```rust
fn meta_validate_check_command_texts(
    report: &mut MetaValidationReporter,
    props_node: roxmltree::Node<'_, '_>,
    language_codes: &[String],
) {
    let synonyms =
        meta_validate_localized_values(meta_info_child(props_node, "Synonym"));
    let lists =
        meta_validate_localized_values(meta_info_child(props_node, "ListPresentation"));

    for language_code in language_codes {
        let list_values = lists
            .iter()
            .filter(|(language, text)| {
                language.as_deref() == Some(language_code.as_str())
                    && !text.trim().is_empty()
            })
            .collect::<Vec<_>>();
        let selected = if list_values.is_empty() {
            synonyms
                .iter()
                .filter(|(language, text)| {
                    language.as_deref() == Some(language_code.as_str())
                        && !text.trim().is_empty()
                })
                .map(|(_, text)| ("Synonym", text))
                .collect::<Vec<_>>()
        } else {
            list_values
                .into_iter()
                .map(|(_, text)| ("ListPresentation", text))
                .collect::<Vec<_>>()
        };
        for (source, text) in selected {
            meta_validate_warn_long_command_text(
                report,
                source,
                text,
                Some(language_code),
            );
        }
    }
}
```

Delete the observed-language and language-neutral branches and their old test.
Specifically:

- delete `meta_validate_observed_language_codes`;
- delete
  `validate_meta_observes_languages_from_sibling_localized_properties`;
- delete `validate_meta_checks_all_language_neutral_presentations`;
- keep `validate_meta_ignores_non_v8_language_elements`, but remove only its
  assertion that calls the deleted observed-language helper.

- [ ] **Step 5: Run native metadata validation tests**

```bash
cargo test -p unica-coder validate_meta_ -- --test-threads=1
cargo test -p unica-coder infrastructure::native_operations::meta -- --test-threads=1
```

Expected: all selected tests pass.

- [ ] **Step 6: Commit the type-aware command rule**

```bash
git add crates/unica-coder/src/infrastructure/native_operations/meta.rs
git -c commit.gpgsign=false commit -m "Ограничить проверку типами с представлением списка"
```

---

### Task 5: Mirror The Contract In Python And Replace The Fixture

**Files:**
- Modify: `tests/fixtures/unica_mcp_script_parity/reference_skills/meta-validate/scripts/meta-validate.py`
- Modify: `tests/ci/test_unica_mcp_script_parity.py`
- Modify: `tests/fixtures/unica_mcp_script_parity/meta-validate-language-aware/Configuration.xml`
- Modify: `tests/fixtures/unica_mcp_script_parity/meta-validate-language-aware/Languages/Русский.xml`
- Modify: `tests/fixtures/unica_mcp_script_parity/meta-validate-language-aware/Languages/English.xml`
- Delete: `tests/fixtures/unica_mcp_script_parity/meta-validate-language-aware/CommonModules/LanguageAware.xml`
- Create: `tests/fixtures/unica_mcp_script_parity/meta-validate-language-aware/Enums/LanguageAware.xml`
- Reference: `tests/fixtures/unica_mcp_script_parity/bsp/meta/Enums/ВажностьПроблемыУчета.xml`
- Reference: `tests/fixtures/unica_mcp_script_parity/cf-validate/Configuration.xml`
- Create: `tests/fixtures/unica_mcp_script_parity/meta-validate-parity-owner/Configuration.xml`
- Create: `tests/fixtures/unica_mcp_script_parity/meta-validate-parity-owner/Languages/Русский.xml`
- Create: `tests/fixtures/unica_mcp_script_parity/bsp/meta/Languages/Русский.xml`

**Interfaces:**
- Consumes: Rust warning/error contract from Tasks 3–4.
- Produces: Python `resolve_owner_context(resolved_path, md_type, obj_name)`.
- Produces: Python `LIST_PRESENTATION_TYPES`.

- [ ] **Step 1: Move the parity scenario to a platform-valid type**

Change the scenario argument and fixture mapping:

```python
arguments={
    "ObjectPath": "src/Enums/LanguageAware.xml",
    "Detailed": True,
},
fixtures=(
    FileFixture(
        "meta-validate-language-aware/Configuration.xml",
        "src/Configuration.xml",
    ),
    FileFixture(
        "meta-validate-language-aware/Languages/Русский.xml",
        "src/Languages/Русский.xml",
    ),
    FileFixture(
        "meta-validate-language-aware/Languages/English.xml",
        "src/Languages/English.xml",
    ),
    FileFixture(
        "meta-validate-language-aware/Enums/LanguageAware.xml",
        "src/Enums/LanguageAware.xml",
    ),
),
```

Add `<Enum>LanguageAware</Enum>` to `Configuration/ChildObjects` and add
`version="2.20"` to every root descriptor.

Define reusable fixture tuples immediately after the `FileFixture` dataclass in
`test_unica_mcp_script_parity.py`:

```python
META_VALIDATE_COMPILED_OWNER_FIXTURES = (
    FileFixture(
        "meta-validate-parity-owner/Configuration.xml",
        "src/Configuration.xml",
    ),
    FileFixture(
        "meta-validate-parity-owner/Languages/Русский.xml",
        "src/Languages/Русский.xml",
    ),
)

BSP_META_VALIDATE_OWNER_FIXTURES = (
    FileFixture(BSP_CF_CONFIGURATION_FIXTURE, "src/Configuration.xml"),
    FileFixture(
        "bsp/meta/Languages/Русский.xml",
        "src/Languages/Русский.xml",
    ),
)
```

Prepend `META_VALIDATE_COMPILED_OWNER_FIXTURES` to
`meta-validate-catalog-detailed-outfile.fixtures`; the existing meta-compile
setup then registers `Catalog.ParityCatalog` into that owner. Prepend
`BSP_META_VALIDATE_OWNER_FIXTURES` to the fixtures of all six existing BSP
`meta-validate` scenarios: Catalog, Document, Report, CommonModule, Enum, and
InformationRegister. Do not add the owner tuple to neighboring `meta.info`
scenarios, because they do not read it.

Create the meta-compile owner fixture as an exact copy of
`tests/fixtures/unica_mcp_script_parity/cf-validate/Configuration.xml`. It is a
known-valid `2.17` owner and already registers `Language.Русский`; the existing
meta-compile setup adds `Catalog.ParityCatalog` to its `ChildObjects`.

Create both new Russian language descriptors with this shape, using
`version="2.17"` and UUID `77777777-7777-4777-8777-777777777777` for the
meta-compile owner, and `version="2.21"` plus UUID
`88888888-8888-4888-8888-888888888888` for BSP:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.17">
  <Language uuid="77777777-7777-4777-8777-777777777777">
    <Properties>
      <Name>Русский</Name>
      <Synonym/>
      <Comment/>
      <LanguageCode>ru</LanguageCode>
    </Properties>
  </Language>
</MetaDataObject>
```

Add two `VALIDATION_FAILURE_SCENARIOS` using the same rewritten Enum:

```python
ParityScenario(
    name="meta-validate-missing-owner",
    tool="unica.meta.validate",
    skill="meta-validate",
    script="meta-validate.py",
    arguments={"ObjectPath": "src/Enums/LanguageAware.xml", "Detailed": True},
    fixtures=(
        FileFixture(
            "meta-validate-language-aware/Enums/LanguageAware.xml",
            "src/Enums/LanguageAware.xml",
        ),
    ),
    expect_ok=False,
),
ParityScenario(
    name="meta-validate-missing-registered-language",
    tool="unica.meta.validate",
    skill="meta-validate",
    script="meta-validate.py",
    arguments={"ObjectPath": "src/Enums/LanguageAware.xml", "Detailed": True},
    fixtures=(
        FileFixture(
            "meta-validate-language-aware/Configuration.xml",
            "src/Configuration.xml",
        ),
        FileFixture(
            "meta-validate-language-aware/Languages/Русский.xml",
            "src/Languages/Русский.xml",
        ),
        FileFixture(
            "meta-validate-language-aware/Enums/LanguageAware.xml",
            "src/Enums/LanguageAware.xml",
        ),
    ),
    expect_ok=False,
),
```

- [ ] **Step 2: Create the valid Enum descriptor**

Adapt the tracked Designer-exported Enum
`tests/fixtures/unica_mcp_script_parity/bsp/meta/Enums/ВажностьПроблемыУчета.xml`
rather than inventing a minimal object shape. Preserve its namespace set,
property order, standard attributes, characteristics, and other platform
fields. Make these exact substitutions:

- root format `2.21` -> `2.20`;
- object UUID -> `44444444-4444-4444-8444-444444444444`;
- all generated-type names -> `EnumRef.LanguageAware`,
  `EnumManager.LanguageAware`, and `EnumList.LanguageAware`;
- generated IDs -> the six `5555...551` through `5555...556` values below;
- `Name` -> `LanguageAware`;
- `Synonym` and `ListPresentation` -> the localized values below;
- `Comment` -> empty;
- remove the sample `EnumValue` and leave `<ChildObjects/>`.

The rewritten key sections are:

```xml
<Enum uuid="44444444-4444-4444-8444-444444444444">
  <InternalInfo>
    <xr:GeneratedType name="EnumRef.LanguageAware" category="Ref">
      <xr:TypeId>55555555-5555-4555-8555-555555555551</xr:TypeId>
      <xr:ValueId>55555555-5555-4555-8555-555555555552</xr:ValueId>
    </xr:GeneratedType>
    <xr:GeneratedType name="EnumManager.LanguageAware" category="Manager">
      <xr:TypeId>55555555-5555-4555-8555-555555555553</xr:TypeId>
      <xr:ValueId>55555555-5555-4555-8555-555555555554</xr:ValueId>
    </xr:GeneratedType>
    <xr:GeneratedType name="EnumList.LanguageAware" category="List">
      <xr:TypeId>55555555-5555-4555-8555-555555555555</xr:TypeId>
      <xr:ValueId>55555555-5555-4555-8555-555555555556</xr:ValueId>
    </xr:GeneratedType>
  </InternalInfo>
  <Properties>
    <Name>LanguageAware</Name>
    <Synonym>
      <v8:item>
        <v8:lang>ru</v8:lang>
        <v8:content>Языковое перечисление</v8:content>
      </v8:item>
      <v8:item>
        <v8:lang>en</v8:lang>
        <v8:content>A very long enum title intended for the command interface</v8:content>
      </v8:item>
    </Synonym>
    <Comment/>
    <ListPresentation>
      <v8:item>
        <v8:lang>ru</v8:lang>
        <v8:content>Перечисления</v8:content>
      </v8:item>
    </ListPresentation>
  </Properties>
  <ChildObjects/>
</Enum>
```

Include the `xr` and `v8` namespace declarations used by those elements.

- [ ] **Step 3: Run parity and verify RED**

```bash
python3.12 tests/ci/test_unica_mcp_script_parity.py -k meta_validate_language_aware
```

Expected: FAIL because the native validator now requires owner registration and
uses the type gate while the Python oracle still uses silent/observed fallback.

- [ ] **Step 4: Implement targeted Python owner/profile resolution**

Define:

```python
LIST_PRESENTATION_TYPES = {
    "ExchangePlan",
    "FilterCriterion",
    "Catalog",
    "Document",
    "DocumentJournal",
    "Enum",
    "ChartOfCharacteristicTypes",
    "ChartOfAccounts",
    "ChartOfCalculationTypes",
    "InformationRegister",
    "AccumulationRegister",
    "AccountingRegister",
    "CalculationRegister",
    "BusinessProcess",
    "Task",
}


def resolve_owner_context(resolved_path, md_type, obj_name):
    if md_type in {"ExternalReport", "ExternalDataProcessor"}:
        return resolved_path, []

    config_dir = find_config_dir(resolved_path)
    if config_dir is None:
        raise ValueError(
            f"Configuration.xml owner not found for {md_type}.{obj_name}"
        )
    configuration_path = os.path.join(config_dir, "Configuration.xml")
    try:
        configuration_tree = etree.parse(
            configuration_path,
            etree.XMLParser(remove_blank_text=False),
        )
    except OSError as error:
        raise ValueError(f"failed to read owner {configuration_path}: {error}") from error
    except etree.XMLSyntaxError as error:
        raise ValueError(f"failed to parse {configuration_path}: {error}") from error

    configuration = required_configuration_element(
        configuration_tree,
        configuration_path,
    )
    if not configuration_registers(
        configuration,
        md_type,
        obj_name,
    ):
        raise ValueError(
            f"{md_type}.{obj_name} is not registered in {configuration_path}"
        )
    if md_type not in LIST_PRESENTATION_TYPES:
        return configuration_path, []
    return configuration_path, required_configuration_language_codes(
        configuration,
        config_dir,
    )
```

`required_configuration_language_codes` catches only
`OSError`/`etree.XMLSyntaxError` and raises a path-specific `ValueError` for
empty sets, missing files, malformed descriptors, or empty codes.

Implement its collaborators with these contracts:

```python
def find_config_dir(resolved_path):
    workspace_root = os.path.abspath(os.getcwd())
    directory = os.path.dirname(os.path.abspath(resolved_path))
    while os.path.commonpath([directory, workspace_root]) == workspace_root:
        candidate = os.path.join(directory, "Configuration.xml")
        if os.path.isfile(candidate):
            return directory
        if directory == workspace_root:
            break
        parent = os.path.dirname(directory)
        if parent == directory:
            break
        directory = parent
    return None


def required_configuration_element(configuration_tree, configuration_path):
    root = configuration_tree.getroot()
    artifacts = [
        child
        for child in root
        if isinstance(child.tag, str)
        and etree.QName(child).namespace == MD_NS
    ]
    if (
        etree.QName(root).namespace != MD_NS
        or etree.QName(root).localname != "MetaDataObject"
        or len(artifacts) != 1
        or etree.QName(artifacts[0]).localname != "Configuration"
    ):
        raise ValueError(
            f"{configuration_path} does not contain exactly one Configuration descriptor"
        )
    return artifacts[0]


def configuration_registers(configuration, md_type, obj_name):
    child_objects = configuration.find(f"./{{{MD_NS}}}ChildObjects")
    if child_objects is None:
        return False
    return any(
        etree.QName(child).localname == md_type
        and (child.text or "").strip() == obj_name
        for child in child_objects
    )


def required_configuration_language_codes(configuration, config_dir):
    child_objects = configuration.find(f"./{{{MD_NS}}}ChildObjects")
    language_names = [
        (child.text or "").strip()
        for child in (() if child_objects is None else child_objects)
        if etree.QName(child).localname == "Language" and (child.text or "").strip()
    ]
    if not language_names:
        configuration_path = os.path.join(config_dir, "Configuration.xml")
        raise ValueError(
            f"{configuration_path} has no registered language profile"
        )
    codes = []
    for language_name in language_names:
        language_path = os.path.join(config_dir, "Languages", f"{language_name}.xml")
        if not os.path.isfile(language_path):
            raise ValueError(
                f"registered language file not found: {language_path}"
            )
        try:
            language_tree = etree.parse(
                language_path,
                etree.XMLParser(remove_blank_text=False),
            )
        except OSError as error:
            raise ValueError(
                f"failed to read {language_path}: {error}"
            ) from error
        except etree.XMLSyntaxError as error:
            raise ValueError(
                f"failed to parse {language_path}: {error}"
            ) from error
        language_root = language_tree.getroot()
        language_artifacts = [
            child
            for child in language_root
            if isinstance(child.tag, str)
            and etree.QName(child).namespace == MD_NS
        ]
        if (
            etree.QName(language_root).namespace != MD_NS
            or etree.QName(language_root).localname != "MetaDataObject"
            or len(language_artifacts) != 1
            or etree.QName(language_artifacts[0]).localname != "Language"
        ):
            raise ValueError(
                f"registered language descriptor is not Language: {language_path}"
            )
        language = language_artifacts[0]
        code = language_tree.findtext(
            f".//{{{MD_NS}}}Language/{{{MD_NS}}}Properties/{{{MD_NS}}}LanguageCode"
        )
        code = (code or "").strip()
        if not code:
            raise ValueError(f"empty LanguageCode in {language_path}")
        if code not in codes:
            codes.append(code)
    return codes
```

Stop at the first existing `Configuration.xml`; if it does not register the
object, `resolve_owner_context` reports the mismatch and does not climb
farther.

Immediately after the script sets the validation header from `md_type` and
`obj_name`, resolve the owner and translate targeted failures into the normal
reporter rather than leaking a traceback:

```python
try:
    owner_path, language_codes = resolve_owner_context(
        resolved_path,
        md_type,
        obj_name,
    )
except ValueError as error:
    report_error(f"1. Owner context: {error}")
    finalize()
    sys.exit(1)
```

Delete the old `configuration_language_codes` helper. No owner/profile read
path may retain `except Exception` or return an empty list after an I/O/XML
failure.

- [ ] **Step 5: Replace Python fallback selection**

Run the command-text loop only for `md_type in LIST_PRESENTATION_TYPES`. Remove
observed `v8:lang` and neutral branches. Use the same per-language list-first
selection as Rust.

- [ ] **Step 6: Run parity and fixture validation**

```bash
python3.12 tests/ci/test_unica_mcp_script_parity.py -k meta_validate_language_aware
python3.12 tests/ci/test_unica_mcp_script_parity.py -k meta_validate_missing
python3.12 tests/ci/test_unica_mcp_script_parity.py
```

Expected: the focused scenario and the full parity suite pass.

- [ ] **Step 7: Commit parity and fixture changes**

```bash
git add \
  tests/ci/test_unica_mcp_script_parity.py \
  tests/fixtures/unica_mcp_script_parity/bsp/meta/Languages/Русский.xml \
  tests/fixtures/unica_mcp_script_parity/meta-validate-language-aware \
  tests/fixtures/unica_mcp_script_parity/meta-validate-parity-owner \
  tests/fixtures/unica_mcp_script_parity/reference_skills/meta-validate/scripts/meta-validate.py
git -c commit.gpgsign=false commit -m "Синхронизировать owner-aware parity meta-validate"
```

---

### Task 6: Synchronize Reader-Facing Documentation

**Files:**
- Modify: `plugins/unica/references/platform/metadata-conventions.md`
- Modify: `plugins/unica/references/README.md`
- Create: `tests/ci/test_reference_metadata_conventions.py`

**Interfaces:**
- Consumes: final owner/type/selection contract.
- Produces: a test-enforced reference contract with a valid relative link.

- [ ] **Step 1: Add the failing documentation contract**

Create:

```python
from pathlib import Path
import unittest


REPO_ROOT = Path(__file__).resolve().parents[2]


class MetadataConventionReferenceTests(unittest.TestCase):
    def test_reference_describes_owner_aware_list_presentation_contract(self) -> None:
        text = (
            REPO_ROOT
            / "plugins"
            / "unica"
            / "references"
            / "platform"
            / "metadata-conventions.md"
        ).read_text(encoding="utf-8")
        for marker in (
            "ListPresentation",
            "Configuration.xml",
            "Languages/<Name>.xml",
            "ExternalReport",
            "ExternalDataProcessor",
        ):
            self.assertIn(marker, text)
        self.assertNotIn("наблюдаемым значениям `v8:lang`", text)
        self.assertNotIn("языконезависим", text)

    def test_reference_index_uses_path_relative_to_itself(self) -> None:
        text = (
            REPO_ROOT / "plugins" / "unica" / "references" / "README.md"
        ).read_text(encoding="utf-8")
        self.assertIn("`platform/metadata-conventions.md`", text)
        self.assertNotIn("`references/platform/metadata-conventions.md`", text)


if __name__ == "__main__":
    unittest.main()
```

- [ ] **Step 2: Run the test and verify RED**

```bash
python3.12 tests/ci/test_reference_metadata_conventions.py
```

Expected: failures for missing owner/external markers and the broken README
path.

- [ ] **Step 3: Update the reference**

Replace the final paragraph of `## Представления` with:

```markdown
- Длина текста в командном интерфейсе — не больше 38 символов, лучше уложиться
  в 30. Правило применяется только к типам с платформенным свойством
  `ListPresentation`: `ExchangePlan`, `FilterCriterion`, `Catalog`, `Document`,
  `DocumentJournal`, `Enum`, `ChartOfCharacteristicTypes`, `ChartOfAccounts`,
  `ChartOfCalculationTypes`, `InformationRegister`, `AccumulationRegister`,
  `AccountingRegister`, `CalculationRegister`, `BusinessProcess`, `Task`.
  Фактическая область проверки — пересечение этого списка с типами, которые
  поддерживает `meta-validate`.
- Для любого объекта конфигурации или расширения `meta-validate` читает
  владеющий `Configuration.xml` и требует точную регистрацию пары
  тип/имя в `Configuration/ChildObjects`.
- Для каждого зарегистрированного языка код берётся только из
  `Languages/<Name>.xml`. Сначала проверяются все непустые значения
  `ListPresentation` с этим кодом; если их нет — все непустые значения
  `Synonym`. Наблюдаемые `v8:lang` и значения без языка не заменяют языковой
  профиль конфигурации.
- Отсутствующий или пустой перевод пропускается. Отсутствие владельца,
  регистрации, списка языков, зарегистрированного языкового файла или
  `LanguageCode` является ошибкой структуры проекта.
- `ExternalReport` и `ExternalDataProcessor` — самостоятельные корневые
  артефакты EPF/ERF. Они не наследуют соседний `Configuration.xml`, а правило
  длины представления списка к ним не применяется.
```

Fix the README entry to:

```markdown
- `platform/metadata-conventions.md` — object naming, synonym, representation, and fill-check conventions.
```

- [ ] **Step 4: Run documentation and package contracts**

```bash
python3.12 tests/ci/test_reference_metadata_conventions.py
python3.12 tests/ci/test_attributions.py
python3.12 tests/ci/test_skill_provenance.py
python3.12 tests/ci/test_package_unica_plugin.py
```

Expected: all commands pass.

- [ ] **Step 5: Commit the reference synchronization**

```bash
git add \
  plugins/unica/references/README.md \
  plugins/unica/references/platform/metadata-conventions.md \
  tests/ci/test_reference_metadata_conventions.py
git -c commit.gpgsign=false commit -m "Документировать owner-aware проверку представлений"
```

---

### Task 7: Full Verification And Review Handoff

**Files:**
- Verify: all files changed from `origin/main...HEAD`
- Do not push or resolve GitHub threads without explicit user authorization

**Interfaces:**
- Consumes: all prior tasks.
- Produces: a locally reviewable PR branch with fresh evidence.

- [ ] **Step 1: Run formatting and static checks**

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
git diff --check origin/main...HEAD
```

Expected: every command exits zero.

- [ ] **Step 2: Run the full Rust suite serially**

```bash
cargo test --workspace -- --test-threads=1
```

Expected: zero failed tests. Record passed and ignored counts from the fresh
output.

- [ ] **Step 3: Run the full Python CI suite**

```bash
python3.12 -m unittest discover -s tests/ci -p 'test_*.py'
```

Expected: zero failed tests. Record passed and skipped counts.

- [ ] **Step 4: Audit the final behavior contract**

Run:

```bash
rg -n "InternalLocalOwnerOnly|internalLocalOwnerOnly|follow_metadata_references" \
  crates/unica-coder/src
rg -n "observed_language|language-neutral|observed.*v8:lang" \
  crates/unica-coder/src/infrastructure/native_operations/meta.rs \
  tests/fixtures/unica_mcp_script_parity/reference_skills/meta-validate/scripts/meta-validate.py
git diff --stat origin/main...HEAD
git status --short --branch
```

Expected:

- both forbidden-pattern searches return no matches;
- the diff contains only the approved validator, fixture, reference, test,
  spec, and plan scope;
- the worktree is clean.

- [ ] **Step 5: Inspect the live PR state without writing**

```bash
gh pr view 184 --repo IngvarConsulting/unica \
  --json number,url,headRefName,headRefOid,baseRefName,mergeable,statusCheckRollup
```

Expected: PR 184 is still open. Report whether its remote head differs from the
local branch and which checks would rerun after a push.

- [ ] **Step 6: Present the verified branch**

Report:

- local branch and commit list;
- exact Rust/Python test counts;
- PR 188 ancestry proof;
- resolved findings and any remaining risk;
- that no push, GitHub reply, thread resolution, or review submission has been
  performed.
