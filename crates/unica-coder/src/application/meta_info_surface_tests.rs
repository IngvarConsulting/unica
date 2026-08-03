use super::{OperationResult, UnicaApplication};
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};
use uuid::Uuid;

struct TempWorkspace(PathBuf);

impl TempWorkspace {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "unica-platform-meta-info-{label}-{}-{}",
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

fn create_info_workspace(label: &str) -> TempWorkspace {
    let workspace = TempWorkspace::new(label);
    let initialized = UnicaApplication::new()
        .call_tool(
            "unica.cf.init",
            &Map::from_iter([
                (
                    "cwd".to_string(),
                    Value::String(workspace.path().display().to_string()),
                ),
                ("Name".to_string(), Value::String("MetaInfo".to_string())),
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
    let definition = workspace.path().join("info-fixture.json");
    std::fs::write(
        &definition,
        serde_json::to_vec(&serde_json::json!([{
            "type": "Catalog",
            "name": "Inspectable",
            "synonym": "Inspectable synonym",
            "attributes": ["Code: String(9)", "Amount: Number(15,2)"],
            "tabularSections": [{"name": "Rows", "attributes": ["Value: String(20)"]}]
        }]))
        .unwrap(),
    )
    .unwrap();
    let compiled = UnicaApplication::new()
        .call_tool(
            "unica.meta.compile",
            &Map::from_iter([
                (
                    "cwd".to_string(),
                    Value::String(workspace.path().display().to_string()),
                ),
                (
                    "JsonPath".to_string(),
                    Value::String(definition.display().to_string()),
                ),
                ("OutputDir".to_string(), Value::String("src".to_string())),
                ("dryRun".to_string(), Value::Bool(false)),
            ]),
        )
        .unwrap();
    assert!(compiled.ok, "{:?}", compiled.errors);
    workspace
}

fn call_info(
    workspace: &Path,
    extra: impl IntoIterator<Item = (String, Value)>,
) -> OperationResult {
    call_info_path(workspace, "Catalog.Inspectable", extra)
}

fn call_info_path(
    workspace: &Path,
    metadata_path: &str,
    extra: impl IntoIterator<Item = (String, Value)>,
) -> OperationResult {
    let mut args = Map::from_iter([
        ("sourceSet".to_string(), Value::String("main".to_string())),
        (
            "metadataPath".to_string(),
            Value::String(metadata_path.to_string()),
        ),
    ]);
    args.extend(extra);
    let previous = std::env::current_dir().unwrap();
    std::env::set_current_dir(workspace).unwrap();
    let result = UnicaApplication::new().call_unregistered_meta_info_for_integration_tests(&args);
    std::env::set_current_dir(previous).unwrap();
    result.expect("private typed meta.info call")
}

fn assert_logical_diagnostic(result: &OperationResult, workspace: &Path, code: &str) {
    let diagnostics = result
        .diagnostics
        .as_ref()
        .and_then(Value::as_array)
        .expect("structured diagnostics");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic["code"] == code),
        "missing {code} diagnostic: {diagnostics:?}"
    );
    assert!(
        !serde_json::to_string(diagnostics)
            .unwrap()
            .contains(&workspace.display().to_string()),
        "diagnostics must expose logical identities only"
    );
}

fn fixture_workspace(label: &str, fixture: &str, files: &[&str]) -> TempWorkspace {
    let workspace = TempWorkspace::new(label);
    std::fs::write(
        workspace.path().join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
    )
    .unwrap();
    let fixture_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/unica_mcp_script_parity")
        .join(fixture);
    for relative in files {
        let destination = workspace.path().join("src").join(relative);
        std::fs::create_dir_all(destination.parent().unwrap()).unwrap();
        std::fs::copy(fixture_root.join(relative), destination).unwrap();
    }
    workspace
}

#[test]
fn meta_info_private_coordinator_returns_local_structure_validation_and_default_related_sections() {
    let workspace = create_info_workspace("local");

    let result = call_info(
        workspace.path(),
        [("limit".to_string(), Value::Number(1_u64.into()))],
    );

    assert!(result.ok, "{:?}", result.errors);
    let data = result.data.expect("typed metadata info data");
    assert_eq!(data["metadataPath"], "Catalog.Inspectable");
    assert_eq!(data["kind"], "Catalog");
    assert_eq!(data["name"], "Inspectable");
    assert_eq!(data["synonym"], "Inspectable synonym");
    assert_eq!(data["support"], "supported");
    assert!(!data["properties"].as_array().unwrap().is_empty());
    assert!(data["owners"].as_array().unwrap().is_empty());
    assert_eq!(data["validation"]["status"], "passed");
    assert_eq!(
        data["collections"]["attributes"].as_array().unwrap().len(),
        2
    );
    assert_eq!(
        data["collections"]["tabularSections"][0]["attributes"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        data["collections"]["attributes"][0]["type"]["variants"][0]["kind"],
        "string"
    );
    for collection in [
        "dimensions",
        "resources",
        "enumValues",
        "columns",
        "forms",
        "templates",
        "commands",
    ] {
        assert!(data["collections"][collection].is_array(), "{collection}");
    }
    assert!(data["related"]["modules"].is_object());
    assert!(data["related"]["roles"].is_object());
    assert!(data["related"]["subscriptions"].is_object());
    assert!(data["related"]["functionalOptions"].is_object());
    assert!(data["related"].get("predefinedItems").is_none());
    assert!(result.stdout.is_none());
}

#[test]
fn meta_info_private_coordinator_hard_fails_malformed_xml_but_keeps_semantic_failure_data() {
    let malformed = create_info_workspace("malformed");
    std::fs::write(
        malformed.path().join("src/Catalogs/Inspectable.xml"),
        b"<MetaDataObject><Catalog>",
    )
    .unwrap();

    let malformed_result = call_info(malformed.path(), []);
    assert!(!malformed_result.ok);
    assert!(malformed_result.data.is_none());

    let semantic = create_info_workspace("semantic");
    let descriptor = semantic.path().join("src/Catalogs/Inspectable.xml");
    let xml = std::fs::read_to_string(&descriptor).unwrap();
    let duplicate = xml.replacen("<Name>Amount</Name>", "<Name>Code</Name>", 1);
    assert_ne!(duplicate, xml, "fixture must contain the second attribute");
    std::fs::write(&descriptor, duplicate).unwrap();

    let semantic_result = call_info(semantic.path(), []);
    assert!(!semantic_result.ok);
    assert_eq!(
        semantic_result.data.as_ref().unwrap()["validation"]["status"],
        "failed"
    );
    assert_eq!(
        semantic_result.data.as_ref().unwrap()["name"],
        "Inspectable"
    );
}

#[test]
fn meta_info_private_closed_read_marks_a_malformed_attribute_incomplete_and_failed() {
    let workspace = create_info_workspace("malformed-attribute");
    let descriptor = workspace.path().join("src/Catalogs/Inspectable.xml");
    let mut xml = std::fs::read_to_string(&descriptor).unwrap();
    let start = xml.find("<Attribute uuid=").expect("compiled attribute");
    let end = start
        + xml[start..]
            .find("</Attribute>")
            .expect("compiled attribute end")
        + "</Attribute>".len();
    xml.replace_range(start..end, "<Attribute>Broken</Attribute>");
    std::fs::write(&descriptor, xml).unwrap();

    let result = call_info(workspace.path(), []);

    assert!(!result.ok);
    let data = result
        .data
        .as_ref()
        .expect("local metadata remains available");
    assert_eq!(data["name"], "Inspectable");
    assert_eq!(data["collections"]["attributes"][0]["name"], "Broken");
    assert_eq!(data["collections"]["attributes"][0]["incomplete"], true);
    assert_eq!(data["validation"]["status"], "failed");
    assert_logical_diagnostic(&result, workspace.path(), "validation_failed");
}

#[test]
fn meta_info_private_closed_read_proof_requires_every_registered_language_image() {
    let files = [
        "Configuration.xml",
        "Enums/LanguageAware.xml",
        "Languages/English.xml",
        "Languages/Русский.xml",
    ];
    let valid = fixture_workspace("language-valid", "meta-validate-language-aware", &files);
    let valid_result = call_info_path(valid.path(), "Enum.LanguageAware", []);
    assert_eq!(
        valid_result.data.as_ref().unwrap()["validation"]["status"],
        "passed"
    );

    let missing = fixture_workspace("language-missing", "meta-validate-language-aware", &files);
    std::fs::remove_file(missing.path().join("src/Languages/English.xml")).unwrap();
    let missing_result = call_info_path(missing.path(), "Enum.LanguageAware", []);

    assert!(!missing_result.ok);
    assert_eq!(
        missing_result.data.as_ref().unwrap()["validation"]["status"],
        "failed"
    );
    assert!(missing_result.data.as_ref().unwrap()["name"].is_string());
    assert_logical_diagnostic(&missing_result, missing.path(), "provider_unavailable");

    let invalid = fixture_workspace("language-invalid", "meta-validate-language-aware", &files);
    std::fs::write(invalid.path().join("src/Languages/English.xml"), "not XML").unwrap();
    let invalid_result = call_info_path(invalid.path(), "Enum.LanguageAware", []);

    assert!(!invalid_result.ok);
    assert_eq!(
        invalid_result.data.as_ref().unwrap()["validation"]["status"],
        "failed"
    );
    assert!(invalid_result.data.as_ref().unwrap()["name"].is_string());
    assert_logical_diagnostic(&invalid_result, invalid.path(), "provider_unavailable");
}

#[test]
fn meta_info_private_closed_read_proof_rejects_a_missing_forward_register_image() {
    let files = [
        "Configuration.xml",
        "Documents/Регистратор.xml",
        "InformationRegisters/SubordinateRegister.xml",
        "Languages/Русский.xml",
    ];
    let workspace = fixture_workspace(
        "missing-forward-register",
        "meta-validate-subordinate-register",
        &files,
    );
    std::fs::remove_file(
        workspace
            .path()
            .join("src/InformationRegisters/SubordinateRegister.xml"),
    )
    .unwrap();

    let result = call_info_path(workspace.path(), "Document.Регистратор", []);

    assert!(!result.ok);
    assert_eq!(
        result.data.as_ref().unwrap()["validation"]["status"],
        "failed"
    );
    assert_eq!(result.data.as_ref().unwrap()["name"], "Регистратор");
    assert_logical_diagnostic(&result, workspace.path(), "provider_unavailable");
}

#[test]
fn meta_info_private_closed_read_proof_rejects_reverse_registrar_inconsistency() {
    let files = [
        "Configuration.xml",
        "Documents/Регистратор.xml",
        "InformationRegisters/SubordinateRegister.xml",
        "Languages/Русский.xml",
    ];
    let workspace = fixture_workspace(
        "reverse-registrar",
        "meta-validate-subordinate-register",
        &files,
    );
    let registrar = workspace.path().join("src/Documents/Регистратор.xml");
    let bytes = std::fs::read_to_string(&registrar).unwrap().replace(
        "InformationRegister.SubordinateRegister",
        "InformationRegister.Other",
    );
    std::fs::write(&registrar, bytes).unwrap();

    let result = call_info_path(
        workspace.path(),
        "InformationRegister.SubordinateRegister",
        [],
    );

    assert!(!result.ok);
    assert_eq!(
        result.data.as_ref().unwrap()["validation"]["status"],
        "failed"
    );
    assert_eq!(result.data.as_ref().unwrap()["name"], "SubordinateRegister");
    assert_logical_diagnostic(&result, workspace.path(), "validation_failed");
}
