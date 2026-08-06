use super::{OperationResult, UnicaApplication};
use crate::composition::testing::with_meta_remove_before_reauthorization_hook;
use crate::test_support::{tree_snapshot, ProcessCwdGuard};
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};
use uuid::Uuid;

struct TempWorkspace(PathBuf);

impl TempWorkspace {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "unica-platform-meta-remove-{label}-{}-{}",
            std::process::id(),
            Uuid::new_v4()
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempWorkspace {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn create_remove_workspace(label: &str) -> TempWorkspace {
    let workspace = TempWorkspace::new(label);
    let initialized = UnicaApplication::new()
        .call_tool(
            "unica.cf.init",
            &Map::from_iter([
                (
                    "cwd".to_string(),
                    Value::String(workspace.path().display().to_string()),
                ),
                ("Name".to_string(), Value::String("MetaRemove".to_string())),
                ("OutputDir".to_string(), Value::String("src".to_string())),
                ("dryRun".to_string(), Value::Bool(false)),
            ]),
        )
        .unwrap();
    assert!(initialized.ok, "{:?}", initialized.errors);
    std::fs::write(
        workspace.path().join("v8project.yaml"),
        concat!(
            "format: DESIGNER\n",
            "source-set:\n",
            "  - name: main\n",
            "    type: CONFIGURATION\n",
            "    path: src\n",
        ),
    )
    .unwrap();
    for name in ["Removable", "Sibling"] {
        let _cwd = ProcessCwdGuard::enter(workspace.path()).unwrap();
        let added = UnicaApplication::new()
            .call_tool(
                "unica.meta.add",
                &Map::from_iter([
                    ("sourceSet".to_string(), Value::String("main".to_string())),
                    ("kind".to_string(), Value::String("Catalog".to_string())),
                    ("name".to_string(), Value::String(name.to_string())),
                    ("dryRun".to_string(), Value::Bool(false)),
                ]),
            )
            .unwrap();
        assert!(added.ok, "Catalog.{name}: {:?}", added.errors);
    }
    workspace
}

fn remove_args(dry_run: bool) -> Map<String, Value> {
    Map::from_iter([
        ("sourceSet".to_string(), Value::String("main".to_string())),
        (
            "metadataPath".to_string(),
            Value::String("Catalog.Removable".to_string()),
        ),
        ("dryRun".to_string(), Value::Bool(dry_run)),
    ])
}

fn call_remove(workspace: &Path, dry_run: bool) -> OperationResult {
    let _cwd = ProcessCwdGuard::enter(workspace).unwrap();
    UnicaApplication::new()
        .call_tool("unica.meta.remove", &remove_args(dry_run))
        .expect("private typed meta.remove call")
}

#[test]
fn meta_remove_private_coordinator_preview_and_apply_publish_events_cache_and_files() {
    let preview_workspace = create_remove_workspace("preview");
    let descriptor = preview_workspace.path().join("src/Catalogs/Removable.xml");
    let owner = preview_workspace.path().join("src/Configuration.xml");
    let descriptor_before = std::fs::read(&descriptor).unwrap();
    let owner_before = std::fs::read(&owner).unwrap();

    let preview = call_remove(preview_workspace.path(), true);

    assert!(preview.ok, "{:?}", preview.errors);
    assert_eq!(
        preview.data.as_ref().unwrap()["validation"]["status"],
        "passed"
    );
    let expected_effects = serde_json::json!([{
        "operation": "removeObject",
        "target": "Catalog.Removable",
        "before": {
            "metadataPath": "Catalog.Removable",
            "kind": "Catalog",
            "name": "Removable"
        },
        "after": null
    }]);
    assert_eq!(preview.data.as_ref().unwrap()["effects"], expected_effects);
    assert_eq!(preview.cache.mode, "dry-run");
    assert_eq!(preview.cache.events, ["MetadataChanged"]);
    assert_eq!(std::fs::read(&descriptor).unwrap(), descriptor_before);
    assert_eq!(std::fs::read(&owner).unwrap(), owner_before);

    let apply_workspace = create_remove_workspace("apply");
    let descriptor = apply_workspace.path().join("src/Catalogs/Removable.xml");
    let owner = apply_workspace.path().join("src/Configuration.xml");

    let applied = call_remove(apply_workspace.path(), false);

    assert!(applied.ok, "{:?}", applied.errors);
    assert_eq!(
        applied.data.as_ref().unwrap()["validation"]["status"],
        "passed"
    );
    assert_eq!(applied.data.as_ref().unwrap()["effects"], expected_effects);
    assert_eq!(applied.cache.mode, "applied");
    assert_eq!(applied.cache.events, ["MetadataChanged"]);
    assert!(applied
        .cache
        .invalidated
        .contains(&"workspace_graph".to_string()));
    assert!(applied
        .cache
        .invalidated
        .contains(&"metadata_graph".to_string()));
    assert!(!descriptor.exists());
    assert!(!std::fs::read_to_string(owner)
        .unwrap()
        .contains("<Catalog>Removable</Catalog>"));
}

#[test]
fn meta_remove_reauthorizes_support_state_after_reference_and_subsystem_planning() {
    let workspace = create_remove_workspace("support-authorization-drift");
    let source = workspace.path().join("src");
    let support = source.join("Ext/ParentConfigurations.bin");
    let project = workspace.path().join(".v8-project.json");
    let support_bytes = concat!(
        "\u{feff}{6,1,1,dddddddd-dddd-dddd-dddd-dddddddddddd,0,",
        "eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee,\"1.0\",\"Vendor\",",
        "\"VendorConf\",0,0,0}"
    )
    .as_bytes()
    .to_vec();
    std::fs::write(&support, support_bytes).unwrap();
    std::fs::write(&project, r#"{"editingAllowedCheck":"off"}"#).unwrap();
    let expected = tree_snapshot(&source);
    let denied_project = br#"{"editingAllowedCheck":"deny"}"#.to_vec();
    let project_for_hook = project.clone();
    let denied_project_for_hook = denied_project.clone();

    let result = with_meta_remove_before_reauthorization_hook(
        move || std::fs::write(project_for_hook, denied_project_for_hook).unwrap(),
        || call_remove(workspace.path(), false),
    );

    assert!(
        !result.ok,
        "stale remove authorization unexpectedly published"
    );
    assert_eq!(
        result.diagnostics.as_ref().unwrap()[0]["code"],
        "support_locked",
        "{result:?}"
    );
    assert!(result.cache.events.is_empty());
    assert!(result.cache.invalidated.is_empty());
    assert!(result.cache.refreshed.is_empty());
    assert_eq!(tree_snapshot(&source), expected);
    assert_eq!(std::fs::read(project).unwrap(), denied_project);
}

#[test]
fn meta_remove_warning_is_derived_from_late_support_authorization_for_preview_and_apply() {
    for dry_run in [true, false] {
        let workspace = create_remove_workspace(if dry_run {
            "support-warning-drift-preview"
        } else {
            "support-warning-drift-apply"
        });
        let source = workspace.path().join("src");
        let descriptor = source.join("Catalogs/Removable.xml");
        let owner = source.join("Configuration.xml");
        let support = source.join("Ext/ParentConfigurations.bin");
        let project = workspace.path().join(".v8-project.json");
        std::fs::write(
            &support,
            concat!(
                "\u{feff}{6,1,1,dddddddd-dddd-dddd-dddd-dddddddddddd,0,",
                "eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee,\"1.0\",\"Vendor\",",
                "\"VendorConf\",0,0,0}"
            ),
        )
        .unwrap();
        std::fs::write(&project, r#"{"editingAllowedCheck":"off"}"#).unwrap();
        let source_before = tree_snapshot(&source);
        let project_for_hook = project.clone();

        let result = with_meta_remove_before_reauthorization_hook(
            move || std::fs::write(project_for_hook, r#"{"editingAllowedCheck":"warn"}"#).unwrap(),
            || call_remove(workspace.path(), dry_run),
        );

        assert!(result.ok, "dryRun={dry_run}: {result:?}");
        assert_eq!(
            result.data.as_ref().unwrap()["diagnostics"],
            serde_json::json!([{
                "code": "support_locked",
                "severity": "warning",
                "message": "metadata support policy permits removal with a warning",
                "metadataPath": "Catalog.Removable"
            }]),
            "dryRun={dry_run}"
        );
        assert_eq!(
            result.cache.mode,
            if dry_run { "dry-run" } else { "applied" }
        );
        assert_eq!(result.cache.events, ["MetadataChanged"]);
        assert_eq!(
            std::fs::read_to_string(&project).unwrap(),
            r#"{"editingAllowedCheck":"warn"}"#
        );
        if dry_run {
            assert_eq!(tree_snapshot(&source), source_before);
        } else {
            assert!(!descriptor.exists());
            assert!(!std::fs::read_to_string(owner)
                .unwrap()
                .contains("<Catalog>Removable</Catalog>"));
        }
    }
}

#[test]
fn meta_remove_reauthorizes_every_planned_subsystem_cleanup_before_mutations() {
    let workspace = create_remove_workspace("subsystem-support-authorization-drift");
    let source = workspace.path().join("src");
    let subsystem = source.join("Subsystems/Main.xml");
    std::fs::create_dir_all(subsystem.parent().unwrap()).unwrap();
    std::fs::write(
        &subsystem,
        concat!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
            "<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.20\">\n",
            "<Subsystem uuid=\"11111111-1111-1111-1111-111111111111\">\n",
            "<Properties><Name>Main</Name></Properties>\n",
            "<Content><Item>Catalog.Removable</Item></Content>\n",
            "</Subsystem>\n",
            "</MetaDataObject>"
        ),
    )
    .unwrap();
    let support = source.join("Ext/ParentConfigurations.bin");
    std::fs::write(
        &support,
        concat!(
            "\u{feff}{6,0,1,dddddddd-dddd-dddd-dddd-dddddddddddd,0,",
            "eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee,\"1.0\",\"Vendor\",",
            "\"VendorConf\",0,0,0}"
        ),
    )
    .unwrap();
    let locked_support = concat!(
        "\u{feff}{6,0,1,dddddddd-dddd-dddd-dddd-dddddddddddd,0,",
        "eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee,\"1.0\",\"Vendor\",",
        "\"VendorConf\",1,0,0,11111111-1111-1111-1111-111111111111,",
        "11111111-1111-1111-1111-111111111111}"
    )
    .as_bytes()
    .to_vec();
    let support_for_hook = support.clone();
    let locked_support_for_hook = locked_support.clone();
    let mut expected = tree_snapshot(&source);
    expected.insert(
        PathBuf::from("Ext/ParentConfigurations.bin"),
        locked_support.clone(),
    );

    let result = with_meta_remove_before_reauthorization_hook(
        move || std::fs::write(support_for_hook, locked_support_for_hook).unwrap(),
        || call_remove(workspace.path(), false),
    );

    assert!(
        !result.ok,
        "locked subsystem cleanup unexpectedly published"
    );
    let diagnostic = &result.diagnostics.as_ref().unwrap()[0];
    assert_eq!(diagnostic["code"], "support_locked", "{result:?}");
    assert_eq!(diagnostic["field"], "dependencies");
    assert!(diagnostic["message"]
        .as_str()
        .unwrap()
        .contains("subsystem `Main`"));
    assert!(result.cache.events.is_empty());
    assert_eq!(tree_snapshot(&source), expected);
}

#[test]
fn meta_remove_rejects_descriptor_identity_mismatch_before_effect_projection() {
    let workspace = create_remove_workspace("identity-mismatch");
    let descriptor = workspace.path().join("src/Catalogs/Removable.xml");
    let xml = std::fs::read_to_string(&descriptor).unwrap();
    let mismatched = xml.replacen("<Name>Removable</Name>", "<Name>Different</Name>", 1);
    assert_ne!(mismatched, xml, "fixture must expose descriptor identity");
    std::fs::write(&descriptor, mismatched).unwrap();

    let result = call_remove(workspace.path(), true);

    assert!(!result.ok);
    assert!(result.data.is_none());
    assert!(
        result
            .diagnostics
            .as_ref()
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .any(|diagnostic| diagnostic["code"] == "target_not_found"),
        "{:?}",
        result.diagnostics
    );
}
