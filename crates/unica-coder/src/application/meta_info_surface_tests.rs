use super::{compile_legacy_metadata_fixture, OperationResult, UnicaApplication};
use crate::composition::testing::{with_registrar_processing_hook, RegistrarProcessingPhase};
use crate::domain::cancellation::CancellationToken;
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use uuid::Uuid;

struct TempWorkspace(PathBuf);

fn process_cwd_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

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
    let _cwd_lock = process_cwd_lock();
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
    let compiled = compile_legacy_metadata_fixture(&Map::from_iter([
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
    ]))
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
    let _cwd_lock = process_cwd_lock();
    let previous = std::env::current_dir().unwrap();
    std::env::set_current_dir(workspace).unwrap();
    let result = UnicaApplication::new().call_tool("unica.meta.info", &args);
    std::env::set_current_dir(previous).unwrap();
    result.expect("private typed meta.info call")
}

fn call_info_path_cancellable(
    workspace: &Path,
    metadata_path: &str,
    extra: impl IntoIterator<Item = (String, Value)>,
    cancellation: CancellationToken,
) -> OperationResult {
    let mut args = Map::from_iter([
        ("sourceSet".to_string(), Value::String("main".to_string())),
        (
            "metadataPath".to_string(),
            Value::String(metadata_path.to_string()),
        ),
    ]);
    args.extend(extra);
    let _cwd_lock = process_cwd_lock();
    let previous = std::env::current_dir().unwrap();
    std::env::set_current_dir(workspace).unwrap();
    let result =
        UnicaApplication::new().call_tool_cancellable("unica.meta.info", &args, cancellation);
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

fn assert_no_error_diagnostic(result: &OperationResult, code: &str) {
    let Some(diagnostics) = result.diagnostics.as_ref().and_then(Value::as_array) else {
        return;
    };
    assert!(
        diagnostics
            .iter()
            .all(|diagnostic| diagnostic["code"] != code || diagnostic["severity"] != "error"),
        "unexpected {code} error diagnostic: {diagnostics:?}"
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
fn meta_info_private_closed_read_preserves_missing_and_empty_child_names_as_incomplete() {
    let workspace = create_info_workspace("malformed-attribute");
    let descriptor = workspace.path().join("src/Catalogs/Inspectable.xml");
    let mut xml = std::fs::read_to_string(&descriptor).unwrap();
    for replacement in [
        "<Attribute><Properties><Synonym/></Properties></Attribute>",
        "<Attribute><Properties><Name> </Name></Properties></Attribute>",
    ] {
        let start = xml.find("<Attribute uuid=").expect("compiled attribute");
        let end = start
            + xml[start..]
                .find("</Attribute>")
                .expect("compiled attribute end")
            + "</Attribute>".len();
        xml.replace_range(start..end, replacement);
    }
    std::fs::write(&descriptor, xml).unwrap();

    let result = call_info(workspace.path(), []);

    assert!(!result.ok);
    let data = result
        .data
        .as_ref()
        .expect("local metadata remains available");
    assert_eq!(data["name"], "Inspectable");
    assert_eq!(
        data["collections"]["attributes"].as_array().unwrap().len(),
        2
    );
    assert_eq!(data["collections"]["attributes"][0]["name"], "");
    assert_eq!(data["collections"]["attributes"][0]["incomplete"], true);
    assert_eq!(data["collections"]["attributes"][1]["name"], "");
    assert_eq!(data["collections"]["attributes"][1]["incomplete"], true);
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
    assert_no_error_diagnostic(&result, "provider_unavailable");
}

#[test]
fn meta_info_private_closed_read_accepts_complete_registrar_evidence() {
    let files = [
        "Configuration.xml",
        "Documents/Регистратор.xml",
        "InformationRegisters/SubordinateRegister.xml",
        "Languages/Русский.xml",
    ];
    let workspace = fixture_workspace(
        "registrar-complete",
        "meta-validate-subordinate-register",
        &files,
    );

    let result = call_info_path(
        workspace.path(),
        "InformationRegister.SubordinateRegister",
        [],
    );

    assert!(result.ok, "{result:?}");
    assert_eq!(
        result.data.as_ref().unwrap()["validation"]["status"],
        "passed"
    );
    assert_no_error_diagnostic(&result, "provider_unavailable");
    assert_no_error_diagnostic(&result, "validation_failed");
}

#[test]
fn meta_info_private_closed_read_treats_absent_and_empty_documents_as_complete_empty_graphs() {
    let files = [
        "Configuration.xml",
        "InformationRegisters/SubordinateRegister.xml",
        "Languages/Русский.xml",
    ];
    let absent = fixture_workspace(
        "registrar-documents-absent",
        "meta-validate-subordinate-register",
        &files,
    );
    let absent_result =
        call_info_path(absent.path(), "InformationRegister.SubordinateRegister", []);
    let empty = fixture_workspace(
        "registrar-documents-empty",
        "meta-validate-subordinate-register",
        &files,
    );
    std::fs::create_dir_all(empty.path().join("src/Documents")).unwrap();
    let empty_result = call_info_path(empty.path(), "InformationRegister.SubordinateRegister", []);

    for (result, workspace) in [
        (&absent_result, absent.path()),
        (&empty_result, empty.path()),
    ] {
        assert!(!result.ok);
        assert_eq!(result.data.as_ref().unwrap()["name"], "SubordinateRegister");
        assert_eq!(
            result.data.as_ref().unwrap()["validation"]["status"],
            "failed"
        );
        assert_logical_diagnostic(result, workspace, "validation_failed");
        assert_no_error_diagnostic(result, "provider_unavailable");
    }
}

#[test]
fn meta_info_private_closed_read_registrar_scan_enforces_byte_cap() {
    let files = [
        "Configuration.xml",
        "Documents/Регистратор.xml",
        "InformationRegisters/SubordinateRegister.xml",
        "Languages/Русский.xml",
    ];
    let workspace = fixture_workspace(
        "registrar-byte-cap",
        "meta-validate-subordinate-register",
        &files,
    );
    std::fs::File::create(workspace.path().join("src/Documents/000-Oversized.xml"))
        .unwrap()
        .set_len(64 * 1024 * 1024 + 1)
        .unwrap();

    let result = call_info_path(
        workspace.path(),
        "InformationRegister.SubordinateRegister",
        [],
    );

    assert!(!result.ok);
    assert_logical_diagnostic(&result, workspace.path(), "provider_unavailable");
    assert_no_error_diagnostic(&result, "validation_failed");
}

#[test]
fn meta_info_private_closed_read_reports_malformed_registrar_evidence_unavailable() {
    let files = [
        "Configuration.xml",
        "Documents/Регистратор.xml",
        "InformationRegisters/SubordinateRegister.xml",
        "Languages/Русский.xml",
    ];
    let workspace = fixture_workspace(
        "registrar-malformed",
        "meta-validate-subordinate-register",
        &files,
    );
    std::fs::write(
        workspace.path().join("src/Documents/Регистратор.xml"),
        "not XML",
    )
    .unwrap();

    let result = call_info_path(
        workspace.path(),
        "InformationRegister.SubordinateRegister",
        [],
    );

    assert!(!result.ok);
    assert_eq!(result.data.as_ref().unwrap()["name"], "SubordinateRegister");
    assert_logical_diagnostic(&result, workspace.path(), "provider_unavailable");
    assert_no_error_diagnostic(&result, "validation_failed");
}

#[test]
fn meta_info_post_capture_cancellation_after_first_identity_parse_is_provider_unavailable() {
    let files = [
        "Configuration.xml",
        "Documents/Регистратор.xml",
        "InformationRegisters/SubordinateRegister.xml",
        "Languages/Русский.xml",
    ];
    let workspace = fixture_workspace(
        "registrar-first-identity-cancel",
        "meta-validate-subordinate-register",
        &files,
    );
    let cancellation = CancellationToken::new();
    let cancellation_for_hook = cancellation.clone();

    let result = with_registrar_processing_hook(
        move |phase| {
            if matches!(
                phase,
                RegistrarProcessingPhase::AfterIdentityParse { ordinal: 0, .. }
            ) {
                cancellation_for_hook.cancel();
            }
        },
        || {
            call_info_path_cancellable(
                workspace.path(),
                "InformationRegister.SubordinateRegister",
                [],
                cancellation,
            )
        },
    );

    assert!(!result.ok);
    assert_eq!(result.data.as_ref().unwrap()["name"], "SubordinateRegister");
    assert_logical_diagnostic(&result, workspace.path(), "provider_unavailable");
    assert_no_error_diagnostic(&result, "validation_failed");
}

#[test]
fn meta_info_post_capture_cancellation_after_large_identity_parse_is_provider_unavailable() {
    let files = [
        "Configuration.xml",
        "Documents/Регистратор.xml",
        "InformationRegisters/SubordinateRegister.xml",
        "Languages/Русский.xml",
    ];
    let workspace = fixture_workspace(
        "registrar-large-identity-cancel",
        "meta-validate-subordinate-register",
        &files,
    );
    let original =
        std::fs::read_to_string(workspace.path().join("src/Documents/Регистратор.xml")).unwrap();
    let inflated = original.replace(
        "</MetaDataObject>",
        &format!("<!--{}--></MetaDataObject>", "x".repeat(4 * 1024 * 1024)),
    );
    std::fs::write(
        workspace.path().join("src/Documents/000-Large.xml"),
        inflated,
    )
    .unwrap();
    let cancellation = CancellationToken::new();
    let cancellation_for_hook = cancellation.clone();

    let result = with_registrar_processing_hook(
        move |phase| {
            if matches!(
                phase,
                RegistrarProcessingPhase::AfterIdentityParse { logical_path, .. }
                    if logical_path.ends_with("000-Large.xml")
            ) {
                cancellation_for_hook.cancel();
            }
        },
        || {
            call_info_path_cancellable(
                workspace.path(),
                "InformationRegister.SubordinateRegister",
                [],
                cancellation,
            )
        },
    );

    assert!(!result.ok);
    assert_eq!(result.data.as_ref().unwrap()["name"], "SubordinateRegister");
    assert_logical_diagnostic(&result, workspace.path(), "provider_unavailable");
    assert_no_error_diagnostic(&result, "validation_failed");
}

#[test]
fn meta_info_post_capture_cancellation_after_last_registrar_parse_is_provider_unavailable() {
    let files = [
        "Configuration.xml",
        "Documents/Регистратор.xml",
        "InformationRegisters/SubordinateRegister.xml",
        "Languages/Русский.xml",
    ];
    let workspace = fixture_workspace(
        "registrar-last-parse-cancel",
        "meta-validate-subordinate-register",
        &files,
    );
    std::fs::copy(
        workspace.path().join("src/Documents/Регистратор.xml"),
        workspace.path().join("src/Documents/Z-Second.xml"),
    )
    .unwrap();
    let cancellation = CancellationToken::new();
    let cancellation_for_hook = cancellation.clone();

    let result = with_registrar_processing_hook(
        move |phase| {
            if matches!(
                phase,
                RegistrarProcessingPhase::AfterRegistrarParse {
                    ordinal,
                    total,
                    ..
                } if ordinal + 1 == *total
            ) {
                cancellation_for_hook.cancel();
            }
        },
        || {
            call_info_path_cancellable(
                workspace.path(),
                "InformationRegister.SubordinateRegister",
                [],
                cancellation,
            )
        },
    );

    assert!(!result.ok);
    assert_eq!(result.data.as_ref().unwrap()["name"], "SubordinateRegister");
    assert_logical_diagnostic(&result, workspace.path(), "provider_unavailable");
    assert_no_error_diagnostic(&result, "validation_failed");
}

#[test]
fn meta_info_post_capture_cancellation_before_complete_return_is_provider_unavailable() {
    let files = [
        "Configuration.xml",
        "Documents/Регистратор.xml",
        "InformationRegisters/SubordinateRegister.xml",
        "Languages/Русский.xml",
    ];
    let workspace = fixture_workspace(
        "registrar-final-return-cancel",
        "meta-validate-subordinate-register",
        &files,
    );
    let cancellation = CancellationToken::new();
    let cancellation_for_hook = cancellation.clone();

    let result = with_registrar_processing_hook(
        move |phase| {
            if phase == &RegistrarProcessingPhase::BeforeCompleteReturn {
                cancellation_for_hook.cancel();
            }
        },
        || {
            call_info_path_cancellable(
                workspace.path(),
                "InformationRegister.SubordinateRegister",
                [],
                cancellation,
            )
        },
    );

    assert!(!result.ok);
    assert_eq!(result.data.as_ref().unwrap()["name"], "SubordinateRegister");
    assert_logical_diagnostic(&result, workspace.path(), "provider_unavailable");
    assert_no_error_diagnostic(&result, "validation_failed");
}
