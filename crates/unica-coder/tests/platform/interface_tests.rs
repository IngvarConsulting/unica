use super::*;
use crate::application::UnicaApplication;
use serde_json::{Map, Value};
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};
use unica_source_contract::workspace::WorkspaceContext;
use unica_workspace::compile_transaction::{with_commit_failpoint, CommitFailpoint};
use unica_workspace::single_file_publisher::with_before_commit_hook;

fn temp_context(name: &str) -> WorkspaceContext {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("unica-interface-{name}-{nanos}"));
    fs::create_dir_all(&root).unwrap();
    WorkspaceContext {
        cwd: root.clone(),
        workspace_root: root.clone(),
        cache_root: root.join(".build").join("unica"),
        workspace_epoch: 1,
    }
}

fn command_interface_document(namespace: &str, body: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<CommandInterface xmlns=\"{namespace}\" xmlns:xr=\"{INTERFACE_XR_NS}\" version=\"2.20\">\n\
{body}\n\
</CommandInterface>"
    )
}

fn hide_args(ci_path: &str, create_if_missing: bool) -> Map<String, Value> {
    Map::from_iter([
        ("CIPath".to_string(), Value::String(ci_path.to_string())),
        ("Operation".to_string(), Value::String("hide".to_string())),
        (
            "Value".to_string(),
            Value::String("Catalog.Products.StandardCommand.OpenList".to_string()),
        ),
        (
            "CreateIfMissing".to_string(),
            Value::Bool(create_if_missing),
        ),
        ("NoValidate".to_string(), Value::Bool(true)),
    ])
}

fn write_command_interface(context: &WorkspaceContext, ci_rel: &str, text: &str) -> PathBuf {
    let ci_path = context.cwd.join(ci_rel);
    fs::create_dir_all(ci_path.parent().unwrap()).unwrap();
    fs::write(&ci_path, text.as_bytes()).unwrap();
    ci_path
}

fn subsystem_document(name: &str, include_help: &str) -> String {
    format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" xmlns:v8=\"http://v8.1c.ru/8.1/data/core\" version=\"2.20\">\n\
\t<Subsystem uuid=\"11111111-2222-4333-8444-555555555555\">\n\
\t\t<Properties>\n\
\t\t\t<Name>{name}</Name>\n\
\t\t\t<Synonym/>\n\
\t\t\t<Comment/>\n\
\t\t\t<IncludeHelpInContents>{include_help}</IncludeHelpInContents>\n\
\t\t\t<IncludeInCommandInterface>true</IncludeInCommandInterface>\n\
\t\t\t<UseOneCommand>false</UseOneCommand>\n\
\t\t\t<Explanation/>\n\
\t\t\t<Picture/>\n\
\t\t\t<Content/>\n\
\t\t</Properties>\n\
\t\t<ChildObjects/>\n\
\t</Subsystem>\n\
</MetaDataObject>"
        )
}

fn write_valid_subsystem_owner(context: &WorkspaceContext, ci_rel: &str) -> (PathBuf, Vec<u8>) {
    let ci_path = context.cwd.join(ci_rel);
    let object_dir = ci_path.parent().unwrap().parent().unwrap();
    let owner_path = object_dir.with_extension("xml");
    let name = owner_path.file_stem().unwrap().to_str().unwrap();
    let bytes = subsystem_document(name, "true").into_bytes();
    fs::create_dir_all(owner_path.parent().unwrap()).unwrap();
    fs::write(&owner_path, &bytes).unwrap();
    (owner_path, bytes)
}

fn configuration_owner_document(include_help: &str) -> String {
    format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.20\">\n\
\t<Configuration uuid=\"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa\">\n\
\t\t<Properties><Name>Demo</Name><IncludeHelpInContents>{include_help}</IncludeHelpInContents></Properties>\n\
\t\t<ChildObjects><Subsystem>AuditSubsystem</Subsystem></ChildObjects>\n\
\t</Configuration>\n\
</MetaDataObject>"
        )
}

#[test]
fn create_if_missing_does_not_leave_file_after_failed_operation() {
    let context = temp_context("create-if-missing-error");
    let ci_rel = "src/Subsystems/NewSales/Ext/CommandInterface.xml";
    let ci_path = context.cwd.join(ci_rel);
    write_valid_subsystem_owner(&context, ci_rel);
    let mut args = Map::new();
    args.insert("CIPath".to_string(), Value::String(ci_rel.to_string()));
    args.insert(
        "Operation".to_string(),
        Value::String("subsystem-order".to_string()),
    );
    args.insert("Value".to_string(), Value::String("[]".to_string()));
    args.insert("CreateIfMissing".to_string(), Value::Bool(true));
    args.insert("NoValidate".to_string(), Value::Bool(true));

    let outcome = edit_interface(&args, &context);

    assert!(!outcome.ok, "{outcome:?}");
    assert!(
        !ci_path.exists(),
        "partial file left at {}",
        ci_path.display()
    );
    let _ = fs::remove_dir_all(&context.workspace_root);
}

#[test]
fn create_if_missing_hide_show_create_valid_command_interface() {
    for (operation, expected_common) in [("hide", "false"), ("show", "true")] {
        let context = temp_context(&format!("create-if-missing-{operation}"));
        let ci_rel = format!("src/Subsystems/New{operation}/Ext/CommandInterface.xml");
        let ci_path = context.cwd.join(&ci_rel);
        write_valid_subsystem_owner(&context, &ci_rel);
        let mut args = Map::new();
        args.insert("CIPath".to_string(), Value::String(ci_rel));
        args.insert(
            "Operation".to_string(),
            Value::String(operation.to_string()),
        );
        args.insert(
            "Value".to_string(),
            Value::String("Catalog.Products.StandardCommand.OpenList".to_string()),
        );
        args.insert("CreateIfMissing".to_string(), Value::Bool(true));

        let execution = edit_interface_with_data(&args, &context);
        let outcome = &execution.outcome;

        assert!(outcome.ok, "{operation}: {outcome:?}");
        assert!(
            execution.data.as_ref().is_some_and(|data| data.validated),
            "{operation}: validation did not run successfully: {outcome:?}"
        );
        let text = fs::read_to_string(&ci_path).unwrap();
        assert!(text.contains("<CommandsVisibility>"), "{operation}: {text}");
        assert!(
            text.contains(&format!("<xr:Common>{expected_common}</xr:Common>")),
            "{operation}: {text}"
        );
        let _ = fs::remove_dir_all(&context.workspace_root);
    }
}

#[test]
pub(crate) fn repeated_interface_edit_preserves_identity_but_reports_attempted_update() {
    use unica_workspace::test_support::file_identity_for_test;

    let context = temp_context("repeat-noop");
    let ci_rel = "src/Subsystems/Sales/Ext/CommandInterface.xml";
    write_valid_subsystem_owner(&context, ci_rel);
    let ci_path = write_command_interface(
        &context,
        ci_rel,
        &command_interface_document(INTERFACE_CI_NS, ""),
    );
    let args = hide_args(ci_rel, false);

    let first = edit_interface(&args, &context);
    assert!(first.ok, "{first:?}");
    assert!(!first.changes.is_empty(), "{first:?}");
    let after = fs::read(&ci_path).unwrap();
    let identity = file_identity_for_test(&ci_path).unwrap();

    let repeated = edit_interface(&args, &context);
    assert!(repeated.ok, "{repeated:?}");
    assert_eq!(repeated.changes.len(), 1, "{repeated:?}");
    assert!(repeated.summary.contains("added 0, removed 0, modified 0"));
    assert_eq!(fs::read(&ci_path).unwrap(), after);
    assert_eq!(file_identity_for_test(&ci_path).unwrap(), identity);
    fs::remove_dir_all(context.workspace_root).unwrap();
}

#[test]
fn interface_edit_rejects_invalid_existing_document_without_mutating_preimage() {
    let invalid_cases = [
            (
                "bogus-direct-child",
                command_interface_document(INTERFACE_CI_NS, "\t<Bogus/>"),
            ),
            (
                "invalid-common",
                command_interface_document(
                    INTERFACE_CI_NS,
                    "\t<CommandsVisibility>\n\
\t\t<Command name=\"Catalog.Existing.StandardCommand.OpenList\"><Visibility><xr:Common>banana</xr:Common></Visibility></Command>\n\
\t</CommandsVisibility>",
                ),
            ),
            (
                "malformed",
                format!(
                    "<CommandInterface xmlns=\"{INTERFACE_CI_NS}\"><CommandsVisibility>"
                ),
            ),
            (
                "wrong-root",
                format!("<WrongRoot xmlns=\"{INTERFACE_CI_NS}\"/>"),
            ),
            (
                "wrong-root-namespace",
                command_interface_document("urn:not-command-interface", ""),
            ),
            (
                "wrong-section-namespace",
                command_interface_document(
                    INTERFACE_CI_NS,
                    "\t<bad:CommandsVisibility xmlns:bad=\"urn:not-command-interface\"/>",
                ),
            ),
            (
                "wrong-section-order",
                command_interface_document(
                    INTERFACE_CI_NS,
                    "\t<CommandsPlacement/>\n\t<CommandsVisibility/>",
                ),
            ),
            (
                "duplicate-section",
                command_interface_document(
                    INTERFACE_CI_NS,
                    "\t<CommandsVisibility/>\n\t<CommandsVisibility/>",
                ),
            ),
        ];

    for (case, source) in invalid_cases {
        let context = temp_context(case);
        let ci_rel = "src/Subsystems/Existing/Ext/CommandInterface.xml";
        write_valid_subsystem_owner(&context, ci_rel);
        let ci_path = write_command_interface(&context, ci_rel, &source);
        let before = fs::read(&ci_path).unwrap();

        let validation = validate_interface(
            &Map::from_iter([("CIPath".to_string(), Value::String(ci_rel.to_string()))]),
            &context,
        );
        assert!(!validation.ok, "{case}: {validation:?}");
        assert!(!validation.errors.is_empty(), "{case}: {validation:?}");

        let outcome = edit_interface(&hide_args(ci_rel, false), &context);

        assert!(!outcome.ok, "{case}: {outcome:?}");
        assert!(!outcome.errors.is_empty(), "{case}: {outcome:?}");
        assert_eq!(fs::read(&ci_path).unwrap(), before, "{case}");
        assert!(outcome.changes.is_empty(), "{case}: {outcome:?}");
        assert!(outcome.artifacts.is_empty(), "{case}: {outcome:?}");
        let _ = fs::remove_dir_all(&context.workspace_root);
    }
}

#[test]
fn public_interface_edit_rejects_invalid_subsystem_owner_without_mutating_any_file() {
    let context = temp_context("public-invalid-subsystem-owner");
    fs::write(
        context.cwd.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
    )
    .unwrap();
    fs::create_dir_all(context.cwd.join("src/Subsystems")).unwrap();
    let configuration_path = context.cwd.join("src/Configuration.xml");
    let configuration = configuration_owner_document("false").into_bytes();
    fs::write(&configuration_path, &configuration).unwrap();
    let owner_path = context.cwd.join("src/Subsystems/AuditSubsystem.xml");
    let invalid_owner = subsystem_document("AuditSubsystem", "banana").into_bytes();
    fs::write(&owner_path, &invalid_owner).unwrap();
    let ci_rel = "src/Subsystems/AuditSubsystem/Ext/CommandInterface.xml";
    let ci_path = write_command_interface(
        &context,
        ci_rel,
        &command_interface_document(INTERFACE_CI_NS, ""),
    );
    let ci_before = fs::read(&ci_path).unwrap();
    let args = Map::from_iter([
        (
            "cwd".to_string(),
            Value::String(context.cwd.display().to_string()),
        ),
        ("CIPath".to_string(), Value::String(ci_rel.to_string())),
        ("Operation".to_string(), Value::String("hide".to_string())),
        (
            "Value".to_string(),
            Value::String("Catalog.Products.StandardCommand.OpenList".to_string()),
        ),
        ("dryRun".to_string(), Value::Bool(false)),
        ("confirm".to_string(), Value::Bool(true)),
    ]);

    let outcome = UnicaApplication::new()
        .call_tool("unica.interface.edit", &args)
        .unwrap();

    assert!(!outcome.ok, "{outcome:?}");
    let errors = outcome.errors.join("\n");
    assert!(errors.contains("IncludeHelpInContents"), "{outcome:?}");
    assert!(errors.contains("banana"), "{outcome:?}");
    assert_eq!(fs::read(&configuration_path).unwrap(), configuration);
    assert_eq!(fs::read(&owner_path).unwrap(), invalid_owner);
    assert_eq!(fs::read(&ci_path).unwrap(), ci_before);
    assert!(outcome.changes.is_empty(), "{outcome:?}");
    assert!(outcome.artifacts.is_empty(), "{outcome:?}");
    let _ = fs::remove_dir_all(&context.workspace_root);
}

#[test]
fn public_interface_edit_rejects_invalid_configuration_owner_without_mutating_any_file() {
    let context = temp_context("public-invalid-configuration-owner");
    fs::write(
        context.cwd.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
    )
    .unwrap();
    fs::create_dir_all(context.cwd.join("src")).unwrap();
    let configuration_path = context.cwd.join("src/Configuration.xml");
    let invalid_configuration = configuration_owner_document("banana").into_bytes();
    fs::write(&configuration_path, &invalid_configuration).unwrap();
    let ci_rel = "src/Ext/CommandInterface.xml";
    let ci_path = write_command_interface(
        &context,
        ci_rel,
        &command_interface_document(INTERFACE_CI_NS, ""),
    );
    let ci_before = fs::read(&ci_path).unwrap();
    let args = Map::from_iter([
        (
            "cwd".to_string(),
            Value::String(context.cwd.display().to_string()),
        ),
        ("CIPath".to_string(), Value::String(ci_rel.to_string())),
        ("Operation".to_string(), Value::String("hide".to_string())),
        (
            "Value".to_string(),
            Value::String("Catalog.Products.StandardCommand.OpenList".to_string()),
        ),
        ("dryRun".to_string(), Value::Bool(false)),
        ("confirm".to_string(), Value::Bool(true)),
    ]);

    let outcome = UnicaApplication::new()
        .call_tool("unica.interface.edit", &args)
        .unwrap();

    assert!(!outcome.ok, "{outcome:?}");
    let errors = outcome.errors.join("\n");
    assert!(errors.contains("IncludeHelpInContents"), "{outcome:?}");
    assert!(errors.contains("banana"), "{outcome:?}");
    assert_eq!(
        fs::read(&configuration_path).unwrap(),
        invalid_configuration
    );
    assert_eq!(fs::read(&ci_path).unwrap(), ci_before);
    assert!(outcome.changes.is_empty(), "{outcome:?}");
    assert!(outcome.artifacts.is_empty(), "{outcome:?}");
    let _ = fs::remove_dir_all(&context.workspace_root);
}

#[test]
fn public_interface_edit_prioritizes_newer_metadata_owner_over_older_command_interface() {
    let context = temp_context("public-mixed-owner-versions");
    fs::write(
        context.cwd.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
    )
    .unwrap();
    fs::create_dir_all(context.cwd.join("src/Subsystems")).unwrap();
    let configuration_path = context.cwd.join("src/Configuration.xml");
    let configuration = configuration_owner_document("false").into_bytes();
    fs::write(&configuration_path, &configuration).unwrap();

    let owner_path = context.cwd.join("src/Subsystems/AuditSubsystem.xml");
    let newer_owner = subsystem_document("AuditSubsystem", "true")
        .replacen(r#"version="2.20""#, r#"version="2.21""#, 1)
        .into_bytes();
    fs::write(&owner_path, &newer_owner).unwrap();

    let ci_rel = "src/Subsystems/AuditSubsystem/Ext/CommandInterface.xml";
    let older_ci = command_interface_document(INTERFACE_CI_NS, "").replacen(
        r#"version="2.20""#,
        r#"version="2.19""#,
        1,
    );
    let ci_path = write_command_interface(&context, ci_rel, &older_ci);
    let ci_before = fs::read(&ci_path).unwrap();
    let args = Map::from_iter([
        (
            "cwd".to_string(),
            Value::String(context.cwd.display().to_string()),
        ),
        ("CIPath".to_string(), Value::String(ci_rel.to_string())),
        ("Operation".to_string(), Value::String("hide".to_string())),
        (
            "Value".to_string(),
            Value::String("Catalog.Products.StandardCommand.OpenList".to_string()),
        ),
        ("dryRun".to_string(), Value::Bool(false)),
        ("confirm".to_string(), Value::Bool(true)),
    ]);

    let outcome = UnicaApplication::new()
        .call_tool("unica.interface.edit", &args)
        .unwrap();

    assert!(!outcome.ok, "{outcome:?}");
    let diagnostic = &outcome.diagnostics.as_ref().unwrap()["formatCompatibility"];
    assert_eq!(diagnostic["code"], "platformVersionUnsupported");
    assert_eq!(diagnostic["actualFormat"], "2.21");
    let warning = outcome.warnings.join("\n");
    assert!(warning.contains("1С 8.5"), "{warning}");
    assert!(!warning.contains("миграц"), "{warning}");
    assert!(!warning.contains("повторно выгруз"), "{warning}");
    assert!(!warning.contains("re-export"), "{warning}");
    assert_eq!(fs::read(&configuration_path).unwrap(), configuration);
    assert_eq!(fs::read(&owner_path).unwrap(), newer_owner);
    assert_eq!(fs::read(&ci_path).unwrap(), ci_before);
    assert!(outcome.changes.is_empty(), "{outcome:?}");
    assert!(outcome.artifacts.is_empty(), "{outcome:?}");
    let _ = fs::remove_dir_all(&context.workspace_root);
}

#[test]
fn interface_edit_post_write_failure_restores_existing_preimage() {
    let context = temp_context("post-write-existing");
    let ci_rel = "src/Subsystems/Existing/Ext/CommandInterface.xml";
    write_valid_subsystem_owner(&context, ci_rel);
    let source = command_interface_document(INTERFACE_CI_NS, "");
    let ci_path = write_command_interface(&context, ci_rel, &source);
    let before = fs::read(&ci_path).unwrap();

    let outcome = with_commit_failpoint(CommitFailpoint::PostWriteValidation, || {
        edit_interface(&hide_args(ci_rel, false), &context)
    });

    assert!(!outcome.ok, "{outcome:?}");
    assert!(
        outcome.errors.join("\n").contains("post-write validation"),
        "{outcome:?}"
    );
    assert_eq!(fs::read(&ci_path).unwrap(), before);
    assert!(outcome.changes.is_empty(), "{outcome:?}");
    assert!(outcome.artifacts.is_empty(), "{outcome:?}");
    let _ = fs::remove_dir_all(&context.workspace_root);
}

#[test]
fn interface_edit_post_write_failure_removes_new_file_and_directories() {
    let context = temp_context("post-write-create");
    let ci_rel = "src/Subsystems/New/Ext/CommandInterface.xml";
    let ci_path = context.cwd.join(ci_rel);
    let (owner_path, owner_bytes) = write_valid_subsystem_owner(&context, ci_rel);

    let outcome = with_commit_failpoint(CommitFailpoint::PostWriteValidation, || {
        edit_interface(&hide_args(ci_rel, true), &context)
    });

    assert!(!outcome.ok, "{outcome:?}");
    assert!(
        outcome.errors.join("\n").contains("post-write validation"),
        "{outcome:?}"
    );
    assert!(!ci_path.exists(), "{outcome:?}");
    assert!(
        !ci_path.parent().unwrap().parent().unwrap().exists(),
        "{outcome:?}"
    );
    assert_eq!(fs::read(owner_path).unwrap(), owner_bytes);
    assert!(outcome.changes.is_empty(), "{outcome:?}");
    assert!(outcome.artifacts.is_empty(), "{outcome:?}");
    let _ = fs::remove_dir_all(&context.workspace_root);
}

#[test]
fn interface_edit_rejects_stale_preimage_without_overwriting_concurrent_change() {
    let context = temp_context("stale-preimage");
    let ci_rel = "src/Subsystems/Existing/Ext/CommandInterface.xml";
    write_valid_subsystem_owner(&context, ci_rel);
    let source = command_interface_document(INTERFACE_CI_NS, "");
    let ci_path = write_command_interface(&context, ci_rel, &source);
    let concurrent = command_interface_document(
        INTERFACE_CI_NS,
        "\t<GroupsOrder><Group>Concurrent</Group></GroupsOrder>",
    )
    .into_bytes();
    let hook_path = ci_path.clone();
    let hook_bytes = concurrent.clone();

    let outcome = with_before_commit_hook(
        move |path| {
            assert_eq!(path, hook_path);
            fs::write(path, &hook_bytes).unwrap();
        },
        || edit_interface(&hide_args(ci_rel, false), &context),
    );

    assert!(!outcome.ok, "{outcome:?}");
    let errors = outcome.errors.join("\n");
    assert!(
        errors.contains("changed") || errors.contains("preimage"),
        "{outcome:?}"
    );
    assert_eq!(fs::read(&ci_path).unwrap(), concurrent);
    assert!(outcome.changes.is_empty(), "{outcome:?}");
    assert!(outcome.artifacts.is_empty(), "{outcome:?}");
    let _ = fs::remove_dir_all(&context.workspace_root);
}

#[test]
pub(crate) fn interface_edit_rolls_back_if_unchanged_metadata_owner_changes_during_publication() {
    let context = temp_context("metadata-owner-race");
    fs::write(
        context.cwd.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
    )
    .unwrap();
    fs::create_dir_all(context.cwd.join("src")).unwrap();
    fs::write(
        context.cwd.join("src/Configuration.xml"),
        configuration_owner_document("true"),
    )
    .unwrap();
    let ci_rel = "src/Subsystems/AuditSubsystem/Ext/CommandInterface.xml";
    let (owner_path, _) = write_valid_subsystem_owner(&context, ci_rel);
    let ci_path = write_command_interface(
        &context,
        ci_rel,
        &command_interface_document(INTERFACE_CI_NS, ""),
    );
    let ci_before = fs::read(&ci_path).unwrap();
    let concurrent_owner = subsystem_document("AuditSubsystem", "true")
        .replace("<Explanation/>", "<Explanation>Concurrent</Explanation>");
    let owner_for_hook = owner_path.clone();
    let owner_bytes_for_hook = concurrent_owner.as_bytes().to_vec();

    let outcome = with_before_commit_hook(
        move |_| fs::write(&owner_for_hook, &owner_bytes_for_hook).unwrap(),
        || edit_interface(&hide_args(ci_rel, false), &context),
    );

    assert!(!outcome.ok, "{outcome:?}");
    assert!(
        outcome.errors.join("\n").contains("read guard"),
        "{outcome:?}"
    );
    assert_eq!(fs::read(&ci_path).unwrap(), ci_before);
    assert_eq!(fs::read_to_string(&owner_path).unwrap(), concurrent_owner);
    let _ = fs::remove_dir_all(&context.workspace_root);
}

#[test]
fn interface_edit_classifies_ci_and_metadata_owner_as_one_dependency_set() {
    let context = temp_context("mixed-dependency-versions");
    fs::write(
        context.cwd.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
    )
    .unwrap();
    fs::create_dir_all(context.cwd.join("src")).unwrap();
    fs::write(
        context.cwd.join("src/Configuration.xml"),
        configuration_owner_document("true"),
    )
    .unwrap();
    let ci_rel = "src/Subsystems/AuditSubsystem/Ext/CommandInterface.xml";
    let (owner_path, _) = write_valid_subsystem_owner(&context, ci_rel);
    let newer_owner = subsystem_document("AuditSubsystem", "true").replacen(
        r#"version="2.20""#,
        r#"version="2.21""#,
        1,
    );
    fs::write(&owner_path, &newer_owner).unwrap();
    let older_ci = command_interface_document(INTERFACE_CI_NS, "").replacen(
        r#"version="2.20""#,
        r#"version="2.19""#,
        1,
    );
    let ci_path = write_command_interface(&context, ci_rel, &older_ci);
    let ci_before = fs::read(&ci_path).unwrap();

    let outcome = edit_interface(&hide_args(ci_rel, false), &context);

    assert!(!outcome.ok, "{outcome:?}");
    let errors = outcome.errors.join("\n");
    assert!(errors.contains("newer than supported 2.20"), "{outcome:?}");
    assert!(errors.contains("1C 8.5 support is planned"), "{outcome:?}");
    assert!(!errors.contains("re-export the source"), "{outcome:?}");
    assert_eq!(fs::read(&ci_path).unwrap(), ci_before);
    assert_eq!(fs::read_to_string(&owner_path).unwrap(), newer_owner);
    let _ = fs::remove_dir_all(&context.workspace_root);
}
