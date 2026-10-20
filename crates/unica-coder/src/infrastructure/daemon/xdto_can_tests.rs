use super::actor_capacity_tests::direct_v5;
use super::*;
use crate::application::invocation_store::ToolIdentity;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

const PACKAGE: &str = "main:XDTOPackage.EnterpriseData_1_17_3";
const OBJECT: &str = "main:XDTOPackage.EnterpriseData_1_17_3.Type.EnterpriseData";

struct Fixture {
    _root: tempfile::TempDir,
    workspace: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let workspace = crate::test_support::canonical_path(root.path()).join("workspace");
        let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap();
        for (relative, bytes) in crate::test_support::tree_snapshot(
            &repository.join("tests/fixtures/acceptance/workspace"),
        ) {
            let path = workspace.join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, bytes).unwrap();
        }
        Self {
            _root: root,
            workspace,
        }
    }
}

fn assert_contract_and_preview(at: &str, operation: &str, arguments: Value, has_values: bool) {
    let fixture = Fixture::new();
    let runtime = V5CanonicalInvocationRuntime::new(
        Arc::new(crate::infrastructure::daemon::v13_service::CanonicalV13ReadService::default()),
        Arc::new(TokioClock),
    );
    let before = crate::test_support::tree_snapshot(&fixture.workspace);
    let call = |tool, arguments| {
        direct_v5(
            &runtime,
            InvocationRequest::new(tool, arguments, fixture.workspace.to_string_lossy(), 7_000)
                .unwrap(),
        )
        .unwrap()
    };
    let owner = call(ToolIdentity::View, json!({"at": at}));
    assert!(
        owner.ok,
        "the canonical fixture must resolve {at}: {owner:?}"
    );
    let operation_at = format!("{at}.Can.{operation}");
    let detailed = call(ToolIdentity::View, json!({"at": operation_at}));
    assert!(detailed.ok, "detailed XDTO Can must resolve: {detailed:?}");
    assert_eq!(detailed.at.as_deref(), Some(operation_at.as_str()));
    let data = detailed.data.unwrap();
    assert_eq!(data["at"], operation_at);
    assert_eq!(data["kind"], "Can");
    assert_eq!(data["props"]["op"], operation);
    assert_eq!(data["props"]["implemented"], true);
    let contract = &data["props"]["contract"];
    let schema = &contract["argsSchema"];
    assert_eq!(schema["type"], "object");
    assert_eq!(schema["additionalProperties"], false);
    assert!(contract["exampleArgs"].is_object());
    assert!(contract["exampleArgs"].get("at").is_none());
    let oracle = jsonschema::validator_for(schema).expect("XDTO Can publishes valid JSON Schema");
    assert!(oracle.is_valid(&contract["exampleArgs"]));
    assert!(oracle.is_valid(&arguments), "{operation}: {arguments}");
    assert!(!oracle.is_valid(&json!({"unknown": true})));
    if has_values {
        assert_eq!(schema["required"], json!(["values"]));
        assert_eq!(
            schema["properties"]["values"]["additionalProperties"],
            false
        );
        assert!(!oracle.is_valid(&json!({})));
    } else {
        assert_eq!(contract["exampleArgs"], json!({}));
        assert!(!oracle.is_valid(&json!({"values": {}})));
    }
    assert_eq!(
        crate::test_support::tree_snapshot(&fixture.workspace),
        before
    );
    let preview = call(
        ToolIdentity::Apply,
        json!({"at": at, "ops": [{"op": operation, "args": arguments}]}),
    );
    assert!(preview.ok, "real {operation} preview at {at}: {preview:?}");
    let planned = preview.data.unwrap();
    assert_eq!(planned["mode"], "preview");
    assert!(planned["executionToken"]
        .as_str()
        .is_some_and(|token| !token.is_empty()));
    assert!(planned["effects"]
        .as_u64()
        .is_some_and(|effects| effects > 0));
    assert_eq!(
        crate::test_support::tree_snapshot(&fixture.workspace),
        before
    );
}

#[test]
fn value_type_add_can_is_closed_and_previews_without_writes() {
    assert_contract_and_preview(
        PACKAGE,
        "valueType.add",
        json!({"values": {"name": "XdtoContractValue", "base": "xs:string"}}),
        true,
    );
}

#[test]
fn object_type_add_can_is_closed_and_previews_without_writes() {
    assert_contract_and_preview(
        PACKAGE,
        "objectType.add",
        json!({"values": {"name": "XdtoContractObject"}}),
        true,
    );
}

#[test]
fn property_add_can_is_closed_and_previews_without_writes() {
    assert_contract_and_preview(
        OBJECT,
        "property.add",
        json!({"values": {"property": {"name": "XdtoContractProperty", "type": "xs:string", "minOccurs": 0}}}),
        true,
    );
}

#[test]
fn type_remove_can_has_no_payload_and_previews_without_writes() {
    assert_contract_and_preview(
        "main:XDTOPackage.EnterpriseData_1_17_3.Type.СоставнойЛюбойОбъект",
        "type.remove",
        json!({}),
        false,
    );
}

#[test]
fn property_remove_can_has_no_payload_and_previews_without_writes() {
    assert_contract_and_preview(
        "main:XDTOPackage.EnterpriseData_1_17_3.Type.EnterpriseData.Property.Версия",
        "property.remove",
        json!({}),
        false,
    );
}
#[test]
fn forward_reference_is_validated_on_the_final_public_package_without_writes() {
    let fixture = Fixture::new();
    let runtime = V5CanonicalInvocationRuntime::new(
        Arc::new(crate::infrastructure::daemon::v13_service::CanonicalV13ReadService::default()),
        Arc::new(TokioClock),
    );
    let before = crate::test_support::tree_snapshot(&fixture.workspace);
    let request = InvocationRequest::new(
        ToolIdentity::Apply,
        json!({
            "at": PACKAGE,
            "ops": [
                {"op": "valueType.add", "args": {"values": {"name": "XdtoFinalA", "base": "tns:XdtoLaterB"}}},
                {"op": "valueType.add", "args": {"values": {"name": "XdtoLaterB", "base": "xs:string"}}}
            ]
        }),
        fixture.workspace.to_string_lossy(),
        7_000,
    )
    .unwrap();
    let preview = direct_v5(&runtime, request).unwrap();
    assert_eq!(
        crate::test_support::tree_snapshot(&fixture.workspace),
        before
    );
    assert!(
        preview.ok,
        "the final package resolves the later type: {preview:?}"
    );
    let planned = preview.data.unwrap();
    assert_eq!(planned["mode"], "preview");
    assert!(planned["effects"]
        .as_u64()
        .is_some_and(|effects| effects > 0));
    assert!(planned["executionToken"]
        .as_str()
        .is_some_and(|token| !token.is_empty()));
}
