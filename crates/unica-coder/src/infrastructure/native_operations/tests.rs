use super::NativeOperationAdapter;
use crate::infrastructure::native_operations::typed_result::NativeInvocationContext;
use crate::infrastructure::workspace::discover_workspace;
use serde_json::{json, Map};
use std::fs;
use std::time::{Duration, Instant};
use unica_work_control::cancellation::CancellationToken;
use unica_work_control::provider_deadline::ProviderDeadline;

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
        unica_metadata_designer::subsystem::child_subsystem_stub_xml(
            "Sales",
            "2.20",
            crate::infrastructure::source_ids::fresh_uuid,
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
        NativeInvocationContext::new(
            &crate::infrastructure::support_state::WorkspaceSupportStateReader::new(&context),
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

fn temp_root(name: &str) -> crate::infrastructure::temp_test_root::TempTestRoot {
    crate::infrastructure::temp_test_root::TempTestRoot::new(
        &format!("unica-native-ops-{name}"),
        "",
    )
}

/// The facade's own shape: reads this crate's sources, not a workspace.
mod medium_facade {
    use std::path::PathBuf;

    #[test]
    fn native_operations_rs_is_thin_facade_not_xml_dsl_monolith() {
        let infrastructure_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("infrastructure");
        let path = infrastructure_dir.join("native_operations.rs");
        let text = std::fs::read_to_string(&path).unwrap();
        let line_count = text.lines().count();

        assert!(
            line_count < 200,
            "native_operations.rs must stay a thin facade; got {line_count} lines"
        );
        assert!(
            !text.contains("match operation"),
            "operation-specific XML/DSL dispatch belongs in backend modules"
        );
        assert!(
            !infrastructure_dir
                .join("native_operations_backend.rs")
                .exists(),
            "native_operations_backend.rs must not return; split operation logic by family under native_operations/"
        );
    }
}
