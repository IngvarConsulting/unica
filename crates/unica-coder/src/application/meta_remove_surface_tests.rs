use super::{OperationResult, UnicaApplication};
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
        let previous = std::env::current_dir().unwrap();
        std::env::set_current_dir(workspace.path()).unwrap();
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
        std::env::set_current_dir(previous).unwrap();
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
    let previous = std::env::current_dir().unwrap();
    std::env::set_current_dir(workspace).unwrap();
    let result = UnicaApplication::new().call_tool("unica.meta.remove", &remove_args(dry_run));
    std::env::set_current_dir(previous).unwrap();
    result.expect("private typed meta.remove call")
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
