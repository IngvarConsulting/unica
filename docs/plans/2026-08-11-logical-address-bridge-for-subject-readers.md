# Logical Address Bridge for Subject Readers Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> `superpowers:subagent-driven-development` (recommended) or
> `superpowers:executing-plans` to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking. Every production change starts with a
> failing test, records the observed RED reason, and ends with the narrow GREEN
> command before broader verification.

**Goal:** Thirteen subject readers and validators accept `sourceSet +
metadataPath` in addition to their existing physical path, so an address found
by `unica.source.resolve` can be fed straight to `unica.form.info` without the
caller knowing the Designer layout. Closes
[#272](https://github.com/IngvarConsulting/unica/issues/272) for the read
surface and [#299](https://github.com/IngvarConsulting/unica/issues/299), and
records [#273](https://github.com/IngvarConsulting/unica/issues/273) as already
delivered.

**Architecture:** One new seam generalises the proof that
`unica.role.edit` already performs — resolve the logical target, take the proven
descriptor, derive the attached resource under it, re-prove containment. Every
reader reaches that seam through the args→path helper it already owns, so
`handler_resolved_format_paths` and the handler both pick up the logical
selector from a single change per tool. Handler bodies below the path are not
touched.

**Tech Stack:** Rust 2021, `roxmltree`, existing
`domain/source_target.rs` + `infrastructure/platform_xml_source_targets.rs` +
`infrastructure/native_operations/` seams, Python CI contract tests, Markdown
ADR/invariant/acceptance corpus.

## Global Constraints

- ADR-0048 owns the transitional state. A reader publishes **both** selectors and
  requires **exactly one**: neither is the existing missing-argument error, both
  is `selector_conflict`.
- The bridge does not change any answer. For the same object, the logical call
  and the path call return byte-identical typed data. This is the primary
  assertion of Tasks 4–6.
- No handler body below the path resolution changes. Support reading, validation,
  typed data and cache events stay as they are.
- The seam adds no second route to the source. It calls
  `resolve_platform_xml_target(context, &target, TargetKindPolicy::Any)` and
  `platform_xml_resource_evidence`, never its own layout walk.
- Address profile is `PLATFORM_XML_8_3_27_FORMAT_2_20` (ADR-0016). Unknown kind,
  terminal or profile is refused, never guessed from a directory name.
- Refusal messages carry the logical address and source set and **never** a path
  separator (`INV-MCP-SOURCE-SURFACE` spirit; the readers themselves still report
  physical artifacts, which is unchanged and allowed by ADR-0021 §9).
- Mutating tools are out. `form.edit`, `dcs.edit`, `subsystem.edit`, `cf.edit`,
  `support.edit`, `interface.*`, `template.*`, `help.add`, `cfe.*` keep their
  path-only contract.
- Nested subsystems are out: `Subsystem.A.B` does not parse and ADR-0036 keeps
  the nested node outside `unica.source.*`.
- `unica.cfe.diff` is out: it needs two source sets at once and `SourceTarget`
  carries one.
- Run every Rust test with `-- --test-threads=1`. Run Python CI with
  `/opt/homebrew/bin/python3.12`; other interpreters on this machine lack `lxml`
  and fail unrelated suites.

## File Structure

| File | Responsibility |
| --- | --- |
| Create: `crates/unica-coder/src/infrastructure/native_operations/logical_selector.rs` | The whole seam: `AttachedResource`, `LogicalSelection`, `LogicalSelectorFailure`, `logical_selection`, `prove_attached_resource`. One job, read in one sitting — hence a new file rather than growth in `common.rs` (2896 lines) or `form.rs` (19047). |
| Modify: `crates/unica-coder/src/infrastructure/platform_xml_source_targets.rs` | #299: apply `TargetKindPolicy` to every target kind, including `SourceRoot`. |
| Modify: `crates/unica-coder/src/infrastructure/native_operations/mod.rs` | Declare the new module. |
| Modify: `crates/unica-coder/src/infrastructure/native_operations/role.rs` | `prove_role_rights_path` becomes a call into the shared seam; `resolve_role_read_rights_path` gains the logical branch. |
| Modify: `crates/unica-coder/src/infrastructure/native_operations/common.rs` | `resolve_role_read_rights_path` and `resolve_cf_read_config_path` gain the logical branch. |
| Modify: `crates/unica-coder/src/infrastructure/native_operations/mxl.rs` | `resolve_mxl_info_path`, `resolve_mxl_validate_path`, new `resolve_mxl_decompile_path`. |
| Modify: `crates/unica-coder/src/infrastructure/native_operations/dcs.rs` | `inspect_dcs_info_path`, `resolve_dcs_validate_path`. |
| Modify: `crates/unica-coder/src/infrastructure/native_operations/form.rs` | New `resolve_form_read_path(args, context)` replacing two inline `required_path` calls. |
| Modify: `crates/unica-coder/src/infrastructure/native_operations/subsystem.rs` | New `resolve_subsystem_read_path(args, context)` replacing three inline `required_path` calls. |
| Modify: `crates/unica-coder/src/infrastructure/format_guard.rs` | Arms for `mxl-info`, `dcs-info`, `subsystem-info`, `subsystem-validate` so the guard resolves the same way the handler does. |
| Modify: `crates/unica-coder/src/application/tool_contracts.rs` | Thirteen argument lists gain `sourceSet` + `metadataPath`; one shared exactly-one-of check. |
| Modify: `crates/unica-coder/src/application/operation_descriptors.rs` | Thirteen descriptors stop requiring the path argument. |
| Modify: `plugins/unica/skills/*/SKILL.md`, `plugins/unica/README.md` | Executable examples and the selector table. |
| Modify: `spec/architecture/invariants.md`, `spec/architecture/tool-surface.md`, `spec/acceptance/logical-source-addressing-and-resource-access.md` | Derived rule, regenerated ledger, acceptance rows. |

---

## Task 1: Apply the target-kind policy to the source root

Closes #299. Done first because Tasks 2–6 all resolve through this function, and
because it is the only change here that touches a write boundary.

**Files:**

- Modify: `crates/unica-coder/src/infrastructure/platform_xml_source_targets.rs:2181`
- Test: same file, `mod tests`

**Interfaces:**

- Consumes: nothing from earlier tasks.
- Produces: `resolve_platform_xml_target(context, &target, policy)` now refuses
  `(SourceRoot, ModuleOnly)` and `(SourceRoot, ModuleOnlyAllowingAbsent)` with
  `SourceTargetErrorCode::TargetKindMismatch`. Later tasks pass
  `TargetKindPolicy::Any` and are unaffected.

- [ ] **Step 1: Write the failing test**

Add to the `tests` module of
`crates/unica-coder/src/infrastructure/platform_xml_source_targets.rs`:

```rust
#[test]
fn platform_xml_target_kind_policy_applies_to_every_target_kind() {
    let context = fixture(
        "policy-covers-root",
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: cfg\n",
    );
    write_module_fixture(
        &context.workspace_root.join("cfg"),
        "CommonModules/Shared.xml",
        "Shared",
    );

    let root = SourceTarget {
        source_set: "main".to_string(),
        metadata_path: None,
    };
    let object = SourceTarget {
        source_set: "main".to_string(),
        metadata_path: Some(
            MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "CommonModule.Shared").unwrap(),
        ),
    };
    let module = SourceTarget {
        source_set: "main".to_string(),
        metadata_path: Some(
            MetadataAddress::parse(
                PLATFORM_XML_8_3_27_FORMAT_2_20,
                "CommonModule.Shared.Module",
            )
            .unwrap(),
        ),
    };

    // `Any` keeps today's read behaviour for all three kinds.
    for target in [&root, &object, &module] {
        assert!(
            super::resolve_platform_xml_target(&context, target, super::TargetKindPolicy::Any)
                .is_ok(),
            "Any must resolve every kind"
        );
    }

    // `ModuleOnly` admits the module terminal and nothing else.
    assert!(super::resolve_platform_xml_target(
        &context,
        &module,
        super::TargetKindPolicy::ModuleOnly
    )
    .is_ok());
    for target in [&root, &object] {
        let error =
            super::resolve_platform_xml_target(&context, target, super::TargetKindPolicy::ModuleOnly)
                .expect_err("ModuleOnly must refuse a non-module target");
        assert_eq!(error.code, SourceTargetErrorCode::TargetKindMismatch);
        assert!(
            !error.message.contains(std::path::MAIN_SEPARATOR),
            "a refusal must not disclose a physical path: {}",
            error.message
        );
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo test -p unica-coder platform_xml_target_kind_policy_applies -- --test-threads=1
```

Expected: FAIL. `ModuleOnly` + root returns `Ok` because
`resolve_platform_xml_target` returns `resolve_platform_xml_root` before the
`(target_kind, policy)` match. Record the observed assertion text.

- [ ] **Step 3: Move the policy check ahead of the root branch**

In `resolve_platform_xml_target`, replace the early root return with a policy-first
form:

```rust
    let Some(address) = target.metadata_path.as_ref() else {
        // The policy is a fail-closed declaration, so it decides before any
        // resolution succeeds — including the source root, which has no
        // `metadataPath` to match on.
        if !matches!(policy, TargetKindPolicy::Any) {
            return Err(SourceTargetError::new(
                SourceTargetErrorCode::TargetKindMismatch,
                "metadataPath does not identify a module terminal",
            ));
        }
        return resolve_platform_xml_root(context, target, selected);
    };
    match (address.target_kind(), policy) {
        (TargetKind::Module, _) => {
            resolve_platform_xml_module(context, target, selected, address, policy)
        }
        (TargetKind::MetadataObject, TargetKindPolicy::Any) => {
            resolve_platform_xml_object(context, target, selected, address)
        }
        _ => Err(SourceTargetError::new(
            SourceTargetErrorCode::TargetKindMismatch,
            "metadataPath does not identify a module terminal",
        )),
    }
```

- [ ] **Step 4: Run the narrow test, then the file's suite**

```bash
cargo test -p unica-coder platform_xml_target_kind_policy_applies -- --test-threads=1
```

Expected: PASS. Then:

```bash
cargo test -p unica-coder platform_xml_source_targets -- --test-threads=1
```

Expected: PASS. Then prove the writer is unaffected:

```bash
cargo test -p unica-coder code_patch -- --test-threads=1
```

- [ ] **Step 5: Commit**

```bash
git add crates/unica-coder/src/infrastructure/platform_xml_source_targets.rs
git commit -m "fix(source): apply the target-kind policy to the source root"
```

---

## Task 2: The logical selector seam

**Files:**

- Create: `crates/unica-coder/src/infrastructure/native_operations/logical_selector.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/mod.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/role.rs:2517-2568`
- Test: inside `logical_selector.rs`

**Interfaces:**

- Consumes: Task 1's `resolve_platform_xml_target(context, &target, TargetKindPolicy::Any)`.
- Produces, for Tasks 4–6:

```rust
pub(crate) enum AttachedResource { ConfigurationRoot, Descriptor, Rights, Form, Template }
pub(crate) struct LogicalSelection {
    pub(crate) source_set: String,
    pub(crate) metadata_path: Option<MetadataAddress>,
    pub(crate) resource_path: PathBuf,
}
pub(crate) struct LogicalSelectorFailure { code: &'static str, reason: String }
impl LogicalSelectorFailure { pub(crate) fn code(&self) -> &'static str; }
impl std::fmt::Display for LogicalSelectorFailure;

/// `None` — the caller used the legacy path argument, resolve it as before.
/// `Some(Err(_))` — the caller used the logical selector and it failed.
pub(crate) fn logical_selection(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    want: AttachedResource,
    accepted_kinds: &[&str],
) -> Option<Result<LogicalSelection, LogicalSelectorFailure>>;
```

`accepted_kinds` lists the leading canonical tokens the tool reads — `&["Role"]`
for `role.info`, `&["Form", "CommonForm"]` for `form.info`. An empty slice
accepts any object address.

- [ ] **Step 1: Write the failing tests**

Create `crates/unica-coder/src/infrastructure/native_operations/logical_selector.rs`
containing only the test module for now:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Minimal Designer dump: one role, one catalog with one form, one report
    /// with a DCS template, and a binary common template.
    fn fixture(name: &str) -> WorkspaceContext {
        let context = crate::infrastructure::native_operations::tests::workspace(name);
        let src = context.workspace_root.join("src");
        fs::write(
            context.workspace_root.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        write_descriptor(&src, "Roles/Sales.xml", "Role", "Sales");
        fs::create_dir_all(src.join("Roles/Sales/Ext")).unwrap();
        fs::write(src.join("Roles/Sales/Ext/Rights.xml"), "<Rights/>").unwrap();

        write_descriptor(&src, "Catalogs/Items.xml", "Catalog", "Items");
        write_descriptor(&src, "Catalogs/Items/Forms/Order.xml", "Form", "Order");
        fs::create_dir_all(src.join("Catalogs/Items/Forms/Order/Ext")).unwrap();
        fs::write(src.join("Catalogs/Items/Forms/Order/Ext/Form.xml"), "<Form/>").unwrap();

        write_descriptor(&src, "CommonTemplates/Logo.xml", "Template", "Logo");
        fs::create_dir_all(src.join("CommonTemplates/Logo/Ext")).unwrap();
        // A binary template: the descriptor exists, `Template.xml` does not.
        fs::write(src.join("CommonTemplates/Logo/Ext/Template.bin"), [0u8, 1]).unwrap();

        register_children(&src, &["Role.Sales", "Catalog.Items", "CommonTemplate.Logo"]);
        context
    }

    fn selection(
        context: &WorkspaceContext,
        address: &str,
        want: AttachedResource,
        kinds: &[&str],
    ) -> Result<LogicalSelection, LogicalSelectorFailure> {
        let args = Map::from_iter([
            ("sourceSet".to_string(), Value::String("main".to_string())),
            ("metadataPath".to_string(), Value::String(address.to_string())),
        ]);
        logical_selection(&args, context, want, kinds).expect("logical selector was supplied")
    }

    #[test]
    fn attached_resources_are_derived_from_the_proven_descriptor() {
        let context = fixture("attached-resources");
        let src = context.workspace_root.join("src");

        assert_eq!(
            selection(&context, "Role.Sales", AttachedResource::Rights, &["Role"])
                .unwrap()
                .resource_path,
            src.join("Roles/Sales/Ext/Rights.xml")
        );
        assert_eq!(
            selection(
                &context,
                "Catalog.Items.Form.Order",
                AttachedResource::Form,
                &["Form"]
            )
            .unwrap()
            .resource_path,
            src.join("Catalogs/Items/Forms/Order/Ext/Form.xml")
        );
        assert_eq!(
            selection(&context, "Catalog.Items", AttachedResource::Descriptor, &[])
                .unwrap()
                .resource_path,
            src.join("Catalogs/Items.xml")
        );
    }

    #[test]
    fn the_source_root_selects_the_configuration_descriptor() {
        let context = fixture("root-selects-configuration");
        let args = Map::from_iter([("sourceSet".to_string(), Value::String("main".to_string()))]);
        let selection = logical_selection(&args, &context, AttachedResource::ConfigurationRoot, &[])
            .expect("sourceSet alone is a logical selector")
            .unwrap();
        assert_eq!(
            selection.resource_path,
            context.workspace_root.join("src/Configuration.xml")
        );
        assert_eq!(selection.metadata_path, None);
    }

    #[test]
    fn a_proven_object_without_the_requested_resource_is_not_a_missing_target() {
        let context = fixture("binary-template");
        let failure = selection(
            &context,
            "CommonTemplate.Logo",
            AttachedResource::Template,
            &["CommonTemplate"],
        )
        .expect_err("a .bin template has no Template.xml");
        assert_eq!(failure.code(), "resource_absent");
    }

    #[test]
    fn an_address_of_another_kind_is_refused_by_kind_not_by_absence() {
        let context = fixture("wrong-kind");
        let failure = selection(&context, "Catalog.Items", AttachedResource::Rights, &["Role"])
            .expect_err("a catalog is not a role");
        assert_eq!(failure.code(), "target_kind_unsupported");
    }

    #[test]
    fn a_symlinked_resource_is_refused() {
        let context = fixture("symlinked-resource");
        let src = context.workspace_root.join("src");
        let planted = src.join("planted.xml");
        fs::write(&planted, "<Rights/>").unwrap();
        fs::remove_file(src.join("Roles/Sales/Ext/Rights.xml")).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&planted, src.join("Roles/Sales/Ext/Rights.xml")).unwrap();
        #[cfg(not(unix))]
        return;

        let failure = selection(&context, "Role.Sales", AttachedResource::Rights, &["Role"])
            .expect_err("a symlinked resource is not a direct regular file");
        assert_eq!(failure.code(), "containment_denied");
    }

    #[test]
    fn an_unknown_address_and_an_unknown_source_set_are_separate_codes() {
        let context = fixture("separate-codes");
        assert_eq!(
            selection(&context, "Role.Missing", AttachedResource::Rights, &["Role"])
                .expect_err("absent role")
                .code(),
            "target_not_found"
        );
        let args = Map::from_iter([
            ("sourceSet".to_string(), Value::String("nope".to_string())),
            ("metadataPath".to_string(), Value::String("Role.Sales".to_string())),
        ]);
        assert_eq!(
            logical_selection(&args, &context, AttachedResource::Rights, &["Role"])
                .unwrap()
                .expect_err("absent source set")
                .code(),
            "source_set_unknown"
        );
    }

    #[test]
    fn no_refusal_message_discloses_a_path() {
        let context = fixture("no-path-in-message");
        for (address, kinds) in [
            ("Role.Missing", &["Role"][..]),
            ("Catalog.Items", &["Role"][..]),
        ] {
            let failure = selection(&context, address, AttachedResource::Rights, kinds)
                .expect_err("refusal expected");
            assert!(
                !failure.to_string().contains(std::path::MAIN_SEPARATOR),
                "refusal disclosed a path: {failure}"
            );
        }
    }

    #[test]
    fn a_legacy_path_call_is_not_a_logical_selection() {
        let context = fixture("legacy-path");
        let args = Map::from_iter([(
            "RightsPath".to_string(),
            Value::String("src/Roles/Sales/Ext/Rights.xml".to_string()),
        )]);
        assert!(logical_selection(&args, &context, AttachedResource::Rights, &["Role"]).is_none());
    }
}
```

This task also adds the two helpers the fixture uses, `pub(crate)` so Tasks 4–6
build the same dump instead of inventing a second shape:

```rust
/// A minimal 8.3.27 / format 2.20 descriptor: the platform proves identity by
/// the kind tag and `<Name>`, and the resolver checks exactly that.
pub(crate) fn write_descriptor(source_root: &Path, relative: &str, kind: &str, name: &str) {
    let path = source_root.join(relative);
    fs::create_dir_all(path.parent().expect("descriptor has a directory")).unwrap();
    fs::write(
        &path,
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
             <MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.20\">\n\
             \t<{kind} uuid=\"11111111-2222-3333-4444-555555555555\">\n\
             \t\t<Properties>\n\t\t\t<Name>{name}</Name>\n\t\t</Properties>\n\
             \t</{kind}>\n</MetaDataObject>\n"
        ),
    )
    .unwrap();
}

/// Registration is what makes an address resolvable: a descriptor on disk that
/// `Configuration.xml` does not list is not a target.
pub(crate) fn register_children(source_root: &Path, addresses: &[&str]) {
    let children = addresses
        .iter()
        .map(|address| format!("\t\t<ChildObjects>{address}</ChildObjects>\n"))
        .collect::<String>();
    fs::create_dir_all(source_root).unwrap();
    fs::write(
        source_root.join("Configuration.xml"),
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
             <MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.20\">\n\
             \t<Configuration uuid=\"99999999-8888-7777-6666-555555555555\">\n\
             \t\t<Properties>\n\t\t\t<Name>Fixture</Name>\n\t\t</Properties>\n\
             {children}\t</Configuration>\n</MetaDataObject>\n"
        ),
    )
    .unwrap();
}
```

Before writing them, read the `fixture(...)` and `write_module_fixture` helpers
in the `platform_xml_source_targets.rs` test module and match their exact
`Configuration.xml` shape — the registration format is what
`object_registration_evidence` parses, and a near-miss there fails every test in
this task for the wrong reason.

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test -p unica-coder logical_selector -- --test-threads=1
```

Expected: FAIL to compile — `logical_selection`, `AttachedResource`,
`LogicalSelection` do not exist. Record the compiler error.

- [ ] **Step 3: Implement the seam**

Above the test module in the same file:

```rust
//! Logical target → attached resource for subject readers (ADR-0048).
//!
//! The rule is one line of the 8.3.27 layout: a descriptor `<…>/<Stem>.xml`
//! owns its content under `<…>/<Stem>/Ext/<Resource>`. Everything else here is
//! re-proving that the derived path is still inside the selected source set.

use serde_json::{Map, Value};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use crate::domain::source_target::{
    MetadataAddress, SourceTarget, SourceTargetErrorCode, TargetKind,
    PLATFORM_XML_8_3_27_FORMAT_2_20,
};
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::native_operations::common::string_arg;
use crate::infrastructure::path_policy::WorkspacePathPolicy;
use crate::infrastructure::platform::filesystem::metadata_is_link_or_reparse_point;
use crate::infrastructure::platform_xml_source_targets::{
    platform_xml_resource_evidence, resolve_platform_xml_target, PlatformXmlResourceEvidence,
    TargetKindPolicy,
};
use crate::infrastructure::source_roots::normalize_path_identity;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AttachedResource {
    /// `Configuration.xml` at the root of the selected source set.
    ConfigurationRoot,
    /// The descriptor itself is what the reader parses.
    Descriptor,
    Rights,
    Form,
    Template,
}

impl AttachedResource {
    /// The file name under `Ext/`. `None` means the descriptor is the resource.
    const fn file_name(self) -> Option<&'static str> {
        match self {
            Self::ConfigurationRoot | Self::Descriptor => None,
            Self::Rights => Some("Rights.xml"),
            Self::Form => Some("Form.xml"),
            // `TemplateType` decides the extension in a real dump — `.xml`,
            // `.bin` or `.txt`. The readers that ask for a template parse XML,
            // so a binary or text template is `resource_absent`, not a miss.
            Self::Template => Some("Template.xml"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LogicalSelection {
    pub(crate) source_set: String,
    pub(crate) metadata_path: Option<MetadataAddress>,
    pub(crate) resource_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LogicalSelectorFailure {
    code: &'static str,
    reason: String,
}

impl LogicalSelectorFailure {
    fn new(code: &'static str, reason: impl Into<String>) -> Self {
        Self { code, reason: reason.into() }
    }

    pub(crate) fn code(&self) -> &'static str {
        self.code
    }
}

impl std::fmt::Display for LogicalSelectorFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.code, self.reason)
    }
}

pub(crate) fn logical_selection(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    want: AttachedResource,
    accepted_kinds: &[&str],
) -> Option<Result<LogicalSelection, LogicalSelectorFailure>> {
    let source_set = string_arg(args, &["sourceSet"])?.to_string();
    Some(resolve(args, context, want, accepted_kinds, source_set))
}

fn resolve(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    want: AttachedResource,
    accepted_kinds: &[&str],
    source_set: String,
) -> Result<LogicalSelection, LogicalSelectorFailure> {
    let metadata_path = match string_arg(args, &["metadataPath"]) {
        Some(raw) => Some(
            MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, raw).map_err(|error| {
                selector_failure(error.code, "metadataPath is not a valid logical address")
            })?,
        ),
        None => None,
    };
    if let Some(address) = metadata_path.as_ref() {
        let leading = address.as_str().split('.').next().unwrap_or_default();
        let nested = address.as_str().split('.').nth(2).unwrap_or_default();
        if !accepted_kinds.is_empty()
            && !accepted_kinds.contains(&leading)
            && !accepted_kinds.contains(&nested)
        {
            return Err(LogicalSelectorFailure::new(
                "target_kind_unsupported",
                "metadataPath does not identify what this tool reads",
            ));
        }
    }

    let target = SourceTarget { source_set: source_set.clone(), metadata_path: metadata_path.clone() };
    let resolution = resolve_platform_xml_target(context, &target, TargetKindPolicy::Any)
        .map_err(|error| selector_failure(error.code, "the logical target could not be resolved"))?;
    let evidence = platform_xml_resource_evidence(context, &resolution.handle).map_err(|_| {
        LogicalSelectorFailure::new("provider_unavailable", "the target evidence is unavailable")
    })?;

    let resource_path = match (resolution.resolved.target_kind, want) {
        (TargetKind::SourceRoot, AttachedResource::ConfigurationRoot) => {
            prove_regular_file(&evidence, evidence.target_path.join("Configuration.xml"), context)?
        }
        (TargetKind::MetadataObject, AttachedResource::Descriptor) => {
            prove_regular_file(&evidence, evidence.target_path.clone(), context)?
        }
        (TargetKind::MetadataObject, _) => {
            let file_name = want.file_name().expect("object resources are named");
            prove_attached_resource(&evidence, file_name, context)?
        }
        _ => {
            return Err(LogicalSelectorFailure::new(
                "target_kind_unsupported",
                "metadataPath does not identify what this tool reads",
            ))
        }
    };

    Ok(LogicalSelection {
        source_set: resolution.resolved.source_set,
        metadata_path: resolution.resolved.metadata_path,
        resource_path,
    })
}

fn selector_failure(code: SourceTargetErrorCode, reason: &'static str) -> LogicalSelectorFailure {
    let code = match code {
        SourceTargetErrorCode::SourceSetRequired | SourceTargetErrorCode::SourceSetNotFound => {
            "source_set_unknown"
        }
        SourceTargetErrorCode::MetadataAddressNotFound => "target_not_found",
        SourceTargetErrorCode::TargetKindMismatch
        | SourceTargetErrorCode::MetadataAddressInvalid => "target_kind_unsupported",
        SourceTargetErrorCode::ContainmentDenied => "containment_denied",
        SourceTargetErrorCode::AddressProfileUnsupported => "profile_unsupported",
        SourceTargetErrorCode::SourceRootNotAddressable => "provider_unavailable",
    };
    LogicalSelectorFailure::new(code, reason)
}

/// `<…>/<Stem>.xml` → `<…>/<Stem>/Ext/<file_name>`.
fn prove_attached_resource(
    evidence: &PlatformXmlResourceEvidence,
    file_name: &str,
    context: &WorkspaceContext,
) -> Result<PathBuf, LogicalSelectorFailure> {
    let stem = evidence
        .target_path
        .file_stem()
        .and_then(|value| value.to_str())
        .ok_or_else(|| {
            LogicalSelectorFailure::new("provider_unavailable", "the descriptor has no stem")
        })?;
    let parent = evidence.target_path.parent().ok_or_else(|| {
        LogicalSelectorFailure::new("provider_unavailable", "the descriptor has no directory")
    })?;
    prove_regular_file(evidence, parent.join(stem).join("Ext").join(file_name), context)
}

fn prove_regular_file(
    evidence: &PlatformXmlResourceEvidence,
    candidate: PathBuf,
    context: &WorkspaceContext,
) -> Result<PathBuf, LogicalSelectorFailure> {
    let candidate = WorkspacePathPolicy::new(context)
        .resolve_write(candidate)
        .map_err(|_| {
            LogicalSelectorFailure::new(
                "containment_denied",
                "the resource is outside the workspace boundary",
            )
        })?;
    ensure_no_link_components(&evidence.source_root, &candidate)?;
    let normalized_root = normalize_path_identity(&evidence.source_root).map_err(|_| {
        LogicalSelectorFailure::new("provider_unavailable", "source root identity is unavailable")
    })?;
    let normalized = normalize_path_identity(&candidate).map_err(|_| {
        LogicalSelectorFailure::new("resource_absent", "the requested resource is not present")
    })?;
    if !normalized.starts_with(&normalized_root) {
        return Err(LogicalSelectorFailure::new(
            "containment_denied",
            "the resource escaped the selected source set",
        ));
    }
    let metadata = fs::symlink_metadata(&candidate).map_err(|_| {
        LogicalSelectorFailure::new("resource_absent", "the requested resource is not present")
    })?;
    if metadata_is_link_or_reparse_point(&metadata) || !metadata.is_file() {
        return Err(LogicalSelectorFailure::new(
            "containment_denied",
            "the resource is not a direct regular file",
        ));
    }
    Ok(candidate)
}

fn ensure_no_link_components(
    source_root: &Path,
    target: &Path,
) -> Result<(), LogicalSelectorFailure> {
    let relative = target.strip_prefix(source_root).map_err(|_| {
        LogicalSelectorFailure::new(
            "containment_denied",
            "the resource escaped the selected source set",
        )
    })?;
    let mut current = source_root.to_path_buf();
    for component in relative.components() {
        let std::path::Component::Normal(component) = component else {
            return Err(LogicalSelectorFailure::new(
                "containment_denied",
                "the resource contains a non-normal path component",
            ));
        };
        current.push(component);
        let metadata = match fs::symlink_metadata(&current) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == ErrorKind::NotFound => {
                return Err(LogicalSelectorFailure::new(
                    "resource_absent",
                    "the requested resource is not present",
                ))
            }
            Err(_) => {
                return Err(LogicalSelectorFailure::new(
                    "provider_unavailable",
                    "the resource is unreadable",
                ))
            }
        };
        if metadata_is_link_or_reparse_point(&metadata) {
            return Err(LogicalSelectorFailure::new(
                "containment_denied",
                "the resource path traverses a link",
            ));
        }
    }
    Ok(())
}
```

Declare the module in
`crates/unica-coder/src/infrastructure/native_operations/mod.rs` next to the
other `mod` lines, in alphabetical position:

```rust
pub(crate) mod logical_selector;
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
cargo test -p unica-coder logical_selector -- --test-threads=1
```

Expected: PASS, eight tests.

- [ ] **Step 5: Retire the private copy in `role.rs`**

`prove_role_rights_path` and `ensure_role_no_link_components`
(`role.rs:2517-2568`) are the same proof for one resource. Replace the body of
`prove_role_rights_path` with a call to the shared function and delete
`ensure_role_no_link_components`:

```rust
fn prove_role_rights_path(
    evidence: &PlatformXmlResourceEvidence,
    role_name: &str,
    context: &WorkspaceContext,
) -> Result<PathBuf, String> {
    if evidence.target_path.file_stem().and_then(|value| value.to_str()) != Some(role_name) {
        return Err("role descriptor identity does not match metadataPath".to_string());
    }
    crate::infrastructure::native_operations::logical_selector::prove_attached_resource(
        evidence,
        "Rights.xml",
        context,
    )
    .map_err(|failure| failure.to_string())
}
```

Mark `prove_attached_resource` `pub(crate)` for this. Run the role suite to prove
`unica.role.edit` is unchanged:

```bash
cargo test -p unica-coder role_edit -- --test-threads=1
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/unica-coder/src/infrastructure/native_operations/logical_selector.rs \
        crates/unica-coder/src/infrastructure/native_operations/mod.rs \
        crates/unica-coder/src/infrastructure/native_operations/role.rs
git commit -m "feat(source): derive an attached resource from a logical target"
```

---

## Task 3: Publish both selectors and require exactly one

**Files:**

- Modify: `crates/unica-coder/src/application/tool_contracts.rs:31-58` (the arg lists), plus the validation site near `validate_unique_alias_group` (`:1504`)
- Modify: `crates/unica-coder/src/application/operation_descriptors.rs:225-420`
- Test: `crates/unica-coder/src/application/tool_contracts.rs` `mod tests`

**Interfaces:**

- Consumes: nothing from Tasks 1–2 — this is the wire contract alone.
- Produces, for Tasks 4–6: each of the thirteen tools accepts `sourceSet` and
  `metadataPath`; the handler may assume the arguments passed validation.

The thirteen tools take **three** shapes of logical selector, not one. Publishing
`metadataPath` where the tool cannot use it would be a lie in the schema:

| Tool | Logical selector | Legacy argument | Accepted kinds |
| --- | --- | --- | --- |
| `unica.cf.info`, `unica.cf.validate` | `sourceSet` only — a configuration root has no address | `ConfigPath` | — |
| `unica.subsystem.info` | `sourceSet`, `metadataPath` optional — without it, the whole registered tree | `SubsystemPath` | `Subsystem` |
| `unica.subsystem.validate` | `sourceSet` + `metadataPath` — the validator reads one XML | `SubsystemPath` | `Subsystem` |
| `unica.role.info`, `unica.role.validate` | `sourceSet` + `metadataPath` | `RightsPath` | `Role` |
| `unica.form.info`, `unica.form.validate` | `sourceSet` + `metadataPath` | `FormPath` | `Form`, `CommonForm` |
| `unica.dcs.info`, `unica.dcs.validate` | `sourceSet` + `metadataPath` | `TemplatePath` | `Template`, `CommonTemplate` |
| `unica.mxl.info`, `unica.mxl.validate`, `unica.mxl.decompile` | `sourceSet` + `metadataPath` | `TemplatePath` | `Template`, `CommonTemplate` |

`unica.cf.info` and `unica.cf.validate` therefore publish `sourceSet` and **not**
`metadataPath`.

- [ ] **Step 1: Write the failing tests**

In the `tests` module of `tool_contracts.rs`:

```rust
/// The thirteen readers bridged by ADR-0048: tool name, its legacy target
/// argument, and whether the logical branch also requires `metadataPath`.
const BRIDGED_READERS: &[(&str, &str, bool)] = &[
    ("unica.cf.info", "ConfigPath", false),
    ("unica.cf.validate", "ConfigPath", false),
    ("unica.subsystem.info", "SubsystemPath", false),
    ("unica.subsystem.validate", "SubsystemPath", true),
    ("unica.role.info", "RightsPath", true),
    ("unica.role.validate", "RightsPath", true),
    ("unica.form.info", "FormPath", true),
    ("unica.form.validate", "FormPath", true),
    ("unica.dcs.info", "TemplatePath", true),
    ("unica.dcs.validate", "TemplatePath", true),
    ("unica.mxl.info", "TemplatePath", true),
    ("unica.mxl.validate", "TemplatePath", true),
    ("unica.mxl.decompile", "TemplatePath", true),
];

#[test]
fn bridged_readers_publish_two_mutually_exclusive_selector_branches() {
    for (name, legacy, needs_address) in BRIDGED_READERS {
        let tool = tools()
            .into_iter()
            .find(|tool| tool.name == *name)
            .unwrap_or_else(|| panic!("{name} is not registered"));
        let schema = input_schema_for_tool(&tool);
        let properties = schema["properties"].as_object().expect("object schema");
        assert!(
            properties.contains_key("sourceSet"),
            "{name} must publish `sourceSet`"
        );
        assert!(
            properties.contains_key(*legacy),
            "{name} must keep `{legacy}` until its v0.13 removal slice"
        );
        assert_eq!(
            properties.contains_key("metadataPath"),
            *needs_address,
            "{name} publishes `metadataPath` only when it can use one"
        );

        let logical_required = if *needs_address {
            json!(["sourceSet", "metadataPath"])
        } else {
            json!(["sourceSet"])
        };
        let mut forbidden_in_legacy = vec![json!({"required": ["sourceSet"]})];
        if *needs_address {
            forbidden_in_legacy.push(json!({"required": ["metadataPath"]}));
        }
        assert_eq!(
            schema["oneOf"],
            json!([
                {
                    "required": logical_required,
                    "not": {"required": [legacy]}
                },
                {
                    "required": [legacy],
                    "not": {"anyOf": forbidden_in_legacy}
                }
            ]),
            "{name} must publish the two selector branches as mutually exclusive"
        );
        assert_eq!(
            schema["required"],
            json!([]),
            "{name} has no unconditionally required selector"
        );
    }
}

#[test]
fn bridged_readers_refuse_two_selectors_at_once() {
    for (name, legacy, needs_address) in BRIDGED_READERS {
        let tool = tools().into_iter().find(|tool| tool.name == *name).unwrap();
        let mut args = Map::from_iter([
            (legacy.to_string(), json!("src/whatever.xml")),
            ("sourceSet".to_string(), json!("main")),
        ]);
        if *needs_address {
            args.insert("metadataPath".to_string(), json!("Role.Sales"));
        }
        let error = validate_tool_arguments(&tool, &args, false)
            .expect_err("two selectors must be refused before the handler");
        assert!(
            error.contains("selector_conflict"),
            "{name} must refuse two selectors with `selector_conflict`, got: {error}"
        );
    }
}

#[test]
fn bridged_readers_still_refuse_an_empty_call() {
    for (name, _, _) in BRIDGED_READERS {
        let tool = tools().into_iter().find(|tool| tool.name == *name).unwrap();
        assert!(
            validate_tool_arguments(&tool, &Map::new(), false).is_err(),
            "{name} must still refuse a call with no selector at all"
        );
    }
}
```

`input_schema_for_tool` and the three-argument `validate_tool_arguments(tool,
&args, false)` are the helpers this test module already uses; `json!` and
`tools()` are already in scope there.

Path aliases (`formPath`, `Path`, `path`) are folded into the canonical spelling
by `normalize_native_path_aliases` before validation runs, which is why the
branches name only the canonical argument.

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test -p unica-coder bridged_readers -- --test-threads=1
```

Expected: FAIL — `sourceSet`/`metadataPath` are not published and the path is
still `required`. Record the first assertion message.

- [ ] **Step 3: Declare the bridge table once**

Near the argument lists, one table that every later step reads, so the thirteen
tools cannot drift apart:

```rust
/// ADR-0048 bridge: tool name, canonical legacy argument, and whether the
/// logical branch also takes `metadataPath`. `unica.cf.*` selects a
/// configuration root, which has no address, so it publishes `sourceSet` alone.
const BRIDGED_SELECTORS: &[(&str, &str, bool)] = &[
    ("unica.cf.info", "ConfigPath", false),
    ("unica.cf.validate", "ConfigPath", false),
    ("unica.subsystem.info", "SubsystemPath", false),
    ("unica.subsystem.validate", "SubsystemPath", true),
    ("unica.role.info", "RightsPath", true),
    ("unica.role.validate", "RightsPath", true),
    ("unica.form.info", "FormPath", true),
    ("unica.form.validate", "FormPath", true),
    ("unica.dcs.info", "TemplatePath", true),
    ("unica.dcs.validate", "TemplatePath", true),
    ("unica.mxl.info", "TemplatePath", true),
    ("unica.mxl.validate", "TemplatePath", true),
    ("unica.mxl.decompile", "TemplatePath", true),
];

fn bridged_selector(name: &str) -> Option<(&'static str, bool)> {
    BRIDGED_SELECTORS
        .iter()
        .find(|(tool, _, _)| *tool == name)
        .map(|(_, legacy, needs_address)| (*legacy, *needs_address))
}
```

- [ ] **Step 4: Extend the argument lists**

Add `"sourceSet"` to all thirteen argument lists and `"metadataPath"` to the ten
whose third column is `true`. That is `CF_INFO_ARGS`, `ROLE_INFO_ARGS`,
`SUBSYSTEM_INFO_ARGS`, `DCS_INFO_ARGS`, `FORM_INFO_ARGS`, `MXL_INFO_ARGS`, plus
the lists of the six `*.validate` tools and `mxl.decompile`. Where a validator
still draws from the shared `NATIVE_XML_DSL_ARGS`, give it its own narrow list
in the style of the readers above, carrying only the arguments that tool already
consumed plus the selector, and say so in a comment: the list is the ADR-0048
bridge, not a new catch-all.

- [ ] **Step 5: Publish the two branches as mutually exclusive**

In the function that decorates a schema with constraints — the one that already
sets `oneOf` for `unica.source.resources` and `unica.code.patch` — add the
bridge, modelled on the `source.resources` branch pair:

```rust
    if let Some((legacy, needs_address)) = bridged_selector(tool.name) {
        let logical_required = if needs_address {
            json!(["sourceSet", "metadataPath"])
        } else {
            json!(["sourceSet"])
        };
        let mut forbidden_in_legacy = vec![json!({"required": ["sourceSet"]})];
        if needs_address {
            forbidden_in_legacy.push(json!({"required": ["metadataPath"]}));
        }
        schema["oneOf"] = json!([
            {
                "required": logical_required,
                "not": {"required": [legacy]}
            },
            {
                "required": [legacy],
                "not": {"anyOf": forbidden_in_legacy}
            }
        ]);
    }
```

- [ ] **Step 6: Add the exactly-one-of check**

The schema states the contract; the Rust check is what produces the stable code.
Next to `validate_unique_alias_group`:

```rust
/// ADR-0048: a bridged reader accepts exactly one selector. Two at once is a
/// caller mistake, not a precedence question — resolving it silently would hide
/// which selector the answer came from.
fn validate_bridged_selector(tool: &ToolSpec, args: &Map<String, Value>) -> Result<(), String> {
    let Some((legacy, needs_address)) = bridged_selector(tool.name) else {
        return Ok(());
    };
    let logical: &[&str] = if needs_address {
        &["sourceSet", "metadataPath"]
    } else {
        &["sourceSet"]
    };
    if contains_any(args, logical) && args.contains_key(legacy) {
        return Err(format!(
            "selector_conflict: {} accepts either `{}` or `{legacy}`, not both",
            tool.name,
            logical.join("` + `"),
        ));
    }
    Ok(())
}
```

Call it from the same place `validate_unique_alias_group` is called, after
`normalize_native_path_aliases` has folded `formPath`/`Path`/`path` into the
canonical spelling.

- [ ] **Step 7: Make the path optional in the descriptors**

In `operation_descriptors.rs`, change the required-argument list of the thirteen
operations (`cf-info`, `cf-validate`, `role-info`, `role-validate`, `form-info`,
`form-validate`, `dcs-info`, `dcs-validate`, `mxl-info`, `mxl-validate`,
`mxl-decompile`, `subsystem-info`, `subsystem-validate`) from its
`*_PATH_REQUIRED` constant to `EMPTY`; the `oneOf` from Step 5 now carries the
requirement. Leave `source_path_args` and both format policies alone:
`FormatPathPolicy::HandlerResolved` is what routes the logical case in Tasks 4–6.

- [ ] **Step 8: Run tests to verify they pass**

```bash
cargo test -p unica-coder bridged_readers -- --test-threads=1
cargo test -p unica-coder application::tests -- --test-threads=1
```

Expected: PASS. An empty call is now refused by the `oneOf` rather than by a
`required` entry; `bridged_readers_still_refuse_an_empty_call` pins that the
refusal survives the change.

- [ ] **Step 9: Commit**

```bash
git add crates/unica-coder/src/application/tool_contracts.rs \
        crates/unica-coder/src/application/operation_descriptors.rs
git commit -m "feat(mcp): publish the logical selector on bridged readers"
```

---

## Task 4: Bridge `role.info`, `role.validate`, `cf.info`, `cf.validate`

First because both pairs already funnel through an args-taking helper that
`handler_resolved_format_paths` also calls, so one edit per pair reaches both
the handler and the format guard.

**Files:**

- Modify: `crates/unica-coder/src/infrastructure/native_operations/common.rs:1139-1148` (`resolve_role_read_rights_path`)
- Modify: `crates/unica-coder/src/infrastructure/native_operations/common.rs:1212-1217` (`resolve_cf_read_config_path`)
- Test: `crates/unica-coder/src/infrastructure/native_operations/role.rs` `mod tests`, `crates/unica-coder/src/infrastructure/native_operations/cf.rs` `mod tests`

**Interfaces:**

- Consumes: `logical_selection`, `AttachedResource` from Task 2; the published
  arguments from Task 3.
- Produces: the pattern Tasks 5 and 6 repeat verbatim.

- [ ] **Step 1: Write the failing tests**

In `role.rs` tests:

```rust
#[test]
fn role_info_answers_identically_for_a_logical_and_a_physical_selector() {
    let context = role_bridge_fixture("role-info-bridge");
    let physical = analyze_role_info(
        &Map::from_iter([(
            "RightsPath".to_string(),
            Value::String("src/Roles/Sales/Ext/Rights.xml".to_string()),
        )]),
        &context,
    );
    let logical = analyze_role_info(
        &Map::from_iter([
            ("sourceSet".to_string(), Value::String("main".to_string())),
            ("metadataPath".to_string(), Value::String("Role.Sales".to_string())),
        ]),
        &context,
    );
    assert!(logical.outcome.ok, "logical call failed: {}", logical.outcome.summary);
    assert_eq!(logical.data, physical.data);
}

#[test]
fn role_info_accepts_the_russian_kind_alias() {
    let context = role_bridge_fixture("role-info-alias");
    let logical = analyze_role_info(
        &Map::from_iter([
            ("sourceSet".to_string(), Value::String("main".to_string())),
            ("metadataPath".to_string(), Value::String("Роль.Sales".to_string())),
        ]),
        &context,
    );
    assert!(logical.outcome.ok, "{}", logical.outcome.summary);
}

#[test]
fn role_info_refuses_an_address_that_is_not_a_role() {
    let context = role_bridge_fixture("role-info-wrong-kind");
    let outcome = analyze_role_info(
        &Map::from_iter([
            ("sourceSet".to_string(), Value::String("main".to_string())),
            ("metadataPath".to_string(), Value::String("Catalog.Items".to_string())),
        ]),
        &context,
    )
    .outcome;
    assert!(!outcome.ok);
    assert!(outcome.summary.contains("target_kind_unsupported"), "{}", outcome.summary);
}
```

In `cf.rs` tests — the shape differs because a configuration root has no address:

```rust
#[test]
fn cf_info_answers_identically_for_a_source_set_and_a_config_path() {
    let context = cf_bridge_fixture("cf-info-bridge");
    let physical = analyze_cf_info(
        &Map::from_iter([("ConfigPath".to_string(), Value::String("src".to_string()))]),
        &context,
    );
    let logical = analyze_cf_info(
        &Map::from_iter([("sourceSet".to_string(), Value::String("main".to_string()))]),
        &context,
    );
    assert!(logical.outcome.ok, "logical call failed: {}", logical.outcome.summary);
    assert_eq!(logical.data, physical.data);
}

#[test]
fn cf_info_reports_an_unknown_source_set_as_such() {
    let context = cf_bridge_fixture("cf-info-unknown-set");
    let outcome = analyze_cf_info(
        &Map::from_iter([("sourceSet".to_string(), Value::String("nope".to_string()))]),
        &context,
    )
    .outcome;
    assert!(!outcome.ok);
    assert!(outcome.summary.contains("source_set_unknown"), "{}", outcome.summary);
}
```

`role_bridge_fixture` and `cf_bridge_fixture` both call Task 2's
`write_descriptor` and `register_children` against a workspace carrying
`v8project.yaml` with one `CONFIGURATION` source set named `main` at `src`. Write
one shared builder in Task 4 and reuse it in Tasks 5 and 6 — the per-tool
wrappers differ only in which descriptors and `Ext/` files they add.

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test -p unica-coder role_info_answers_identically -- --test-threads=1
```

Expected: FAIL — `analyze_role_info` reports a missing `RightsPath`, because
`resolve_role_read_rights_path` does not read the logical selector.

- [ ] **Step 3: Add the logical branch to both helpers**

In `common.rs`:

```rust
pub(crate) fn resolve_role_read_rights_path(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<PathBuf, String> {
    if let Some(selection) = logical_selection(args, context, AttachedResource::Rights, &["Role"]) {
        return selection
            .map(|selection| selection.resource_path)
            .map_err(|failure| failure.to_string());
    }
    let raw = required_path(args, RIGHTS_PATH, "RightsPath")?;
    Ok(resolve_role_validate_rights_path(absolutize(raw, &context.cwd)))
}

pub(crate) fn resolve_cf_read_config_path(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<PathBuf, String> {
    if let Some(selection) =
        logical_selection(args, context, AttachedResource::ConfigurationRoot, &[])
    {
        return selection
            .map(|selection| selection.resource_path)
            .map_err(|failure| failure.to_string());
    }
    resolve_configuration_read_path(args, CF_PATH, "ConfigPath", context)
}
```

Import `logical_selection` and `AttachedResource` at the top of `common.rs`.

- [ ] **Step 4: Run tests to verify they pass**

```bash
cargo test -p unica-coder role_info -- --test-threads=1
cargo test -p unica-coder cf_info -- --test-threads=1
cargo test -p unica-coder format_guard -- --test-threads=1
```

Expected: PASS. `format_guard` matters here: `handler_resolved_format_paths`
already routes `role-info | role-validate` and `cf-info | cf-validate` through
these two helpers, so the guard picks up the logical case with no arm change.

- [ ] **Step 5: Commit**

```bash
git add crates/unica-coder/src/infrastructure/native_operations/common.rs \
        crates/unica-coder/src/infrastructure/native_operations/role.rs \
        crates/unica-coder/src/infrastructure/native_operations/cf.rs
git commit -m "feat(role,cf): accept a logical address in the read selectors"
```

---

## Task 5: Bridge `mxl.info`, `mxl.validate`, `mxl.decompile`, `dcs.info`, `dcs.validate`

**Files:**

- Modify: `crates/unica-coder/src/infrastructure/native_operations/mxl.rs:1892` (`resolve_mxl_info_path`), `:1916` (`resolve_mxl_validate_path`), `:843` (inline in `decompile_mxl`)
- Modify: `crates/unica-coder/src/infrastructure/native_operations/dcs.rs:1593` (`inspect_dcs_info_path`), `resolve_dcs_validate_path`
- Modify: `crates/unica-coder/src/infrastructure/format_guard.rs:1029-1049`
- Test: the `tests` modules of `mxl.rs` and `dcs.rs`

**Interfaces:**

- Consumes: `logical_selection`, `AttachedResource::Template` from Task 2.
- Produces: `resolve_mxl_decompile_path(args, context) -> Result<PathBuf, String>`,
  a new named helper replacing the inline `required_path` in `decompile_mxl`.

- [ ] **Step 1: Write the failing tests**

One identical-answer test per tool. Written out for `mxl.info`; the other four
are the same body with the handler swapped for `validate_mxl`, `decompile_mxl`,
`analyze_dcs_info` and `validate_dcs`:

```rust
#[test]
fn mxl_info_answers_identically_for_a_logical_and_a_physical_selector() {
    let context = template_bridge_fixture("mxl-info-bridge");
    let physical = analyze_mxl_info(
        &Map::from_iter([(
            "TemplatePath".to_string(),
            Value::String("src/Reports/Sales/Templates/Main/Ext/Template.xml".to_string()),
        )]),
        &context,
    );
    let logical = analyze_mxl_info(
        &Map::from_iter([
            ("sourceSet".to_string(), Value::String("main".to_string())),
            (
                "metadataPath".to_string(),
                Value::String("Report.Sales.Template.Main".to_string()),
            ),
        ]),
        &context,
    );
    assert!(logical.outcome.ok, "logical call failed: {}", logical.outcome.summary);
    assert_eq!(logical.data, physical.data);
}
```

Plus the case that only a real dump reveals:

```rust
#[test]
fn dcs_info_reports_a_binary_template_as_an_absent_resource_not_a_missing_target() {
    // A `TemplateType` other than DataCompositionSchema writes `Template.bin`
    // or `Template.txt`; the descriptor is present and addressable either way.
    let context = template_bridge_fixture("dcs-binary-template");
    let outcome = analyze_dcs_info(
        &Map::from_iter([
            ("sourceSet".to_string(), Value::String("main".to_string())),
            (
                "metadataPath".to_string(),
                Value::String("CommonTemplate.Logo".to_string()),
            ),
        ]),
        &context,
    )
    .outcome;
    assert!(!outcome.ok);
    assert!(
        outcome.summary.contains("resource_absent"),
        "a proven object without the requested resource is not a missing target: {}",
        outcome.summary
    );
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test -p unica-coder mxl_info -- --test-threads=1
cargo test -p unica-coder dcs_info -- --test-threads=1
```

Expected: FAIL with the missing-`TemplatePath` error. Record it.

- [ ] **Step 3: Add the logical branch to the five resolution points**

The same four-line prologue in each, with `TEMPLATE_KINDS` defined once per file:

```rust
const TEMPLATE_KINDS: &[&str] = &["Template", "CommonTemplate"];

pub(crate) fn resolve_mxl_info_path(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<PathBuf, String> {
    if let Some(selection) =
        logical_selection(args, context, AttachedResource::Template, TEMPLATE_KINDS)
    {
        return selection
            .map(|selection| selection.resource_path)
            .map_err(|failure| failure.to_string());
    }
    // …existing body unchanged…
}
```

For `decompile_mxl`, lift the inline `required_path` into
`resolve_mxl_decompile_path(args, context)` with the same prologue, and call it
from the handler. For `inspect_dcs_info_path`, take the logical branch before the
existing `required_path` and return a `DcsInfoPathInspection` whose `resolution`
is the resolved path and whose `dependencies` are empty — the resolver already
proved the file, so the `Ext/Template.xml` probing below is for the path case
only.

- [ ] **Step 4: Teach the format guard the two missing arms**

In `handler_resolved_format_paths`, `mxl-info` and `dcs-info` currently fall to
`_ => None` and use the raw path argument, which is absent in a logical call.
Add:

```rust
        "mxl-info" => resolve_mxl_info_path(args, context).ok(),
        "dcs-info" => dcs_info_format_path(args, context),
```

where `dcs_info_format_path` returns `inspect_dcs_info_path(args, context).resolution.ok()`.

- [ ] **Step 5: Run tests to verify they pass**

```bash
cargo test -p unica-coder mxl -- --test-threads=1
cargo test -p unica-coder dcs -- --test-threads=1
cargo test -p unica-coder format_guard -- --test-threads=1
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/unica-coder/src/infrastructure/native_operations/mxl.rs \
        crates/unica-coder/src/infrastructure/native_operations/dcs.rs \
        crates/unica-coder/src/infrastructure/format_guard.rs
git commit -m "feat(mxl,dcs): accept a logical template address"
```

---

## Task 6: Bridge `form.info`, `form.validate`, `subsystem.info`, `subsystem.validate`

**Files:**

- Modify: `crates/unica-coder/src/infrastructure/native_operations/form.rs:182`, `:1681`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/subsystem.rs:318`, `:989`, `:2253`
- Modify: `crates/unica-coder/src/infrastructure/format_guard.rs`
- Test: the `tests` modules of `form.rs` and `subsystem.rs`

**Interfaces:**

- Consumes: `logical_selection`, `AttachedResource::{Form, Descriptor}` from Task 2.
- Produces: `resolve_form_read_path(args, context) -> Result<PathBuf, String>`
  and `resolve_subsystem_read_path(args, context) -> Result<PathBuf, String>`,
  the single args→path entry point of each pair.

- [ ] **Step 1: Write the failing tests**

One identical-answer test per tool, same body as Task 5's, with these pairs:

| Handler | Physical | Logical |
| --- | --- | --- |
| `analyze_form_info_with_data` | `FormPath: "src/Catalogs/Items/Forms/Order/Ext/Form.xml"` | `Catalog.Items.Form.Order` |
| `validate_form` | same | same |
| `analyze_subsystem_info` | `SubsystemPath: "src/Subsystems/Parent.xml"` | `Subsystem.Parent` |
| `validate_subsystem` | same | same |

Plus the two cases specific to this pair:

```rust
#[test]
fn form_info_reads_a_common_form_and_a_nested_form_by_address() {
    let context = form_bridge_fixture("form-bridge");
    for address in ["CommonForm.Settings", "Catalog.Items.Form.Order"] {
        let logical = analyze_form_info_with_data(
            &Map::from_iter([
                ("sourceSet".to_string(), Value::String("main".to_string())),
                ("metadataPath".to_string(), Value::String(address.to_string())),
            ]),
            &context,
        );
        assert!(logical.outcome.ok, "{address}: {}", logical.outcome.summary);
    }
}

#[test]
fn subsystem_info_reads_the_whole_registered_tree_from_the_source_set_alone() {
    // `sourceSet` with no `metadataPath` is the source root — the same answer
    // the `Subsystems` directory path gives today.
    let context = subsystem_bridge_fixture("subsystem-root");
    let by_path = analyze_subsystem_info(
        &Map::from_iter([(
            "SubsystemPath".to_string(),
            Value::String("src/Subsystems".to_string()),
        )]),
        &context,
    );
    let by_set = analyze_subsystem_info(
        &Map::from_iter([("sourceSet".to_string(), Value::String("main".to_string()))]),
        &context,
    );
    assert!(by_set.outcome.ok, "{}", by_set.outcome.summary);
    assert_eq!(by_set.data, by_path.data);
}

#[test]
fn subsystem_info_refuses_a_nested_address_instead_of_guessing() {
    // ADR-0036 keeps the nested node outside `unica.source.*`, so the grammar
    // must refuse it rather than resolve a plausible-looking path.
    let context = subsystem_bridge_fixture("subsystem-nested");
    let outcome = analyze_subsystem_info(
        &Map::from_iter([
            ("sourceSet".to_string(), Value::String("main".to_string())),
            (
                "metadataPath".to_string(),
                Value::String("Subsystem.Parent.Child".to_string()),
            ),
        ]),
        &context,
    )
    .outcome;
    assert!(!outcome.ok);
    assert!(outcome.summary.contains("target_kind_unsupported"), "{}", outcome.summary);
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test -p unica-coder form_info -- --test-threads=1
cargo test -p unica-coder subsystem_info -- --test-threads=1
```

Expected: FAIL with the missing-argument errors.

- [ ] **Step 3: Introduce the two named resolvers**

In `form.rs`:

```rust
const FORM_KINDS: &[&str] = &["Form", "CommonForm"];

pub(crate) fn resolve_form_read_path(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<PathBuf, String> {
    if let Some(selection) = logical_selection(args, context, AttachedResource::Form, FORM_KINDS) {
        return selection
            .map(|selection| selection.resource_path)
            .map_err(|failure| failure.to_string());
    }
    let raw_path = required_path(args, FORM_PATH, "FormPath")?;
    Ok(resolve_form_info_path(absolutize(raw_path, &context.cwd)))
}
```

Replace the two inline `required_path(args, FORM_PATH, "FormPath")` sites
(`:182` in `validate_form`, `:1681` in `analyze_form_info_with_data`) with a call
to it. Both sites currently call `resolve_form_info_path` on the absolutised
path a line or two later; fold that into the helper as shown and drop it from the
call sites.

In `subsystem.rs`, `AttachedResource::Descriptor` is what the reader wants
(`Subsystems/<S>.xml` *is* the subsystem XML), and `sourceSet` alone yields the
source root, which `analyze_subsystem_info` already treats as "the whole tree":

```rust
const SUBSYSTEM_KINDS: &[&str] = &["Subsystem"];

pub(crate) fn resolve_subsystem_read_path(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<PathBuf, String> {
    if args.get("sourceSet").is_some() && args.get("metadataPath").is_none() {
        // The registered tree of the whole set: today's `Subsystems` directory.
        let selection = logical_selection(args, context, AttachedResource::ConfigurationRoot, &[])
            .expect("sourceSet is present")
            .map_err(|failure| failure.to_string())?;
        let root = selection
            .resource_path
            .parent()
            .ok_or_else(|| "the source root has no directory".to_string())?;
        return Ok(root.join("Subsystems"));
    }
    if let Some(selection) =
        logical_selection(args, context, AttachedResource::Descriptor, SUBSYSTEM_KINDS)
    {
        return selection
            .map(|selection| selection.resource_path)
            .map_err(|failure| failure.to_string());
    }
    let raw_path = required_path(args, SUBSYSTEM_PATH, "SubsystemPath")?;
    Ok(absolutize(raw_path, &context.cwd))
}
```

Replace the three `required_path(args, SUBSYSTEM_PATH, "SubsystemPath")` sites
(`:318` in `subsystem_read_format_dependency_paths`, `:989` in
`validate_subsystem`, `:2253` in the info path) with calls to it, keeping each
site's existing follow-up (`resolve_subsystem_validate_xml`, and so on).

The whole-tree branch is reachable only from `subsystem.info`: Task 3 publishes
`metadataPath` as required for `subsystem.validate`, so a `sourceSet`-only call
to the validator is refused by the schema before the handler runs. Keep the
branch in the shared helper anyway — the validator's own follow-up
(`resolve_subsystem_validate_xml`) rejects a directory, so a future schema change
cannot silently produce a wrong answer here.

- [ ] **Step 4: Update the format-guard arms**

Replace the two path-only arms with the args-taking resolvers:

```rust
        "form-info" | "form-validate" => resolve_form_read_path(args, context).ok(),
        "subsystem-info" | "subsystem-validate" => {
            resolve_subsystem_read_path(args, context).ok()
        }
```

- [ ] **Step 5: Run tests to verify they pass**

```bash
cargo test -p unica-coder form -- --test-threads=1
cargo test -p unica-coder subsystem -- --test-threads=1
cargo test -p unica-coder format_guard -- --test-threads=1
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/unica-coder/src/infrastructure/native_operations/form.rs \
        crates/unica-coder/src/infrastructure/native_operations/subsystem.rs \
        crates/unica-coder/src/infrastructure/format_guard.rs
git commit -m "feat(form,subsystem): accept a logical address in the read selectors"
```

---

## Task 7: Synchronize the public surface

**Files:**

- Modify: `plugins/unica/skills/{cf-info,cf-validate,role-info,role-validate,form-info,form-validate,dcs-info,dcs-validate,mxl-info,mxl-validate,mxl-decompile,subsystem-info,subsystem-validate}/SKILL.md`
- Modify: `plugins/unica/README.md`
- Modify: `spec/architecture/invariants.md`
- Modify: `spec/architecture/tool-surface.md` (regenerated), `spec/architecture/tool-surface-review.json`
- Modify: `spec/acceptance/logical-source-addressing-and-resource-access.md`
- Modify: `tests/ci/test_unica_skills.py` if a skill assertion keys on the path argument

**Interfaces:**

- Consumes: the shipped behaviour of Tasks 1–6.
- Produces: nothing consumed by later tasks.

- [ ] **Step 1: Adjust the CI assertions first and observe the failures**

```bash
/opt/homebrew/bin/python3.12 -m unittest tests.ci.test_unica_skills tests.ci.test_architecture_registry tests.ci.test_unica_mcp_script_parity
```

Record what fails before changing prose.

- [ ] **Step 2: Add the derived rule to the invariant registry**

A new record in `spec/architecture/invariants.md`, Rule in Russian, one
statement:

```markdown
### INV-SOURCE-READER-SELECTOR — Читатель принимает ровно один селектор цели

- **Rule:** Предметный читатель в переходном состоянии публикует логический
  селектор `sourceSet` с необязательным `metadataPath` и своё файловое поле,
  принимает ровно один из них, отклоняет одновременную передачу обоих стабильным
  `selector_conflict` до вызова обработчика и отвечает на логический вызов теми
  же типизированными данными, что на файловый.
- **Decision:** ADR-0048
- **Check:** `ci-test` — `crates/unica-coder/src/application/tool_contracts.rs`
- **Check:** `ci-test` — `crates/unica-coder/src/infrastructure/native_operations/logical_selector.rs`
- **Scope:** source, runtime
```

- [ ] **Step 3: Update the thirteen skills**

Each `SKILL.md` gains a logical `tools/call` example beside its path example and
one sentence naming where an address comes from: `unica.project.map` for the
source-set name, `unica.source.resolve` for the address, `unica.source.locate`
to convert a path found by other means. Examples must execute as deterministic
reads (`INV-SKILL-EXECUTABLE-EXAMPLES`) — readers take no `dryRun` (ADR-0044).

- [ ] **Step 4: Note the transitional state in the package README**

`plugins/unica/README.md` gains a section stating that the thirteen readers
accept either selector, that the physical field is removed per-tool in `v0.13`,
and that a call may not pass both.

- [ ] **Step 5: Add the acceptance rows**

In `spec/acceptance/logical-source-addressing-and-resource-access.md`, under the
address matrix:

| Случай | Ожидаемое доказательство | Проверка |
| --- | --- | --- |
| Прикреплённый ресурс по логическому адресу | Дескриптор `<…>/<Stem>.xml` даёт ресурс `<…>/<Stem>/Ext/<Имя>`; ссылка, выход за корень и нерегулярный файл отклонены раздельно | `cargo test -p unica-coder logical_selector -- --test-threads=1` |
| Мост не меняет ответ | Логический и файловый вызов одного объекта возвращают одинаковые типизированные данные у всех тринадцати читателей | `cargo test -p unica-coder answers_identically -- --test-threads=1` |
| Доказанный объект без ресурса | Двоичный или текстовый макет даёт `resource_absent`, а не `target_not_found` | `cargo test -p unica-coder dcs_info -- --test-threads=1` |
| Два селектора сразу | Одновременные `metadataPath` и файловое поле отклоняются `selector_conflict` до обработчика | `cargo test -p unica-coder application::tests -- --test-threads=1` |
| Политика вида цели покрывает корень | `ModuleOnly` отклоняет корень набора, `Any` его разрешает, ручка не расширяется при повторной проверке | `cargo test -p unica-coder platform_xml_source_targets -- --test-threads=1` |

`cargo test` filters on test *names*, not modules, so every identical-answer test
in Tasks 4–6 must contain the substring `answers_identically` for this row to
select all thirteen. Count them when running it: thirteen tests, not fewer.

- [ ] **Step 6: Regenerate the ledger and run the CI suite**

```bash
/opt/homebrew/bin/python3.12 scripts/ci/generate-tool-surface.py
/opt/homebrew/bin/python3.12 -m unittest tests.ci.test_unica_skills tests.ci.test_architecture_registry tests.ci.test_design_documents tests.ci.test_unica_mcp_script_parity
/opt/homebrew/bin/python3.12 scripts/ci/check-architecture-sync.py --base origin/main
```

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add plugins/unica spec tests
git commit -m "docs(architecture): record the reader selector bridge"
```

---

## Task 8: Prove it end to end, run the regression suite, close #273

**Files:**

- Modify: `tests/ci/test_unica_mcp_smoke.py`
- Modify: `scripts/ci/smoke-unica-mcp.py`

**Interfaces:**

- Consumes: everything above.
- Produces: the merge-ready state.

- [ ] **Step 1: Add the smoke assertions and record the failure**

Through JSON-RPC `tools/call`: resolve an object with `unica.source.resolve`,
feed the returned address to `unica.form.info` and `unica.role.info`, and assert
both succeed with no physical path anywhere in either request. Assert that
passing both `RightsPath` and `metadataPath` to `unica.role.info` fails with
`selector_conflict`.

```bash
/opt/homebrew/bin/python3.12 -m unittest tests.ci.test_unica_mcp_smoke
```

- [ ] **Step 2: Complete any missing wiring**

Without weakening a lower-level guard. If the smoke reveals a gap, fix it at the
seam, not by relaxing `prove_regular_file`.

- [ ] **Step 3: Run the full suite**

```bash
cargo test -p unica-coder -- --test-threads=1
/opt/homebrew/bin/python3.12 -m unittest discover -s tests/ci -t .
/opt/homebrew/bin/python3.12 -m unittest discover -s tests/dev -t .
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

Expected: all PASS.

- [ ] **Step 4: Verify the tree**

```bash
git diff --check
git status --short
```

Expected: clean. Confirm no published schema lost a path argument — this PR adds
selectors, it removes none.

- [ ] **Step 5: Commit and open the PR**

```bash
git add tests scripts
git commit -m "test(source): prove the logical reader bridge end to end"
```

The PR body closes #272 for the read surface and #299, and states that #273 was
verified already delivered by
`source_resources_object_self_snapshot_returns_the_descriptor` and
`source_resources_report_an_unknown_address_as_a_missing_target` — with a comment
on #273 naming those tests before closing it.

---

## Risks

- **The identical-answer assertion is the whole safety net.** If a reader's typed
  data embeds a physical path, the logical and path calls will differ for a
  legitimate reason. When that happens, compare the data with that field
  excluded and record in the test why — do not weaken the comparison silently.
- **`subsystem.info` root semantics.** `sourceSet` alone maps to
  `<root>/Subsystems`, a directory. That is a real path handed to a handler that
  expects one, but it is not proven the way `prove_regular_file` proves a file.
  Task 6 must assert the directory is inside the proven source root and is not a
  link.
- **The format guard is a second consumer.** Every helper changed in Tasks 4–6 is
  called by `handler_resolved_format_paths`. Changing a helper without running
  `cargo test -p unica-coder format_guard` can silently drop a format dependency
  and let a wrong-format dump through.
- **`Template.xml` is not the only template file.** 150 of 775 templates in the
  reference dump are `.bin` and 30 are `.txt`. The `resource_absent` code is what
  keeps that from reading as "no such object".
- **Argument-list churn.** Six validators currently draw from the shared
  `NATIVE_XML_DSL_ARGS`. Giving each a narrow list is correct but is a second,
  quieter contract change; keep it to adding the two selectors plus the arguments
  that tool already consumed, and let `bridged_readers_publish_both_selectors`
  pin the result.
