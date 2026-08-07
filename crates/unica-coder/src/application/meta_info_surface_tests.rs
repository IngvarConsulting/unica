use super::{OperationResult, UnicaApplication};
use crate::composition::testing::{with_registrar_processing_hook, RegistrarProcessingPhase};
use crate::domain::cancellation::CancellationToken;
use crate::test_support::ProcessCwdGuard;
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
    let _cwd = ProcessCwdGuard::enter(workspace.path()).unwrap();
    let added = UnicaApplication::new().call_tool(
        "unica.meta.add",
        &Map::from_iter([
            ("sourceSet".to_string(), Value::String("main".to_string())),
            ("kind".to_string(), Value::String("Catalog".to_string())),
            ("name".to_string(), Value::String("Inspectable".to_string())),
            ("dryRun".to_string(), Value::Bool(false)),
        ]),
    );
    let edited = UnicaApplication::new()
        .call_tool(
            "unica.meta.edit",
            &Map::from_iter([
                ("sourceSet".to_string(), Value::String("main".to_string())),
                (
                    "metadataPath".to_string(),
                    Value::String("Catalog.Inspectable".to_string()),
                ),
                (
                    "operations".to_string(),
                    serde_json::json!([
                        {"op": "setProperties", "values": {"Synonym": "Inspectable synonym"}},
                        {"op": "add", "collection": "attributes", "elements": [
                            {"name": "Code", "type": {"variants": [{"kind": "string", "length": 9, "allowedLength": "variable"}] }},
                            {"name": "Amount", "type": {"variants": [{"kind": "number", "digits": 15, "fraction": 2, "sign": "any"}] }}
                        ]},
                        {"op": "add", "collection": "tabularSections", "elements": [{
                            "name": "Rows",
                            "attributes": [{"name": "Value", "type": {"variants": [{"kind": "string", "length": 20, "allowedLength": "variable"}] }}]
                        }]}
                    ]),
                ),
                ("dryRun".to_string(), Value::Bool(false)),
            ]),
        );
    let added = added.unwrap();
    assert!(added.ok, "{:?}", added.errors);
    let edited = edited.unwrap();
    assert!(edited.ok, "{:?}", edited.errors);
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
    let _cwd = ProcessCwdGuard::enter(workspace).unwrap();
    UnicaApplication::new()
        .call_tool("unica.meta.info", &args)
        .expect("private typed meta.info call")
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
    let _cwd = ProcessCwdGuard::enter(workspace).unwrap();
    UnicaApplication::new()
        .call_tool_cancellable("unica.meta.info", &args, cancellation)
        .expect("private typed meta.info call")
}

fn call_meta_tool(workspace: &Path, tool: &str, args: Map<String, Value>) -> OperationResult {
    let _cwd = ProcessCwdGuard::enter(workspace).unwrap();
    UnicaApplication::new()
        .call_tool(tool, &args)
        .expect("private typed metadata call")
}

fn add_catalog(
    workspace: &Path,
    name: &str,
    operations: Option<Value>,
    dry_run: bool,
) -> OperationResult {
    let mut args = Map::from_iter([
        ("sourceSet".to_string(), Value::String("main".to_string())),
        ("kind".to_string(), Value::String("Catalog".to_string())),
        ("name".to_string(), Value::String(name.to_string())),
        ("dryRun".to_string(), Value::Bool(dry_run)),
    ]);
    if let Some(operations) = operations {
        args.insert("operations".to_string(), operations);
    }
    call_meta_tool(workspace, "unica.meta.add", args)
}

fn call_edit(
    workspace: &Path,
    metadata_path: &str,
    operations: Value,
    dry_run: bool,
) -> OperationResult {
    call_meta_tool(
        workspace,
        "unica.meta.edit",
        Map::from_iter([
            ("sourceSet".to_string(), Value::String("main".to_string())),
            (
                "metadataPath".to_string(),
                Value::String(metadata_path.to_string()),
            ),
            ("operations".to_string(), operations),
            ("dryRun".to_string(), Value::Bool(dry_run)),
        ]),
    )
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
fn info_without_sections_is_local_only() {
    let workspace = create_info_workspace("local");

    let result = call_info(
        workspace.path(),
        [("limit".to_string(), Value::Number(50_u64.into()))],
    );

    assert!(result.ok, "{:?}", result.errors);
    let data = result.data.expect("typed metadata info data");
    assert_eq!(data["metadataPath"], "Catalog.Inspectable");
    assert_eq!(data["kind"], "Catalog");
    assert_eq!(data["name"], "Inspectable");
    assert_eq!(data["synonym"], "Inspectable synonym");
    assert_eq!(data["support"], "supported");
    assert!(!data["properties"].as_array().unwrap().is_empty());
    assert!(data["relations"]["owners"].as_array().unwrap().is_empty());
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
    // `meta.info` no longer consults a code index at all, so there is no
    // section left that could be absent for provider reasons.
    assert!(data.get("related").is_none());
    assert_eq!(data["usage"], serde_json::json!({}));
    assert!(result.stdout.is_none());
}

#[test]
fn info_preserves_local_structure_when_child_resource_evidence_is_unavailable() {
    let workspace = create_info_workspace("child-evidence-unavailable");
    let edited = call_edit(
        workspace.path(),
        "Catalog.Inspectable",
        serde_json::json!([{
            "op": "add",
            "collection": "forms",
            "elements": [{"name": "Main"}]
        }]),
        false,
    );
    assert!(edited.ok, "{:?}", edited.errors);
    std::fs::write(
        workspace
            .path()
            .join("src/Catalogs/Inspectable/Forms/Main/unexpected.bin"),
        b"unexpected",
    )
    .unwrap();

    let result = call_info(workspace.path(), []);

    assert!(!result.ok);
    assert_eq!(result.data.as_ref().unwrap()["name"], "Inspectable");
    assert_eq!(
        result.data.as_ref().unwrap()["collections"]["forms"][0]["name"],
        "Main"
    );
    assert_logical_diagnostic(&result, workspace.path(), "provider_unavailable");
}

#[test]
fn info_observes_every_typed_mutation_field() {
    let workspace = create_info_workspace("round-trip-fields");
    let owner = add_catalog(workspace.path(), "Owner", None, false);
    assert!(owner.ok, "{:?}", owner.errors);
    let seeded = add_catalog(
        workspace.path(),
        "RoundTrip",
        Some(serde_json::json!([{
            "op": "add",
            "collection": "attributes",
            "elements": [
                {"name": "First"},
                {"name": "Existing", "type": {"variants": [{"kind": "string", "length": 8, "allowedLength": "variable"}]}},
                {"name": "RemoveMe"},
                {"name": "Last"}
            ]
        }])),
        false,
    );
    assert!(seeded.ok, "{:?}", seeded.errors);
    let operations = serde_json::json!([
        {"op": "setProperties", "values": {"Comment": "Observed comment"}},
        {"op": "add", "collection": "attributes", "elements": [{
            "name": "Added",
            "synonym": "Added synonym",
            "comment": "Added comment",
            "type": {"variants": [{"kind": "string", "length": 12, "allowedLength": "fixed"}]},
            "required": true,
            "fillValue": {"kind": "string", "value": "seed"},
            "position": {"before": "Existing"}
        }]},
        {"op": "add", "collection": "tabularSections", "elements": [{
            "name": "Rows",
            "synonym": "Rows synonym",
            "comment": "Rows comment",
            "attributes": [
                {"name": "LineText", "type": {"variants": [{"kind": "string", "length": 20, "allowedLength": "variable"}]}, "required": true},
                {"name": "LineNumber", "type": {"variants": [{"kind": "number", "digits": 10, "fraction": 2, "sign": "nonNegative"}]}, "required": false, "position": {"before": "LineText"}}
            ]
        }]},
        {"op": "update", "collection": "attributes", "elements": [{
            "name": "Existing",
            "newName": "Renamed",
            "synonym": "Renamed synonym",
            "comment": "Renamed comment",
            "type": {"variants": [{"kind": "number", "digits": 6, "fraction": 2, "sign": "any"}]},
            "required": true,
            "fillValue": {"kind": "number", "value": "12.50"},
            "position": {"before": "First"}
        }]},
        {"op": "remove", "collection": "attributes", "names": ["RemoveMe"]},
        {"op": "editRelations", "relation": "owners", "mode": "replace", "targets": [{"metadataPath": "Catalog.Owner"}]}
    ]);

    let preview = call_edit(
        workspace.path(),
        "Catalog.RoundTrip",
        operations.clone(),
        true,
    );
    assert!(preview.ok, "{:?}", preview.errors);
    let effects = preview.data.as_ref().unwrap()["effects"]
        .as_array()
        .expect("semantic preview effects");
    assert_eq!(effects.len(), operations.as_array().unwrap().len());
    assert_eq!(
        effects
            .iter()
            .map(|effect| effect["operation"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec![
            "setProperties",
            "add",
            "add",
            "update",
            "remove",
            "editRelations"
        ]
    );
    assert!(effects[1]["before"].is_null());
    assert!(effects[4]["after"].is_null());
    assert_eq!(effects[0]["before"], serde_json::json!({"Comment": ""}));
    assert_eq!(
        effects[0]["after"],
        serde_json::json!({"Comment": "Observed comment"})
    );
    assert_eq!(effects[1]["after"][0]["name"], "Added");
    assert_eq!(effects[1]["after"][0]["required"], true);
    assert_eq!(
        effects[1]["after"][0]["fillValue"],
        serde_json::json!({"kind": "string", "value": "seed"})
    );
    assert_eq!(effects[3]["before"][0]["name"], "Existing");
    assert_eq!(effects[3]["after"][0]["name"], "Renamed");
    assert_eq!(
        effects[3]["after"][0]["fillValue"],
        serde_json::json!({"kind": "number", "value": "12.50"})
    );
    assert_eq!(effects[4]["before"][0]["name"], "RemoveMe");
    assert_eq!(effects[5]["before"], serde_json::json!([]));
    assert_eq!(
        effects[5]["after"],
        serde_json::json!([{"kind": "object", "value": "Catalog.Owner"}])
    );
    assert!(effects
        .iter()
        .enumerate()
        .all(|(index, effect)| effect["operationIndex"] == Value::Number((index as u64).into())));
    assert!(!serde_json::to_string(effects)
        .unwrap()
        .contains("MetaDataObject"));

    let applied = call_edit(workspace.path(), "Catalog.RoundTrip", operations, false);
    assert!(applied.ok, "{:?}", applied.errors);
    let result = call_info_path(workspace.path(), "Catalog.RoundTrip", []);
    assert!(result.ok, "{:?}", result.errors);
    let data = result.data.unwrap();
    assert!(data["properties"]
        .as_array()
        .unwrap()
        .iter()
        .any(|property| {
            property["key"] == "Comment" && property["value"] == "Observed comment"
        }));
    assert_eq!(
        data["collections"]["attributes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|element| element["name"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["Renamed", "First", "Added", "Last"]
    );
    let renamed = &data["collections"]["attributes"][0];
    assert_eq!(renamed["synonym"], "Renamed synonym");
    assert_eq!(renamed["comment"], "Renamed comment");
    assert_eq!(renamed["required"], true);
    assert_eq!(
        renamed["fillValue"],
        serde_json::json!({"kind": "number", "value": "12.50"})
    );
    assert_eq!(renamed["type"]["variants"][0]["kind"], "number");
    let rows = &data["collections"]["tabularSections"][0];
    assert_eq!(rows["synonym"], "Rows synonym");
    assert_eq!(rows["comment"], "Rows comment");
    assert_eq!(
        rows["attributes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|element| element["name"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["LineNumber", "LineText"]
    );
    assert_eq!(rows["attributes"][0]["required"], false);
    assert_eq!(rows["attributes"][1]["required"], true);
    assert_eq!(
        data["relations"]["owners"],
        serde_json::json!([{"kind": "object", "value": "Catalog.Owner"}])
    );

    let template_preview = add_catalog(workspace.path(), "TemplateOnly", None, true);
    assert!(template_preview.ok, "{:?}", template_preview.errors);
    assert_eq!(
        template_preview.data.unwrap()["effects"],
        serde_json::json!([{
            "operation": "createTemplate",
            "target": "Catalog.TemplateOnly",
            "before": null,
            "after": {"kind": "Catalog", "name": "TemplateOnly"}
        }])
    );
}

#[test]
fn info_marks_malformed_optional_field_incomplete_with_diagnostic() {
    let workspace = create_info_workspace("malformed-optional-type");
    let descriptor = workspace.path().join("src/Catalogs/Inspectable.xml");
    let xml = std::fs::read_to_string(&descriptor).unwrap();
    let malformed = xml.replacen(
        "<v8:Type>xs:string</v8:Type>",
        "<v8:Type>xs:unsupported</v8:Type>",
        1,
    );
    assert_ne!(malformed, xml, "fixture must contain a typed attribute");
    std::fs::write(&descriptor, malformed).unwrap();

    let result = call_info(workspace.path(), []);

    assert!(!result.ok);
    let data = result.data.as_ref().expect("partial typed info");
    assert_eq!(data["collections"]["attributes"][0]["incomplete"], true);
    assert!(result
        .diagnostics
        .as_ref()
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .any(|diagnostic| diagnostic["field"] == "collections.attributes[0].type"));
}

#[test]
fn info_marks_bare_fill_value_incomplete_with_diagnostic() {
    let workspace = create_info_workspace("malformed-optional-fill-value");
    let descriptor = workspace.path().join("src/Catalogs/Inspectable.xml");
    let xml = std::fs::read_to_string(&descriptor).unwrap();
    let malformed = xml.replacen("<FillValue xsi:nil=\"true\"/>", "<FillValue/>", 1);
    assert_ne!(
        malformed, xml,
        "fixture must contain an absent fill value marker"
    );
    std::fs::write(&descriptor, malformed).unwrap();

    let result = call_info(workspace.path(), []);

    assert!(!result.ok);
    let data = result.data.as_ref().expect("partial typed info");
    assert_eq!(data["collections"]["attributes"][0]["incomplete"], true);
    assert!(result
        .diagnostics
        .as_ref()
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .any(|diagnostic| diagnostic["field"] == "collections.attributes[0].fillValue"));
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

const COMMAND_INTERFACE_RULE: &str = "belongs to no subsystem with IncludeInCommandInterface";

fn write_subsystem(workspace: &Path, relative_dir: &str, name: &str, include: &str, content: &str) {
    let dir = workspace.join(relative_dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join(format!("{name}.xml")),
        format!(
            concat!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
                "<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" ",
                "xmlns:v8=\"http://v8.1c.ru/8.1/data/core\" ",
                "xmlns:xr=\"http://v8.1c.ru/8.3/xcf/readable\" ",
                "xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" version=\"2.20\">\n",
                "\t<Subsystem uuid=\"77777777-7777-4777-8777-777777777777\">\n",
                "\t\t<Properties>\n",
                "\t\t\t<Name>{}</Name>\n",
                "\t\t\t<IncludeInCommandInterface>{}</IncludeInCommandInterface>\n",
                "\t\t\t<Content>\n",
                "{}",
                "\t\t\t</Content>\n",
                "\t\t</Properties>\n",
                "\t</Subsystem>\n",
                "</MetaDataObject>\n"
            ),
            name, include, content
        ),
    )
    .unwrap();
}

fn content_item(reference: &str) -> String {
    format!("\t\t\t\t<xr:Item xsi:type=\"xr:MDObjectRef\">{reference}</xr:Item>\n")
}

fn register_command_interface_warnings(workspace: &Path, name: &str) -> Vec<String> {
    {
        let _cwd = ProcessCwdGuard::enter(workspace).unwrap();
        let added = UnicaApplication::new()
            .call_tool(
                "unica.meta.add",
                &Map::from_iter([
                    ("sourceSet".to_string(), Value::String("main".to_string())),
                    (
                        "kind".to_string(),
                        Value::String("InformationRegister".to_string()),
                    ),
                    ("name".to_string(), Value::String(name.to_string())),
                    ("dryRun".to_string(), Value::Bool(false)),
                ]),
            )
            .unwrap();
        assert!(added.ok, "{name}: {:?}", added.errors);
    }
    let result = call_info_path(workspace, &format!("InformationRegister.{name}"), []);
    assert!(result.ok, "{name}: {:?}", result.errors);
    let data = result.data.expect("typed meta.info data");
    data["validation"]["diagnostics"]
        .as_array()
        .expect("validation diagnostics")
        .iter()
        .filter(|diagnostic| diagnostic["severity"] == "warning")
        .map(|diagnostic| diagnostic["message"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn info_warns_when_a_command_interface_register_is_in_no_subsystem() {
    let workspace = create_info_workspace("register-without-subsystem");
    let warnings = register_command_interface_warnings(workspace.path(), "Orphan");

    assert!(
        warnings
            .iter()
            .any(|warning| warning.contains(COMMAND_INTERFACE_RULE)),
        "{warnings:?}"
    );
}

#[test]
fn info_accepts_a_register_listed_in_a_command_interface_subsystem() {
    let workspace = create_info_workspace("register-in-subsystem");
    write_subsystem(
        workspace.path(),
        "src/Subsystems",
        "Sales",
        "true",
        &content_item("InformationRegister.Listed"),
    );
    let warnings = register_command_interface_warnings(workspace.path(), "Listed");

    assert!(
        !warnings
            .iter()
            .any(|warning| warning.contains(COMMAND_INTERFACE_RULE)),
        "{warnings:?}"
    );
}

#[test]
fn info_warns_when_the_only_subsystem_is_excluded_from_the_command_interface() {
    let workspace = create_info_workspace("register-in-excluded-subsystem");
    write_subsystem(
        workspace.path(),
        "src/Subsystems",
        "Service",
        "false",
        &content_item("InformationRegister.Excluded"),
    );
    let warnings = register_command_interface_warnings(workspace.path(), "Excluded");

    assert!(
        warnings
            .iter()
            .any(|warning| warning.contains(COMMAND_INTERFACE_RULE)),
        "{warnings:?}"
    );
}

#[test]
fn info_accepts_a_register_listed_in_a_nested_subsystem() {
    let workspace = create_info_workspace("register-in-nested-subsystem");
    write_subsystem(workspace.path(), "src/Subsystems", "Parent", "true", "");
    write_subsystem(
        workspace.path(),
        "src/Subsystems/Parent/Subsystems",
        "Child",
        "true",
        &content_item("InformationRegister.Nested"),
    );
    let warnings = register_command_interface_warnings(workspace.path(), "Nested");

    assert!(
        !warnings
            .iter()
            .any(|warning| warning.contains(COMMAND_INTERFACE_RULE)),
        "{warnings:?}"
    );
}
