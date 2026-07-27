use super::NativeOperationAdapter;
use crate::infrastructure::workspace::discover_workspace;
use serde_json::{json, Map};
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn missing_native_mutation_handler_is_contract_error() {
    let root = temp_root("missing-mutation-handler");
    fs::create_dir_all(root.join("src")).unwrap();
    let context = discover_workspace(Some(root.clone())).unwrap();

    let result = NativeOperationAdapter::invoke(
        "definitely-missing-operation",
        "unica.definitely.missing",
        &Map::new(),
        &context,
        false,
        true,
    );

    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .contains("native mutation handler is not registered"));
}

#[test]
fn code_patch_cannot_use_the_data_dropping_plain_dispatch_path() {
    let root = temp_root("typed-code-patch-dispatch");
    let context = discover_workspace(Some(root.clone())).unwrap();

    let error = NativeOperationAdapter::invoke(
        "code-patch",
        "unica.code.patch",
        &Map::new(),
        &context,
        true,
        true,
    )
    .unwrap_err();

    assert!(error.contains("typed native-operation result path"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn compile_preview_without_payload_uses_the_safe_dry_run_placeholder() {
    let root = temp_root("compile-preview-fallback");
    let context = discover_workspace(Some(root.clone())).unwrap();

    let result = NativeOperationAdapter::invoke(
        "meta-compile",
        "unica.meta.compile",
        &Map::new(),
        &context,
        true,
        true,
    )
    .expect("a missing preview payload must preserve the legacy dry-run contract");

    assert!(result.ok);
    assert!(result.summary.contains("dry run"));
    assert_eq!(
        result.changes,
        vec!["no files changed because dryRun is true".to_string()]
    );
    assert!(result.artifacts.is_empty());
    assert!(result
        .warnings
        .iter()
        .any(|warning| warning.contains("detailed compile preview is unavailable")));
    assert!(fs::read_dir(&root).unwrap().next().is_none());

    fs::remove_dir_all(root).unwrap();
}

/// Verifies the facade routes meta-edit dry-runs to the detailed preview.
#[test]
fn meta_edit_dry_run_dispatches_to_projected_diff_preview() {
    let root = temp_root("meta-edit-dry-run-dispatch");
    let object_path = root.join("Catalogs/Preview.xml");
    fs::create_dir_all(object_path.parent().unwrap()).unwrap();
    let original = br#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:cfg="http://v8.1c.ru/8.1/data/enterprise/current-config" xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" version="2.20">
	<Catalog uuid="11111111-1111-4111-8111-111111111111">
		<Properties>
			<Name>Preview</Name>
			<Synonym/>
			<Comment/>
			<Owners/>
			<InputByString/>
			<BasedOn/>
		</Properties>
		<ChildObjects/>
	</Catalog>
</MetaDataObject>
"#;
    fs::write(&object_path, original).unwrap();
    let context = discover_workspace(Some(root.clone())).unwrap();
    let args = serde_json::from_value(json!({
        "ObjectPath": object_path.display().to_string(),
        "Operation": "modify-property",
        "Value": "Comment=Dispatched"
    }))
    .unwrap();

    let result =
        NativeOperationAdapter::invoke("meta-edit", "unica.meta.edit", &args, &context, true, true)
            .unwrap();

    assert!(result.ok, "{result:?}");
    let stdout = result.stdout.as_deref().unwrap_or_default();
    assert!(stdout.contains("--- a/"), "{stdout}");
    assert!(stdout.contains("-\t\t\t<Comment/>"), "{stdout}");
    assert!(
        stdout.contains("+\t\t\t<Comment>Dispatched</Comment>"),
        "{stdout}"
    );
    assert_eq!(fs::read(&object_path).unwrap(), original);

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn subsystem_preview_with_unavailable_parent_uses_the_legacy_placeholder() {
    let root = temp_root("subsystem-preview-parent-fallback");
    let context = discover_workspace(Some(root.clone())).unwrap();
    let args = serde_json::from_value(serde_json::json!({
        "OutputDir": root.display().to_string(),
        "Value": r#"{"name":"Child"}"#,
        "Parent": "Subsystems/Missing.xml"
    }))
    .unwrap();

    let result = NativeOperationAdapter::invoke(
        "subsystem-compile",
        "unica.subsystem.compile",
        &args,
        &context,
        true,
        true,
    )
    .unwrap();

    assert!(result.ok);
    assert!(result.summary.contains("dry run"));
    assert!(result.warnings[0].contains("parent subsystem is unavailable"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn read_only_native_dispatch_does_not_honor_legacy_outfile() {
    let root = temp_root("read-only-outfile");
    let config_path = root.join("Configuration.xml");
    let original = br#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20">
  <Configuration>
    <Properties>
      <Name>ReadOnlyContract</Name>
    </Properties>
    <ChildObjects/>
  </Configuration>
</MetaDataObject>
"#;
    fs::write(&config_path, original).unwrap();
    let context = discover_workspace(Some(root.clone())).unwrap();
    let args = serde_json::from_value(json!({
        "ConfigPath": "Configuration.xml",
        "Mode": "brief",
        "OutFile": "Configuration.xml"
    }))
    .unwrap();

    let result =
        NativeOperationAdapter::invoke("cf-info", "unica.cf.info", &args, &context, false, false)
            .unwrap();

    assert!(result.ok, "{result:?}");
    assert_eq!(fs::read(&config_path).unwrap(), original);
    assert!(result
        .stdout
        .as_deref()
        .is_some_and(|stdout| stdout.contains("ReadOnlyContract")));

    fs::remove_dir_all(root).unwrap();
}

fn temp_root(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("unica-native-ops-{name}-{nanos}"));
    fs::create_dir_all(&root).unwrap();
    root
}
