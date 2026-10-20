use super::actor_capacity_tests::direct_v5;
use super::*;
use crate::application::invocation_store::ToolIdentity;
use crate::infrastructure::platform::filesystem::RetainedDirectoryCapability;
use std::path::Path;

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
        let before = crate::test_support::tree_snapshot(&source);
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
        assert_eq!(crate::test_support::tree_snapshot(&source), before);
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
        let after = crate::test_support::tree_snapshot(&source);
        let replay = invoke_apply(
            &runtime,
            workspace.path(),
            serde_json::json!({"executionToken":token}),
        );
        assert_eq!(
            serde_json::to_value(&replay).unwrap(),
            serde_json::to_value(&published).unwrap()
        );
        assert_eq!(crate::test_support::tree_snapshot(&source), after);
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
    let before = crate::test_support::tree_snapshot(&source);
    let result = invoke_apply(
        &runtime,
        workspace.path(),
        serde_json::json!({"executionToken":token}),
    );
    assert!(!result.ok, "{result:?}");
    assert_eq!(result.diagnostics[0]["code"], "stale_revision");
    assert_eq!(crate::test_support::tree_snapshot(&source), before);
    assert_eq!(
        RetainedDirectoryCapability::open(&foreign)
            .unwrap()
            .identity(),
        identity
    );
}

#[test]
fn canonical_remove_restores_original_directories_and_cache_on_each_publication_failure() {
    use crate::infrastructure::native_operations::compile_transaction::{
        set_retained_apply_failpoint, RetainedApplyFailpoint,
    };
    use crate::infrastructure::platform::filesystem::portable_permissions;
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
        let before = crate::test_support::tree_snapshot(&source);
        let cache = workspace.path().join(".build/unica");
        let cache_before = cache
            .exists()
            .then(|| crate::test_support::tree_snapshot(&cache));
        set_retained_apply_failpoint(failpoint);
        let result = invoke_apply(
            &runtime,
            workspace.path(),
            serde_json::json!({"executionToken":token}),
        );
        assert!(!result.ok, "{failpoint:?}: {result:?}");
        assert_eq!(
            crate::test_support::tree_snapshot(&source),
            before,
            "{failpoint:?}"
        );
        assert_eq!(
            cache
                .exists()
                .then(|| crate::test_support::tree_snapshot(&cache)),
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
    use crate::infrastructure::native_operations::compile_transaction::{
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

#[test]
fn canonical_form_remove_deletes_its_payload_directories_and_only_its_collection() {
    for unknown_sibling in [false, true] {
        let (workspace, source) = object_fixture(false);
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
        let before = crate::test_support::tree_snapshot(&source);
        let preview = invoke_apply(
            &runtime,
            workspace.path(),
            serde_json::json!({"at":"main:Document.Order.Form.Main","ops":[{"op":"form.remove"}]}),
        );
        assert!(preview.ok, "{preview:?}");
        assert_eq!(crate::test_support::tree_snapshot(&source), before);
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
    use crate::infrastructure::native_operations::compile_transaction::set_retained_apply_before_post_validation_hook;
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
