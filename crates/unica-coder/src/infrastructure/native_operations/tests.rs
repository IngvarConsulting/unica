use super::NativeOperationAdapter;
use crate::domain::cancellation::CancellationToken;
use crate::domain::code_intelligence::ProviderDeadline;
use crate::infrastructure::native_operations::typed_result::NativeInvocationControl;
use crate::infrastructure::workspace::discover_workspace;
use serde_json::{json, Map};
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

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
fn subsystem_info_cannot_use_the_uncontrolled_plain_dispatch_path() {
    let root = temp_root("subsystem-info-plain-dispatch");
    let descriptor = root.join("Sales.xml");
    fs::write(
        &descriptor,
        crate::infrastructure::native_operations::subsystem::child_subsystem_stub_xml(
            "Sales", "2.20",
        ),
    )
    .unwrap();
    let context = discover_workspace(Some(root.clone())).unwrap();
    let args = serde_json::from_value(json!({
        "SubsystemPath": descriptor.display().to_string()
    }))
    .unwrap();

    let error = NativeOperationAdapter::invoke(
        "subsystem-info",
        "unica.subsystem.info",
        &args,
        &context,
        false,
        false,
    )
    .expect_err("subsystem.info must require the controlled prepared path");

    assert!(error.contains("controlled prepared"), "{error}");
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

    let cancellation = CancellationToken::new();
    let result = NativeOperationAdapter::invoke_with_data(
        "cf-info",
        "unica.cf.info",
        &args,
        &context,
        false,
        false,
        &crate::infrastructure::support_state::WorkspaceSupportStateReader::new(&context),
        NativeInvocationControl::new(
            &cancellation,
            ProviderDeadline::new(Instant::now() + Duration::from_secs(5)),
        ),
    )
    .unwrap();

    assert!(result.adapter.ok, "{:?}", result.adapter);
    assert_eq!(fs::read(&config_path).unwrap(), original);
    // ADR-0023: the answer is data, and a legacy output sink still changes
    // nothing on disk.
    assert_eq!(result.data.unwrap()["name"], "ReadOnlyContract");
    assert!(result.adapter.stdout.is_none());

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
