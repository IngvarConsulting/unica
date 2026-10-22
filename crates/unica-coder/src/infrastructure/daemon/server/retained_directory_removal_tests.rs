use super::actor_capacity_tests::direct_v5;
use super::*;
use std::path::Path;
use unica_execution::invocation_store::ToolIdentity;
use unica_workspace::state_fs::StateDirectory as RetainedDirectoryCapability;

fn invoke_apply(
    runtime: &V5CanonicalInvocationRuntime,
    workspace: &Path,
    arguments: serde_json::Value,
) -> DomainResult {
    direct_v5(
        runtime,
        InvocationRequest::new(
            ToolIdentity::Apply,
            arguments,
            std::fs::canonicalize(workspace).unwrap().to_string_lossy(),
            7_000,
        )
        .unwrap(),
    )
    .unwrap()
}

fn object_fixture(unknown_sibling: bool) -> (tempfile::TempDir, std::path::PathBuf) {
    let workspace = tempfile::tempdir().unwrap();
    let source = std::fs::canonicalize(workspace.path()).unwrap().join("src");
    let payload = source.join("Documents/Order");
    std::fs::create_dir_all(payload.join("Ext")).unwrap();
    std::fs::create_dir_all(payload.join("Empty/Deep")).unwrap();
    let untouched = source.join("UnrelatedEmpty");
    std::fs::create_dir(&untouched).unwrap();
    if unknown_sibling {
        std::fs::create_dir_all(source.join("Documents/Unknown/Empty")).unwrap();
    }
    std::fs::write(
        workspace.path().join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
    )
    .unwrap();
    std::fs::write(
        source.join("Configuration.xml"),
        r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Store</Name></Properties><ChildObjects><Document>Order</Document></ChildObjects></Configuration></MetaDataObject>"#,
    )
    .unwrap();
    let descriptor = source.join("Documents/Order.xml");
    std::fs::write(
        &descriptor,
        r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:app="http://v8.1c.ru/8.2/managed-application/core" xmlns:cfg="http://v8.1c.ru/8.1/data/enterprise/current-config" xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" version="2.20"><Document uuid="11111111-1111-4111-8111-111111111111"><Properties><Name>Order</Name><Synonym/><Comment/></Properties><ChildObjects/></Document></MetaDataObject>"#,
    )
    .unwrap();
    let module = payload.join("Ext/ObjectModule.bsl");
    std::fs::write(&module, b"Procedure Execute() Export\nEndProcedure\n").unwrap();
    (workspace, source)
}

fn runtime() -> V5CanonicalInvocationRuntime {
    V5CanonicalInvocationRuntime::new(
        Arc::new(crate::infrastructure::daemon::v13_service::CanonicalV13ReadService::default()),
        Arc::new(TokioClock),
    )
}

fn removal_preview(runtime: &V5CanonicalInvocationRuntime, workspace: &Path) -> serde_json::Value {
    let preview = invoke_apply(
        runtime,
        workspace,
        serde_json::json!({"at":"main:Document.Order","ops":[{"op":"object.remove"}]}),
    );
    assert!(preview.ok, "{preview:?}");
    preview.data.unwrap()["executionToken"].clone()
}

#[test]
fn canonical_object_remove_removes_its_directories_and_preserves_unknown_siblings() {
    for unknown_sibling in [false, true] {
        let (workspace, source) = object_fixture(unknown_sibling);
        let payload = source.join("Documents/Order");
        let descriptor = source.join("Documents/Order.xml");
        let module = payload.join("Ext/ObjectModule.bsl");
        let untouched = source.join("UnrelatedEmpty");
        let before = unica_workspace::test_support::tree_snapshot(&source);
        let directory_paths = [
            payload.clone(),
            payload.join("Empty/Deep"),
            untouched.clone(),
        ];
        let identities = directory_paths
            .iter()
            .map(|path| RetainedDirectoryCapability::open(path).unwrap().identity())
            .collect::<Vec<_>>();
        let runtime = V5CanonicalInvocationRuntime::new(
            Arc::new(
                crate::infrastructure::daemon::v13_service::CanonicalV13ReadService::default(),
            ),
            Arc::new(TokioClock),
        );
        let preview = invoke_apply(
            &runtime,
            workspace.path(),
            serde_json::json!({"at":"main:Document.Order","ops":[{"op":"object.remove","args":{}}]}),
        );
        assert!(preview.ok, "{preview:?}");
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&source),
            before
        );
        for (path, identity) in directory_paths.iter().zip(&identities) {
            assert_eq!(
                RetainedDirectoryCapability::open(path).unwrap().identity(),
                *identity
            );
        }
        let token = preview.data.as_ref().unwrap()["executionToken"]
            .as_str()
            .unwrap();
        let published = invoke_apply(
            &runtime,
            workspace.path(),
            serde_json::json!({"executionToken":token}),
        );
        assert!(published.ok, "{published:?}");
        assert_eq!(published.data.as_ref().unwrap()["mode"], "published");
        assert!(!descriptor.exists());
        assert!(!module.exists());
        assert!(
            !payload.exists(),
            "canonical object.remove left its payload directories"
        );
        assert_eq!(source.join("Documents").exists(), unknown_sibling);
        if unknown_sibling {
            assert!(source.join("Documents/Unknown/Empty").is_dir());
        }
        assert_eq!(
            RetainedDirectoryCapability::open(&untouched)
                .unwrap()
                .identity(),
            identities[2]
        );
        assert!(!std::fs::read_to_string(source.join("Configuration.xml"))
            .unwrap()
            .contains("<Document>Order</Document>"));
        let successor = source.join("Documents/Order/NewOwnerEmpty");
        std::fs::create_dir_all(&successor).unwrap();
        let successor_identity = RetainedDirectoryCapability::open(&successor)
            .unwrap()
            .identity();
        let after = unica_workspace::test_support::tree_snapshot(&source);
        let replay = invoke_apply(
            &runtime,
            workspace.path(),
            serde_json::json!({"executionToken":token}),
        );
        assert_eq!(
            serde_json::to_value(&replay).unwrap(),
            serde_json::to_value(&published).unwrap()
        );
        assert_eq!(unica_workspace::test_support::tree_snapshot(&source), after);
        assert_eq!(
            RetainedDirectoryCapability::open(&successor)
                .unwrap()
                .identity(),
            successor_identity
        );
    }
}

#[test]
fn canonical_remove_preserves_generated_subtree_and_all_its_ancestors() {
    let (workspace, source) = object_fixture(false);
    let generated = source.join("Documents/Order/Ext/.build/owned-empty");
    std::fs::create_dir_all(&generated).unwrap();
    let paths = [
        source.join("Documents"),
        source.join("Documents/Order"),
        source.join("Documents/Order/Ext"),
        generated,
    ];
    let identities = paths
        .iter()
        .map(|path| RetainedDirectoryCapability::open(path).unwrap().identity())
        .collect::<Vec<_>>();
    let runtime = runtime();
    let token = removal_preview(&runtime, workspace.path());
    let result = invoke_apply(
        &runtime,
        workspace.path(),
        serde_json::json!({"executionToken":token}),
    );
    assert!(result.ok, "{result:?}");
    assert!(!source.join("Documents/Order.xml").exists());
    assert!(!source.join("Documents/Order/Ext/ObjectModule.bsl").exists());
    assert!(!source.join("Documents/Order/Empty").exists());
    for (path, identity) in paths.iter().zip(identities) {
        assert_eq!(
            RetainedDirectoryCapability::open(path).unwrap().identity(),
            identity
        );
    }
}

#[test]
fn canonical_remove_rejects_a_late_empty_directory_without_deleting_it() {
    let (workspace, source) = object_fixture(false);
    let runtime = runtime();
    let token = removal_preview(&runtime, workspace.path());
    let foreign = source.join("Documents/Order/late-empty");
    std::fs::create_dir(&foreign).unwrap();
    let identity = RetainedDirectoryCapability::open(&foreign)
        .unwrap()
        .identity();
    let before = unica_workspace::test_support::tree_snapshot(&source);
    let result = invoke_apply(
        &runtime,
        workspace.path(),
        serde_json::json!({"executionToken":token}),
    );
    assert!(!result.ok, "{result:?}");
    assert_eq!(result.diagnostics[0]["code"], "stale_revision");
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&source),
        before
    );
    assert_eq!(
        RetainedDirectoryCapability::open(&foreign)
            .unwrap()
            .identity(),
        identity
    );
}

#[test]
fn canonical_remove_restores_original_directories_and_cache_on_each_publication_failure() {
    use unica_workspace::compile_transaction::{
        set_retained_apply_failpoint, RetainedApplyFailpoint,
    };
    use unica_workspace::test_support::portable_permissions_for_test as portable_permissions;
    for failpoint in [
        RetainedApplyFailpoint::Source(1),
        RetainedApplyFailpoint::StateMarker,
        RetainedApplyFailpoint::AfterAllPostimages,
    ] {
        let (workspace, source) = object_fixture(false);
        let runtime = runtime();
        let token = removal_preview(&runtime, workspace.path());
        let paths = [
            source.join("Documents"),
            source.join("Documents/Order"),
            source.join("Documents/Order/Empty/Deep"),
        ];
        let identities = paths
            .iter()
            .map(|path| RetainedDirectoryCapability::open(path).unwrap().identity())
            .collect::<Vec<_>>();
        let permissions = paths
            .iter()
            .map(|path| portable_permissions(&std::fs::metadata(path).unwrap()))
            .collect::<Vec<_>>();
        let before = unica_workspace::test_support::tree_snapshot(&source);
        let cache = workspace.path().join(".build/unica");
        let cache_before = cache
            .exists()
            .then(|| unica_workspace::test_support::tree_snapshot(&cache));
        set_retained_apply_failpoint(failpoint);
        let result = invoke_apply(
            &runtime,
            workspace.path(),
            serde_json::json!({"executionToken":token}),
        );
        assert!(!result.ok, "{failpoint:?}: {result:?}");
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&source),
            before,
            "{failpoint:?}"
        );
        assert_eq!(
            cache
                .exists()
                .then(|| unica_workspace::test_support::tree_snapshot(&cache)),
            cache_before,
            "{failpoint:?}"
        );
        for ((path, identity), permissions) in paths.iter().zip(identities).zip(permissions) {
            assert_eq!(
                RetainedDirectoryCapability::open(path).unwrap().identity(),
                identity
            );
            assert!(permissions.matches(&std::fs::metadata(path).unwrap()));
        }
        assert!(std::fs::read_dir(&source).unwrap().all(|entry| !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".unica-apply-dir-")));
    }
}

#[test]
fn canonical_remove_keeps_foreign_public_directory_and_original_recovery_on_rollback_conflict() {
    use unica_workspace::compile_transaction::{
        set_retained_apply_before_rollback_hook, set_retained_apply_failpoint,
        RetainedApplyFailpoint,
    };
    let (workspace, source) = object_fixture(false);
    let runtime = runtime();
    let token = removal_preview(&runtime, workspace.path());
    let original_identity = RetainedDirectoryCapability::open(&source.join("Documents"))
        .unwrap()
        .identity();
    let foreign = source.join("Documents");
    set_retained_apply_before_rollback_hook(move || {
        std::fs::create_dir(&foreign).unwrap();
        std::fs::write(foreign.join("foreign.txt"), b"foreign owner").unwrap();
    });
    set_retained_apply_failpoint(RetainedApplyFailpoint::Source(1));
    let result = invoke_apply(
        &runtime,
        workspace.path(),
        serde_json::json!({"executionToken":token}),
    );
    assert!(!result.ok, "{result:?}");
    assert_eq!(
        result.diagnostics[0]["code"], "rollback_incomplete",
        "{result:?}"
    );
    assert_eq!(
        std::fs::read(source.join("Documents/foreign.txt")).unwrap(),
        b"foreign owner"
    );
    let recovery = std::fs::read_dir(&source)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(".unica-apply-dir-")
        })
        .expect("original tree must remain recoverable");
    assert_eq!(
        RetainedDirectoryCapability::open(&recovery)
            .unwrap()
            .identity(),
        original_identity
    );
    assert_eq!(
        std::fs::read(recovery.join("Order/Ext/ObjectModule.bsl")).unwrap(),
        b"Procedure Execute() Export\nEndProcedure\n"
    );
    assert!(recovery.join("Order/Empty/Deep").is_dir());
}

/// Writers that synthesize synonyms need the configuration default language.
fn add_default_language(source: &Path) {
    let configuration = source.join("Configuration.xml");
    let text = std::fs::read_to_string(&configuration)
        .unwrap()
        .replace(
            "<Name>Store</Name>",
            "<Name>Store</Name><DefaultLanguage>Language.Русский</DefaultLanguage>",
        )
        .replace(
            "<ChildObjects>",
            "<ChildObjects><Language>Русский</Language>",
        );
    std::fs::write(configuration, text).unwrap();
    std::fs::create_dir(source.join("Languages")).unwrap();
    std::fs::write(source.join("Languages/Русский.xml"), r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Language uuid="33333333-3333-4333-8333-333333333333"><Properties><Name>Русский</Name><Comment/><LanguageCode>ru</LanguageCode></Properties></Language></MetaDataObject>"#).unwrap();
}

#[test]
fn canonical_form_remove_deletes_its_payload_directories_and_only_its_collection() {
    for unknown_sibling in [false, true] {
        let (workspace, source) = object_fixture(false);
        add_default_language(&source);
        let owner = source.join("Documents/Order.xml");
        let text = std::fs::read_to_string(&owner).unwrap().replace(
            "<Comment/>",
            "<Comment/><DefaultObjectForm/><DefaultListForm/><DefaultChoiceForm/>",
        );
        std::fs::write(&owner, text).unwrap();
        let runtime = runtime();
        let add = invoke_apply(
            &runtime,
            workspace.path(),
            serde_json::json!({"at":"main:Document.Order","ops":[{"op":"form.add","args":{"items":[{"name":"Main","type":"ObjectForm"}]}}]}),
        );
        assert!(add.ok, "{add:?}");
        let published = invoke_apply(
            &runtime,
            workspace.path(),
            serde_json::json!({"executionToken":add.data.unwrap()["executionToken"]}),
        );
        assert!(published.ok, "{published:?}");
        let forms = source.join("Documents/Order/Forms");
        std::fs::create_dir_all(forms.join("Main/Ext/Form/Items/Empty")).unwrap();
        let other = forms.join("Unrelated/Empty");
        let other_identity = if unknown_sibling {
            std::fs::create_dir_all(&other).unwrap();
            Some(
                RetainedDirectoryCapability::open(&other)
                    .unwrap()
                    .identity(),
            )
        } else {
            None
        };
        let owner_module = source.join("Documents/Order/Ext/ObjectModule.bsl");
        let owner_bytes = std::fs::read(&owner_module).unwrap();
        let before = unica_workspace::test_support::tree_snapshot(&source);
        let preview = invoke_apply(
            &runtime,
            workspace.path(),
            serde_json::json!({"at":"main:Document.Order.Form.Main","ops":[{"op":"form.remove"}]}),
        );
        assert!(preview.ok, "{preview:?}");
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&source),
            before
        );
        assert!(forms.join("Main/Ext/Form/Items/Empty").is_dir());
        let published = invoke_apply(
            &runtime,
            workspace.path(),
            serde_json::json!({"executionToken":preview.data.unwrap()["executionToken"]}),
        );
        assert!(published.ok, "{published:?}");
        assert_eq!(forms.exists(), unknown_sibling);
        assert!(!forms.join("Main").exists());
        if let Some(identity) = other_identity {
            assert_eq!(
                RetainedDirectoryCapability::open(&other)
                    .unwrap()
                    .identity(),
                identity
            );
        }
        assert_eq!(std::fs::read(owner_module).unwrap(), owner_bytes);
        assert!(source.join("Documents/Order/Empty/Deep").is_dir());
        assert!(owner.exists());
    }
}

#[test]
fn canonical_remove_preserves_a_foreign_entry_added_to_displaced_tree() {
    use unica_workspace::compile_transaction::set_retained_apply_before_post_validation_hook;
    let (workspace, source) = object_fixture(false);
    let runtime = runtime();
    let token = removal_preview(&runtime, workspace.path());
    let original_identity = RetainedDirectoryCapability::open(&source.join("Documents"))
        .unwrap()
        .identity();
    let moved_root = source.clone();
    set_retained_apply_before_post_validation_hook(move || {
        let recovery = std::fs::read_dir(&moved_root)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| {
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with(".unica-apply-dir-")
            })
            .unwrap();
        std::fs::create_dir(recovery.join("late-foreign-empty")).unwrap();
        std::fs::write(recovery.join("late-foreign.txt"), b"new owner bytes").unwrap();
    });
    let result = invoke_apply(
        &runtime,
        workspace.path(),
        serde_json::json!({"executionToken":token}),
    );
    assert!(!result.ok, "{result:?}");
    assert_eq!(
        result.diagnostics[0]["code"], "rollback_incomplete",
        "{result:?}"
    );
    let recovery = std::fs::read_dir(&source)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(".unica-apply-dir-")
        })
        .unwrap();
    assert_eq!(
        RetainedDirectoryCapability::open(&recovery)
            .unwrap()
            .identity(),
        original_identity
    );
    assert_eq!(
        std::fs::read(recovery.join("late-foreign.txt")).unwrap(),
        b"new owner bytes"
    );
    assert!(recovery.join("late-foreign-empty").is_dir());
    assert!(recovery.join("Order/Empty/Deep").is_dir());
    assert_eq!(
        std::fs::read(recovery.join("Order/Ext/ObjectModule.bsl")).unwrap(),
        b"Procedure Execute() Export\nEndProcedure\n"
    );
}

/// Every directory under `root`, so empty template directories are visible.
fn directory_set(root: &Path) -> std::collections::BTreeSet<std::path::PathBuf> {
    fn visit(
        root: &Path,
        current: &Path,
        output: &mut std::collections::BTreeSet<std::path::PathBuf>,
    ) {
        for entry in std::fs::read_dir(current).unwrap() {
            let path = entry.unwrap().path();
            if std::fs::symlink_metadata(&path).unwrap().is_dir() {
                output.insert(path.strip_prefix(root).unwrap().to_path_buf());
                visit(root, &path, output);
            }
        }
    }
    let mut output = std::collections::BTreeSet::new();
    visit(root, root, &mut output);
    output
}

/// `Document.Order` with a spreadsheet, a DCS and a binary template. The
/// spreadsheet payload also carries an empty nested directory.
fn template_fixture() -> (
    tempfile::TempDir,
    std::path::PathBuf,
    V5CanonicalInvocationRuntime,
) {
    let (workspace, source) = object_fixture(false);
    add_default_language(&source);
    let runtime = runtime();
    let add = invoke_apply(
        &runtime,
        workspace.path(),
        serde_json::json!({"at":"main:Document.Order","ops":[{"op":"template.add","args":{"items":[
            {"name":"Sheet","templateType":"SpreadsheetDocument"},
            {"name":"Schema","templateType":"DataCompositionSchema"}
        ]}}]}),
    );
    assert!(add.ok, "{add:?}");
    let published = invoke_apply(
        &runtime,
        workspace.path(),
        serde_json::json!({"executionToken":add.data.unwrap()["executionToken"]}),
    );
    assert!(published.ok, "{published:?}");
    let templates = source.join("Documents/Order/Templates");
    // The binary template is the sheet descriptor with another identity;
    // its registration follows the sheet one in the owner.
    let sheet = std::fs::read_to_string(templates.join("Sheet.xml")).unwrap();
    let uuid_start = sheet.find("uuid=\"").unwrap() + "uuid=\"".len();
    let uuid = &sheet[uuid_start..uuid_start + 36];
    let data = sheet
        .replace(uuid, "22222222-2222-4222-8222-222222222222")
        .replace("<Name>Sheet</Name>", "<Name>Data</Name>")
        .replace(
            "<TemplateType>SpreadsheetDocument</TemplateType>",
            "<TemplateType>BinaryData</TemplateType>",
        );
    assert_ne!(data, sheet);
    std::fs::write(templates.join("Data.xml"), data).unwrap();
    std::fs::create_dir_all(templates.join("Data/Ext")).unwrap();
    std::fs::write(
        templates.join("Data/Ext/Template.bin"),
        b"\x00\x01binary\xff",
    )
    .unwrap();
    let owner = source.join("Documents/Order.xml");
    let text = std::fs::read_to_string(&owner).unwrap();
    assert_eq!(text.matches("<Template>Sheet</Template>").count(), 1);
    std::fs::write(
        &owner,
        text.replace(
            "<Template>Sheet</Template>",
            "<Template>Sheet</Template><Template>Data</Template>",
        ),
    )
    .unwrap();
    std::fs::create_dir_all(templates.join("Sheet/Ext/Empty/Deep")).unwrap();
    (workspace, source, runtime)
}

fn template_removal_preview(
    runtime: &V5CanonicalInvocationRuntime,
    workspace: &Path,
    name: &str,
) -> serde_json::Value {
    let at = format!("main:Document.Order.Template.{name}");
    let preview = invoke_apply(
        runtime,
        workspace,
        serde_json::json!({"at":at,"ops":[{"op":"template.remove","args":{"at":at}}]}),
    );
    assert!(preview.ok, "{name}: {preview:?}");
    assert_eq!(preview.data.as_ref().unwrap()["mode"], "preview");
    preview.data.unwrap()["executionToken"].clone()
}

#[test]
fn canonical_template_remove_deletes_its_payload_directories_and_only_its_collection() {
    for foreign_entry in [false, true] {
        let (workspace, source, runtime) = template_fixture();
        let templates = source.join("Documents/Order/Templates");
        let foreign = templates.join("Unrelated/Empty");
        if foreign_entry {
            std::fs::create_dir_all(&foreign).unwrap();
            std::fs::write(templates.join("notes.txt"), b"foreign owner").unwrap();
        }
        let owner_module = source.join("Documents/Order/Ext/ObjectModule.bsl");
        let owner_bytes = std::fs::read(&owner_module).unwrap();
        let order = ["Sheet", "Schema", "Data"];
        for (index, name) in order.iter().copied().enumerate() {
            let remaining = &order[index + 1..];
            let payload = templates.join(name);
            assert!(payload.join("Ext").is_dir(), "{name}");
            let neighbors = remaining
                .iter()
                .map(|other| {
                    let path = templates.join(other);
                    let identity = RetainedDirectoryCapability::open(&path).unwrap().identity();
                    (path, identity)
                })
                .collect::<Vec<_>>();
            let before = unica_workspace::test_support::tree_snapshot(&source);
            let directories = directory_set(&source);
            let token = template_removal_preview(&runtime, workspace.path(), name);
            assert_eq!(
                unica_workspace::test_support::tree_snapshot(&source),
                before
            );
            assert_eq!(directory_set(&source), directories, "{name}: preview wrote");
            let published = invoke_apply(
                &runtime,
                workspace.path(),
                serde_json::json!({"executionToken":token}),
            );
            assert!(published.ok, "{name}: {published:?}");
            assert_eq!(published.data.as_ref().unwrap()["mode"], "published");
            assert!(!payload.exists(), "{name}: payload directories remain");
            assert!(!templates.join(format!("{name}.xml")).exists());
            assert_eq!(
                templates.exists(),
                foreign_entry || !remaining.is_empty(),
                "{name}: collection"
            );
            for (path, identity) in &neighbors {
                assert_eq!(
                    RetainedDirectoryCapability::open(path).unwrap().identity(),
                    *identity,
                    "{name}: neighbor {path:?}"
                );
            }
            let removed_directories = directories
                .difference(&directory_set(&source))
                .cloned()
                .collect::<Vec<_>>();
            let payload_relative = payload.strip_prefix(&source).unwrap();
            assert!(removed_directories.iter().all(|path| {
                path.starts_with(payload_relative)
                    || (remaining.is_empty()
                        && !foreign_entry
                        && path == templates.strip_prefix(&source).unwrap())
            }));
        }
        assert_eq!(std::fs::read(&owner_module).unwrap(), owner_bytes);
        assert!(source.join("Documents/Order/Empty/Deep").is_dir());
        assert!(!std::fs::read_to_string(source.join("Documents/Order.xml"))
            .unwrap()
            .contains("<Template>"));
        if foreign_entry {
            assert!(foreign.is_dir());
            assert_eq!(
                std::fs::read(templates.join("notes.txt")).unwrap(),
                b"foreign owner"
            );
        }
    }
}

#[test]
fn canonical_template_remove_rejects_a_late_entry_in_its_payload_without_deleting_it() {
    for late_file in [false, true] {
        let (workspace, source, runtime) = template_fixture();
        let token = template_removal_preview(&runtime, workspace.path(), "Sheet");
        let late = source.join("Documents/Order/Templates/Sheet/Ext/late");
        if late_file {
            std::fs::write(&late, b"late owner").unwrap();
        } else {
            std::fs::create_dir(&late).unwrap();
        }
        let before = unica_workspace::test_support::tree_snapshot(&source);
        let directories = directory_set(&source);
        let result = invoke_apply(
            &runtime,
            workspace.path(),
            serde_json::json!({"executionToken":token}),
        );
        assert!(!result.ok, "{result:?}");
        assert_eq!(result.diagnostics[0]["code"], "stale_revision");
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&source),
            before
        );
        assert_eq!(directory_set(&source), directories);
    }
}

#[test]
fn canonical_template_remove_restores_its_directories_on_each_publication_failure() {
    use unica_workspace::compile_transaction::{
        set_retained_apply_failpoint, RetainedApplyFailpoint,
    };
    for failpoint in [
        RetainedApplyFailpoint::Source(1),
        RetainedApplyFailpoint::StateMarker,
        RetainedApplyFailpoint::AfterAllPostimages,
    ] {
        let (workspace, source, runtime) = template_fixture();
        // The last template takes its collection along; a rollback must
        // bring back both levels with their identities.
        for name in ["Sheet", "Schema"] {
            let token = template_removal_preview(&runtime, workspace.path(), name);
            let published = invoke_apply(
                &runtime,
                workspace.path(),
                serde_json::json!({"executionToken":token}),
            );
            assert!(published.ok, "{published:?}");
        }
        let templates = source.join("Documents/Order/Templates");
        let paths = [templates.clone(), templates.join("Data/Ext")];
        let identities = paths
            .iter()
            .map(|path| RetainedDirectoryCapability::open(path).unwrap().identity())
            .collect::<Vec<_>>();
        let token = template_removal_preview(&runtime, workspace.path(), "Data");
        let before = unica_workspace::test_support::tree_snapshot(&source);
        let directories = directory_set(&source);
        set_retained_apply_failpoint(failpoint);
        let result = invoke_apply(
            &runtime,
            workspace.path(),
            serde_json::json!({"executionToken":token}),
        );
        assert!(!result.ok, "{failpoint:?}: {result:?}");
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&source),
            before,
            "{failpoint:?}"
        );
        assert_eq!(directory_set(&source), directories, "{failpoint:?}");
        for (path, identity) in paths.iter().zip(identities) {
            assert_eq!(
                RetainedDirectoryCapability::open(path).unwrap().identity(),
                identity,
                "{failpoint:?}: {path:?}"
            );
        }
        // The restored state is the one a new plan removes completely.
        let token = template_removal_preview(&runtime, workspace.path(), "Data");
        let retried = invoke_apply(
            &runtime,
            workspace.path(),
            serde_json::json!({"executionToken":token}),
        );
        assert!(retried.ok, "{failpoint:?}: {retried:?}");
        assert!(!templates.exists(), "{failpoint:?}");
    }
}
