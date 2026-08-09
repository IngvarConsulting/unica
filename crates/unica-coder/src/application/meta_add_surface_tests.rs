use super::{OperationResult, UnicaApplication};
use crate::composition::testing::{
    with_meta_add_after_authorization_hook, with_meta_edit_before_reauthorization_hook,
};
use crate::domain::cancellation::CancellationToken;
use crate::test_support::{tree_snapshot, ProcessCwdGuard};
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};
use uuid::Uuid;

const METADATA_KINDS: &[&str] = &[
    "Catalog",
    "Document",
    "Enum",
    "Constant",
    "InformationRegister",
    "AccumulationRegister",
    "AccountingRegister",
    "CalculationRegister",
    "ChartOfAccounts",
    "ChartOfCharacteristicTypes",
    "ChartOfCalculationTypes",
    "BusinessProcess",
    "Task",
    "ExchangePlan",
    "DocumentJournal",
    "Report",
    "DataProcessor",
    "CommonModule",
    "ScheduledJob",
    "EventSubscription",
    "HTTPService",
    "WebService",
    "DefinedType",
];

struct TempWorkspace(PathBuf);

impl TempWorkspace {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "unica-platform-meta-{label}-{}-{}",
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

fn create_configuration_workspace(label: &str) -> TempWorkspace {
    let workspace = TempWorkspace::new(label);
    let args = Map::from_iter([
        (
            "cwd".to_string(),
            Value::String(workspace.path().display().to_string()),
        ),
        ("Name".to_string(), Value::String("MetaSurface".to_string())),
        ("OutputDir".to_string(), Value::String("src".to_string())),
        ("dryRun".to_string(), Value::Bool(false)),
    ]);
    let result = UnicaApplication::new()
        .call_tool("unica.cf.init", &args)
        .expect("configuration fixture call");
    assert!(result.ok, "{:?}", result.errors);
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
    for (kind, name) in [
        ("Catalog", "MetaAddSource"),
        ("Document", "MetaAddRegistrar"),
        ("ChartOfAccounts", "MetaAddAccounts"),
        ("ChartOfCalculationTypes", "MetaAddCalculationTypes"),
        ("Task", "MetaAddTask"),
        ("CommonModule", "MetaAddHandlers"),
    ] {
        let added = call_add(workspace.path(), kind, name, false);
        assert!(added.ok, "{kind}.{name}: {:?}", added.errors);
    }
    std::fs::write(
        workspace
            .path()
            .join("src/CommonModules/MetaAddHandlers/Ext/Module.bsl"),
        concat!(
            "Procedure Run() Export\n",
            "EndProcedure\n\n",
            "Procedure Handle(Source, Cancel) Export\n",
            "EndProcedure\n",
        ),
    )
    .unwrap();
    workspace
}

/// Минимальные `operations`, делающие объект целостным по ADR-0030.
///
/// Виды без записи в таблице условий не требуют ничего, и инструмент за них
/// ничего не придумывает, поэтому здесь для них пусто.
fn coherence_operations(kind: &str) -> Option<Value> {
    match kind {
        "InformationRegister" | "AccumulationRegister" | "AccountingRegister" => Some(json!([{
            "op": "add",
            "collection": "resources",
            "elements": [{
                "name": "Value",
                "type": {"variants": [{
                    "kind": "number",
                    "digits": 15,
                    "fraction": 2,
                    "sign": "any"
                }]}
            }]
        }])),
        "WebService" => Some(json!([{
            "op": "setProperties",
            "values": {"Namespace": "urn:unica:test"}
        }])),
        _ => None,
    }
}

fn add_args(workspace: &Path, kind: &str, name: &str, dry_run: bool) -> Map<String, Value> {
    let _ = workspace;
    let mut args = Map::from_iter([
        ("sourceSet".to_string(), Value::String("main".to_string())),
        ("kind".to_string(), Value::String(kind.to_string())),
        ("name".to_string(), Value::String(name.to_string())),
        ("dryRun".to_string(), Value::Bool(dry_run)),
    ]);
    if let Some(operations) = coherence_operations(kind) {
        args.insert("operations".to_string(), operations);
    }
    args
}

fn call_add(workspace: &Path, kind: &str, name: &str, dry_run: bool) -> OperationResult {
    let _cwd = ProcessCwdGuard::enter(workspace).unwrap();
    UnicaApplication::new()
        .call_tool("unica.meta.add", &add_args(workspace, kind, name, dry_run))
        .expect("internal meta.add call")
}

fn configured_catalog_add_args(workspace: &Path, name: &str, dry_run: bool) -> Map<String, Value> {
    let mut args = add_args(workspace, "Catalog", name, dry_run);
    args.insert(
        "operations".to_string(),
        json!([
            {
                "op": "setProperties",
                "values": {"Comment": "Configured during creation"}
            },
            {
                "op": "add",
                "collection": "attributes",
                "elements": [{
                    "name": "ExternalCode",
                    "comment": "Created in the same transaction",
                    "type": {
                        "variants": [{
                            "kind": "string",
                            "length": 24,
                            "allowedLength": "variable"
                        }]
                    }
                }]
            }
        ]),
    );
    args
}

fn call_add_with_args(workspace: &Path, args: &Map<String, Value>) -> OperationResult {
    call_add_with_args_result(workspace, args).expect("internal meta.add call")
}

fn call_add_with_args_result(
    workspace: &Path,
    args: &Map<String, Value>,
) -> Result<OperationResult, String> {
    let _cwd = ProcessCwdGuard::enter(workspace)?;
    UnicaApplication::new().call_tool("unica.meta.add", args)
}

fn call_edit(
    workspace: &Path,
    metadata_path: &str,
    operations: Value,
    dry_run: bool,
) -> OperationResult {
    let _cwd = ProcessCwdGuard::enter(workspace).unwrap();
    UnicaApplication::new()
        .call_tool(
            "unica.meta.edit",
            &Map::from_iter([
                ("sourceSet".to_string(), json!("main")),
                ("metadataPath".to_string(), json!(metadata_path)),
                ("operations".to_string(), operations),
                ("dryRun".to_string(), json!(dry_run)),
            ]),
        )
        .expect("internal meta.edit call")
}

#[test]
fn add_refuses_an_incoherent_object_and_names_what_the_platform_requires() {
    // 8.3.27 принимает такой дескриптор как документ и отвергает как объект
    // конфигурации, поэтому отказ выдаётся на входе (ADR-0030).
    let workspace = create_configuration_workspace("incoherent-register");
    let mut args = add_args(workspace.path(), "InformationRegister", "Prices", false);
    args.remove("operations");

    let applied = call_add_with_args(workspace.path(), &args);

    assert!(!applied.ok, "{applied:?}");
    let message = applied.errors.join(" ");
    assert!(
        message.contains("Register without dimensions, resources, and attributes"),
        "{message}"
    );
    assert!(
        message.contains("dimensions, resources, attributes"),
        "{message}"
    );
    assert!(!workspace
        .path()
        .join("src/InformationRegisters/Prices.xml")
        .exists());

    args.insert("dryRun".to_string(), json!(true));
    let preview = call_add_with_args(workspace.path(), &args);

    assert!(
        !preview.ok,
        "dryRun must report the same refusal: {preview:?}"
    );
    assert!(
        !workspace
            .path()
            .join("src/InformationRegisters/Prices.xml")
            .exists(),
        "dryRun must not create metadata files"
    );
}

#[test]
fn edit_judges_the_final_state_of_the_call_not_each_operation() {
    // Замена единственного ресурса — remove вместе с add в одном вызове.
    // Промежуточная пустота нарушением не считается.
    let workspace = create_configuration_workspace("replace-only-resource");
    let created = call_add(workspace.path(), "InformationRegister", "Prices", false);
    assert!(created.ok, "{created:?}");

    let replaced = call_edit(
        workspace.path(),
        "InformationRegister.Prices",
        json!([
            {"op": "remove", "collection": "resources", "names": ["Value"]},
            {"op": "add", "collection": "resources", "elements": [{
                "name": "Price",
                "type": {"variants": [{"kind": "number", "digits": 15, "fraction": 2, "sign": "any"}]}
            }]}
        ]),
        false,
    );

    assert!(replaced.ok, "{replaced:?}");
    let descriptor = workspace.path().join("src/InformationRegisters/Prices.xml");
    let before_refusal = std::fs::read(&descriptor).unwrap();

    let emptied = call_edit(
        workspace.path(),
        "InformationRegister.Prices",
        json!([{"op": "remove", "collection": "resources", "names": ["Price"]}]),
        false,
    );

    assert!(
        !emptied.ok,
        "emptying the register must be refused: {emptied:?}"
    );
    assert_eq!(
        std::fs::read(&descriptor).unwrap(),
        before_refusal,
        "a refused edit must leave the descriptor byte-identical"
    );
}

#[test]
fn add_applies_operations_atomically() {
    let workspace = create_configuration_workspace("configured-create");
    let source = workspace.path().join("src");
    let descriptor = source.join("Catalogs/Configured.xml");
    let before = tree_snapshot(&source);

    let preview_args = configured_catalog_add_args(workspace.path(), "Configured", true);
    let preview = call_add_with_args(workspace.path(), &preview_args);

    assert!(preview.ok, "{:?}", preview.errors);
    assert_eq!(
        preview.data.as_ref().unwrap()["metadataPath"],
        "Catalog.Configured"
    );
    assert_eq!(
        preview.data.as_ref().unwrap()["effects"]
            .as_array()
            .unwrap()
            .iter()
            .map(|effect| effect["operation"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["createTemplate", "setProperties", "add"]
    );
    let effects = preview.data.as_ref().unwrap()["effects"]
        .as_array()
        .unwrap();
    assert!(effects[0].get("operationIndex").is_none());
    assert_eq!(
        effects[1..]
            .iter()
            .map(|effect| effect["operationIndex"].as_u64().unwrap())
            .collect::<Vec<_>>(),
        vec![0, 1]
    );
    assert_eq!(effects[0]["target"], "Catalog.Configured");
    assert!(effects[0]["before"].is_null());
    assert_eq!(
        effects[0]["after"],
        serde_json::json!({"kind": "Catalog", "name": "Configured"})
    );
    assert_eq!(
        tree_snapshot(&source),
        before,
        "preview changed source bytes"
    );
    assert!(!descriptor.exists());

    let apply_args = configured_catalog_add_args(workspace.path(), "Configured", false);
    let applied = call_add_with_args(workspace.path(), &apply_args);

    assert!(applied.ok, "{:?}", applied.errors);
    assert_eq!(
        applied.data.as_ref().unwrap()["effects"]
            .as_array()
            .unwrap()
            .iter()
            .map(|effect| effect["operation"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["createTemplate", "setProperties", "add"]
    );
    let xml = std::fs::read_to_string(&descriptor).expect("configured descriptor");
    assert!(xml.contains("<Comment>Configured during creation</Comment>"));
    assert!(xml.contains("<Name>ExternalCode</Name>"));
    assert!(xml.contains("<v8:StringQualifiers>"));
    assert!(xml.contains("<v8:Length>24</v8:Length>"));
    let owner = std::fs::read_to_string(source.join("Configuration.xml")).unwrap();
    assert!(owner.contains("<Catalog>Configured</Catalog>"));
}

#[test]
fn failed_add_operation_leaves_no_object() {
    let workspace = create_configuration_workspace("failed-configured-create");
    let source = workspace.path().join("src");
    let before = tree_snapshot(&source);
    let mut args = configured_catalog_add_args(workspace.path(), "Rejected", false);
    args.insert(
        "operations".to_string(),
        json!([
            {
                "op": "setProperties",
                "values": {"Comment": "Must remain private"}
            },
            {
                "op": "remove",
                "collection": "attributes",
                "names": ["MissingAttribute"]
            }
        ]),
    );

    let result = call_add_with_args(workspace.path(), &args);

    assert!(
        !result.ok,
        "invalid operation sequence unexpectedly succeeded"
    );
    let diagnostic = &result.diagnostics.as_ref().unwrap()[0];
    assert_eq!(diagnostic["operationIndex"], 1);
    assert_eq!(tree_snapshot(&source), before);
    assert!(!source.join("Catalogs/Rejected.xml").exists());
    assert!(!std::fs::read_to_string(source.join("Configuration.xml"))
        .unwrap()
        .contains("<Catalog>Rejected</Catalog>"));
}

#[test]
fn add_rejects_explicit_empty_operations_without_writes() {
    let workspace = create_configuration_workspace("empty-create-operations");
    let source = workspace.path().join("src");
    let before = tree_snapshot(&source);
    let mut args = add_args(workspace.path(), "Catalog", "RejectedEmpty", false);
    args.insert("operations".to_string(), json!([]));

    let error = call_add_with_args_result(workspace.path(), &args).unwrap_err();

    assert!(error.contains("operations must not be empty"), "{error}");
    assert_eq!(tree_snapshot(&source), before);
}

#[test]
fn add_rejects_kind_incompatible_property_without_writes() {
    let workspace = create_configuration_workspace("incompatible-create-property");
    let source = workspace.path().join("src");
    let before = tree_snapshot(&source);
    let mut args = add_args(workspace.path(), "Catalog", "RejectedProperty", false);
    args.insert(
        "operations".to_string(),
        json!([{
            "op": "setProperties",
            "values": {"NumberLength": 12}
        }]),
    );

    let error = call_add_with_args_result(workspace.path(), &args).unwrap_err();

    assert!(
        error.contains("property `NumberLength` is not supported for Catalog"),
        "{error}"
    );
    assert_eq!(tree_snapshot(&source), before);
}

#[test]
fn meta_add_preview_all_23_kinds_returns_logical_valid_plan_without_writes() {
    let workspace = create_configuration_workspace("preview-all-kinds");
    let owner = workspace.path().join("src/Configuration.xml");
    let owner_before = std::fs::read(&owner).unwrap();
    let mut failures = Vec::new();

    for kind in METADATA_KINDS {
        let name = format!("Preview{kind}");
        let result = call_add(workspace.path(), kind, &name, true);
        if !result.ok {
            failures.push(format!("{kind}: {:?}", result.errors));
            continue;
        }
        let data = result.data.expect("typed mutation data");
        let public_data = serde_json::to_string(&data).unwrap();
        assert!(!public_data.contains(&workspace.path().display().to_string()));
        assert!(!public_data.contains("PlatformXml"));
        assert!(!public_data.contains("source_root"));
        assert_eq!(
            data["metadataPath"],
            Value::String(format!("{kind}.{name}")),
            "{kind}"
        );
        assert_eq!(data["changed"], true, "{kind}");
        assert_eq!(data["validation"]["status"], "passed", "{kind}");
        let plan = data["publicationPlan"]
            .as_array()
            .expect("publication plan");
        assert!(
            plan.iter().any(|entry| entry["resource"] == "descriptor"),
            "{kind}: {plan:?}"
        );
        assert!(
            plan.iter().any(|entry| entry["resource"] == "registration"),
            "{kind}: {plan:?}"
        );
        assert_eq!(std::fs::read(&owner).unwrap(), owner_before, "{kind}");
        let descriptor = workspace
            .path()
            .join("src")
            .join(kind_directory(kind))
            .join(format!("{name}.xml"));
        assert!(!descriptor.exists(), "{kind}: {}", descriptor.display());
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn meta_add_preview_rejects_read_capable_source_without_create_capability() {
    let workspace = TempWorkspace::new("read-only-capability");
    std::fs::create_dir_all(workspace.path().join("erf")).unwrap();
    std::fs::write(
        workspace.path().join("erf/ReadOnly.xml"),
        concat!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
            "<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.20\">",
            "<ExternalReport uuid=\"00000000-0000-0000-0000-000000000001\">",
            "<Properties><Name>ReadOnly</Name></Properties>",
            "</ExternalReport></MetaDataObject>\n",
        ),
    )
    .unwrap();
    std::fs::write(
        workspace.path().join("v8project.yaml"),
        concat!(
            "format: DESIGNER\n",
            "source-set:\n",
            "  - name: main\n",
            "    type: EXTERNAL_REPORTS\n",
            "    path: erf\n",
        ),
    )
    .unwrap();

    let result = call_add(workspace.path(), "Catalog", "Denied", true);
    assert!(!result.ok);
    assert_eq!(
        result.diagnostics.unwrap()[0]["code"],
        "capability_unavailable"
    );
    assert!(!workspace.path().join("erf/Catalogs/Denied.xml").exists());
}

#[test]
fn meta_add_preview_rejects_unsupported_source_format() {
    let workspace = TempWorkspace::new("unsupported-format");
    std::fs::create_dir_all(workspace.path().join("edt")).unwrap();
    std::fs::write(
        workspace.path().join("edt/.project"),
        "<projectDescription/>",
    )
    .unwrap();
    std::fs::write(
        workspace.path().join("v8project.yaml"),
        concat!(
            "format: EDT\n",
            "source-set:\n",
            "  - name: main\n",
            "    type: CONFIGURATION\n",
            "    path: edt\n",
        ),
    )
    .unwrap();

    let result = call_add(workspace.path(), "Catalog", "Denied", true);
    assert!(!result.ok);
    assert_eq!(
        result.diagnostics.unwrap()[0]["code"],
        "capability_unavailable"
    );
}

#[test]
fn meta_add_preview_rejects_platform_xml_outside_supported_format_profile() {
    let workspace = create_configuration_workspace("unsupported-platform-format");
    let owner = workspace.path().join("src/Configuration.xml");
    let current = std::fs::read_to_string(&owner).unwrap();
    let unsupported = current.replacen("version=\"2.20\"", "version=\"2.19\"", 1);
    assert_ne!(unsupported, current);
    std::fs::write(&owner, unsupported).unwrap();
    let before = tree_snapshot(&workspace.path().join("src"));

    let result = call_add(workspace.path(), "Catalog", "Denied", true);

    assert!(!result.ok);
    assert_eq!(
        result.diagnostics.unwrap()[0]["code"],
        "capability_unavailable"
    );
    assert_eq!(tree_snapshot(&workspace.path().join("src")), before);
}

#[test]
fn meta_add_preview_rejects_dangling_common_module_method_dependency() {
    let workspace = create_configuration_workspace("missing-handler-method");
    std::fs::write(
        workspace
            .path()
            .join("src/CommonModules/MetaAddHandlers/Ext/Module.bsl"),
        b"",
    )
    .unwrap();
    let before = tree_snapshot(&workspace.path().join("src"));

    let result = call_add(workspace.path(), "ScheduledJob", "Denied", true);

    assert!(!result.ok);
    assert_eq!(
        result.diagnostics.unwrap()[0]["code"],
        "capability_unavailable"
    );
    assert_eq!(tree_snapshot(&workspace.path().join("src")), before);
}

fn kind_directory(kind: &str) -> &'static str {
    match kind {
        "Catalog" => "Catalogs",
        "Document" => "Documents",
        "Enum" => "Enums",
        "Constant" => "Constants",
        "InformationRegister" => "InformationRegisters",
        "AccumulationRegister" => "AccumulationRegisters",
        "AccountingRegister" => "AccountingRegisters",
        "CalculationRegister" => "CalculationRegisters",
        "ChartOfAccounts" => "ChartsOfAccounts",
        "ChartOfCharacteristicTypes" => "ChartsOfCharacteristicTypes",
        "ChartOfCalculationTypes" => "ChartsOfCalculationTypes",
        "BusinessProcess" => "BusinessProcesses",
        "Task" => "Tasks",
        "ExchangePlan" => "ExchangePlans",
        "DocumentJournal" => "DocumentJournals",
        "Report" => "Reports",
        "DataProcessor" => "DataProcessors",
        "CommonModule" => "CommonModules",
        "ScheduledJob" => "ScheduledJobs",
        "EventSubscription" => "EventSubscriptions",
        "HTTPService" => "HTTPServices",
        "WebService" => "WebServices",
        "DefinedType" => "DefinedTypes",
        other => panic!("missing test directory for {other}"),
    }
}

#[test]
fn meta_add_apply_all_23_kinds_is_atomic_and_duplicate_is_byte_stable() {
    let workspace = create_configuration_workspace("apply-all-kinds");
    for kind in METADATA_KINDS {
        let name = format!("Applied{kind}");
        let result = call_add(workspace.path(), kind, &name, false);
        assert!(result.ok, "{kind}: {:?}", result.errors);
        assert_eq!(
            result.data.as_ref().unwrap()["metadataPath"],
            Value::String(format!("{kind}.{name}")),
            "{kind}"
        );
        let descriptor = workspace
            .path()
            .join("src")
            .join(kind_directory(kind))
            .join(format!("{name}.xml"));
        let bytes = std::fs::read(&descriptor).expect("created descriptor");
        let xml = std::str::from_utf8(&bytes)
            .unwrap()
            .trim_start_matches('\u{feff}');
        let document = roxmltree::Document::parse(xml).expect("created descriptor XML");
        assert_eq!(
            document
                .root_element()
                .children()
                .find(|node| node.is_element())
                .unwrap()
                .tag_name()
                .name(),
            *kind
        );
        if *kind == "EventSubscription" {
            assert!(
                xml.contains("<v8:Type>cfg:CatalogObject.AppliedCatalog</v8:Type>"),
                "event source must use the object wire type: {xml}"
            );
            assert!(!xml.contains("cfg:CatalogRef.AppliedCatalog"));
        }
        if *kind == "CommonModule" {
            assert!(
                xml.contains("<Server>true</Server>"),
                "minimal typed common module must be executable: {xml}"
            );
        }
        let owner =
            std::fs::read_to_string(workspace.path().join("src/Configuration.xml")).unwrap();
        assert!(
            owner.contains(&format!("<{kind}>{name}</{kind}>")),
            "{kind}"
        );

        let before_duplicate = tree_snapshot(&workspace.path().join("src"));
        let duplicate = call_add(workspace.path(), kind, &name, false);
        assert!(!duplicate.ok, "{kind}: duplicate unexpectedly succeeded");
        assert_eq!(
            duplicate.diagnostics.as_ref().unwrap()[0]["code"],
            "already_exists",
            "{kind}: {:?}",
            duplicate.diagnostics
        );
        assert_eq!(
            tree_snapshot(&workspace.path().join("src")),
            before_duplicate,
            "{kind}: duplicate changed source bytes"
        );
    }
}

#[test]
fn meta_add_apply_rejects_partial_descriptor_module_and_registration_without_writes() {
    let workspace = create_configuration_workspace("partial-footprints");
    let source = workspace.path().join("src");

    std::fs::create_dir_all(source.join("Catalogs")).unwrap();
    std::fs::write(source.join("Catalogs/PartialDescriptor.xml"), b"partial").unwrap();
    assert_partial_is_stable(&workspace, "Catalog", "PartialDescriptor");

    std::fs::create_dir_all(source.join("Documents/PartialModule/Ext")).unwrap();
    std::fs::write(
        source.join("Documents/PartialModule/Ext/ObjectModule.bsl"),
        b"partial",
    )
    .unwrap();
    assert_partial_is_stable(&workspace, "Document", "PartialModule");

    let applied = call_add(workspace.path(), "Catalog", "PartialRegistration", false);
    assert!(applied.ok, "{:?}", applied.errors);
    std::fs::remove_file(source.join("Catalogs/PartialRegistration.xml")).unwrap();
    std::fs::remove_dir_all(source.join("Catalogs/PartialRegistration")).unwrap();
    assert_partial_is_stable(&workspace, "Catalog", "PartialRegistration");

    std::fs::create_dir_all(source.join("Catalogs/UnexpectedResource/Ext")).unwrap();
    std::fs::write(
        source.join("Catalogs/UnexpectedResource/Ext/Help.xml"),
        b"unexpected",
    )
    .unwrap();
    assert_partial_is_stable(&workspace, "Catalog", "UnexpectedResource");
}

#[test]
fn meta_add_apply_honors_prepublication_cancellation_without_writes() {
    let workspace = create_configuration_workspace("cancelled");
    let before = tree_snapshot(&workspace.path().join("src"));
    let _cwd = ProcessCwdGuard::enter(workspace.path()).unwrap();
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let result = UnicaApplication::new()
        .call_tool_cancellable(
            "unica.meta.add",
            &add_args(workspace.path(), "Catalog", "Cancelled", false),
            cancellation,
        )
        .unwrap();
    assert!(!result.ok);
    assert_eq!(tree_snapshot(&workspace.path().join("src")), before);
}

#[test]
fn meta_add_apply_rejects_support_locked_configuration_without_writes() {
    let workspace = create_configuration_workspace("support-locked");
    let support = workspace.path().join("src/Ext/ParentConfigurations.bin");
    std::fs::write(
        &support,
        concat!(
            "\u{feff}{6,1,1,dddddddd-dddd-dddd-dddd-dddddddddddd,0,",
            "eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee,\"1.0\",\"Vendor\",",
            "\"VendorConf\",0,0,0}"
        ),
    )
    .unwrap();
    let before = tree_snapshot(&workspace.path().join("src"));

    let result = call_add(workspace.path(), "Catalog", "Locked", false);

    assert!(!result.ok);
    assert_eq!(result.diagnostics.unwrap()[0]["code"], "support_locked");
    assert_eq!(tree_snapshot(&workspace.path().join("src")), before);
}

#[test]
fn meta_add_reauthorizes_bound_support_state_before_transaction_mutations() {
    let workspace = create_configuration_workspace("support-authorization-drift");
    let source = workspace.path().join("src");
    let owner = source.join("Configuration.xml");
    let owner_before = std::fs::read(&owner).unwrap();
    let support = source.join("Ext/ParentConfigurations.bin");
    let support_bytes = concat!(
        "\u{feff}{6,1,1,dddddddd-dddd-dddd-dddd-dddddddddddd,0,",
        "eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee,\"1.0\",\"Vendor\",",
        "\"VendorConf\",0,0,0}"
    )
    .as_bytes()
    .to_vec();
    let support_for_hook = support.clone();
    let support_bytes_for_hook = support_bytes.clone();
    let mut expected = tree_snapshot(&source);
    expected.insert(
        PathBuf::from("Ext/ParentConfigurations.bin"),
        support_bytes.clone(),
    );

    let result = with_meta_add_after_authorization_hook(
        move || std::fs::write(support_for_hook, support_bytes_for_hook).unwrap(),
        || call_add(workspace.path(), "Catalog", "AuthorizationDrift", false),
    );

    assert!(
        !result.ok,
        "stale authorization unexpectedly created metadata"
    );
    assert_eq!(
        result.diagnostics.as_ref().unwrap()[0]["code"],
        "support_locked"
    );
    assert!(result.cache.events.is_empty());
    assert_eq!(std::fs::read(&owner).unwrap(), owner_before);
    assert!(!source.join("Catalogs/AuthorizationDrift.xml").exists());
    assert!(!source.join("Catalogs/AuthorizationDrift").exists());
    assert!(!String::from_utf8(owner_before)
        .unwrap()
        .contains("<Catalog>AuthorizationDrift</Catalog>"));

    assert_eq!(tree_snapshot(&source), expected);
}

#[test]
fn meta_edit_reauthorizes_support_state_after_private_post_image_planning() {
    let workspace = create_configuration_workspace("edit-support-authorization-drift");
    let source = workspace.path().join("src");
    let support = source.join("Ext/ParentConfigurations.bin");
    let support_bytes = concat!(
        "\u{feff}{6,1,1,dddddddd-dddd-dddd-dddd-dddddddddddd,0,",
        "eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee,\"1.0\",\"Vendor\",",
        "\"VendorConf\",0,0,0}"
    )
    .as_bytes()
    .to_vec();
    let support_for_hook = support.clone();
    let support_bytes_for_hook = support_bytes.clone();
    let mut expected = tree_snapshot(&source);
    expected.insert(
        PathBuf::from("Ext/ParentConfigurations.bin"),
        support_bytes.clone(),
    );

    let result = with_meta_edit_before_reauthorization_hook(
        move || std::fs::write(support_for_hook, support_bytes_for_hook).unwrap(),
        || {
            call_edit(
                workspace.path(),
                "Catalog.MetaAddSource",
                json!([{"op": "setProperties", "values": {"Comment": "denied"}}]),
                false,
            )
        },
    );

    assert!(
        !result.ok,
        "stale edit authorization unexpectedly published"
    );
    assert_eq!(
        result.diagnostics.as_ref().unwrap()[0]["code"],
        "support_locked"
    );
    assert!(result.cache.events.is_empty());
    assert_eq!(tree_snapshot(&source), expected);
}

#[test]
fn meta_edit_warning_is_derived_from_late_support_authorization() {
    let workspace = create_configuration_workspace("edit-support-warning-drift");
    let source = workspace.path().join("src");
    let support = source.join("Ext/ParentConfigurations.bin");
    let project = workspace.path().join(".v8-project.json");
    let support_for_hook = support.clone();
    let project_for_hook = project.clone();

    let result = with_meta_edit_before_reauthorization_hook(
        move || {
            std::fs::write(
                support_for_hook,
                concat!(
                    "\u{feff}{6,1,1,dddddddd-dddd-dddd-dddd-dddddddddddd,0,",
                    "eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee,\"1.0\",\"Vendor\",",
                    "\"VendorConf\",0,0,0}"
                ),
            )
            .unwrap();
            std::fs::write(project_for_hook, r#"{"editingAllowedCheck":"warn"}"#).unwrap();
        },
        || {
            call_edit(
                workspace.path(),
                "Catalog.MetaAddSource",
                json!([{"op": "setProperties", "values": {"Comment": "warned"}}]),
                false,
            )
        },
    );

    assert!(
        result.ok,
        "late warn policy must permit editing: {:?}",
        result.errors
    );
    assert_eq!(
        result.data.as_ref().unwrap()["diagnostics"],
        json!([{
            "code": "support_locked",
            "severity": "warning",
            "message": "metadata source support policy permits editing with a warning",
            "metadataPath": "Catalog.MetaAddSource"
        }])
    );
}

fn add_with_synonym(workspace: &TempWorkspace, name: &str, synonym: &str) -> Vec<String> {
    let mut args = add_args(workspace.path(), "Catalog", name, false);
    args.insert(
        "operations".to_string(),
        json!([{"op": "setProperties", "values": {"Synonym": synonym}}]),
    );
    let result = call_add_with_args(workspace.path(), &args);
    assert!(result.ok, "{name}: {:?}", result.errors);
    let data = result.data.expect("typed mutation data");
    assert_eq!(data["validation"]["status"], "passed", "{name}");
    data["validation"]["diagnostics"]
        .as_array()
        .expect("validation diagnostics")
        .iter()
        .filter(|diagnostic| diagnostic["severity"] == "warning")
        .map(|diagnostic| diagnostic["message"].as_str().unwrap().to_string())
        .collect()
}

/// Classifies the command-text warning a synonym of `length` characters draws,
/// so the two thresholds are pinned at their exact boundaries.
fn command_text_warning_at(workspace: &TempWorkspace, name: &str, length: usize) -> Option<String> {
    let warnings = add_with_synonym(workspace, name, &"Д".repeat(length));
    let recommended = warnings
        .iter()
        .any(|warning| warning.contains("recommended 30 characters"));
    let ceiling = warnings
        .iter()
        .any(|warning| warning.contains("longer than 38 characters"));
    assert!(
        !(recommended && ceiling),
        "one value must never draw both warnings: {warnings:?}"
    );
    match (recommended, ceiling) {
        (true, _) => Some("recommended".to_string()),
        (_, true) => Some("ceiling".to_string()),
        _ => None,
    }
}

#[test]
fn add_pins_both_command_text_thresholds_at_their_boundaries() {
    let workspace = create_configuration_workspace("command-text-boundaries");

    assert_eq!(command_text_warning_at(&workspace, "Exactly30", 30), None);
    assert_eq!(
        command_text_warning_at(&workspace, "Exactly31", 31).as_deref(),
        Some("recommended")
    );
    assert_eq!(
        command_text_warning_at(&workspace, "Exactly38", 38).as_deref(),
        Some("recommended")
    );
    assert_eq!(
        command_text_warning_at(&workspace, "Exactly39", 39).as_deref(),
        Some("ceiling")
    );
}

#[test]
fn add_warns_when_command_text_passes_the_recommended_limit() {
    let workspace = create_configuration_workspace("command-text-soft-limit");
    // 33 characters: over the recommended 30 but within the hard 38 ceiling.
    let warnings = add_with_synonym(&workspace, "SoftLimit", "Договоры контрагентов по продажам");

    assert!(
        warnings.iter().any(|warning| warning
            == "3. Properties: Synonym 'Договоры контрагентов по продажам' is longer than \
                the recommended 30 characters (33) for the command interface, language 'ru'"),
        "{warnings:?}"
    );
    assert!(
        !warnings
            .iter()
            .any(|warning| warning.contains("longer than 38 characters")),
        "{warnings:?}"
    );
}

#[test]
fn add_allows_command_text_within_the_recommended_limit() {
    let workspace = create_configuration_workspace("command-text-within-limit");
    let warnings = add_with_synonym(&workspace, "ShortLimit", "Договоры");

    assert!(
        !warnings
            .iter()
            .any(|warning| warning.contains("for the command interface")),
        "{warnings:?}"
    );
}

#[test]
fn add_still_warns_above_the_hard_command_text_ceiling() {
    let workspace = create_configuration_workspace("command-text-hard-limit");
    // 52 characters: the ceiling message replaces the recommended-limit one so
    // a single value never reports both.
    let warnings = add_with_synonym(
        &workspace,
        "HardLimit",
        "Очень длинное наименование для командного интерфейса",
    );

    assert!(
        warnings.iter().any(|warning| warning
            == "3. Properties: Synonym 'Очень длинное наименование для командного интерфейса' \
                is longer than 38 characters (52) for the command interface, language 'ru'"),
        "{warnings:?}"
    );
    assert!(
        !warnings
            .iter()
            .any(|warning| warning.contains("recommended 30 characters")),
        "{warnings:?}"
    );
}

fn assert_partial_is_stable(workspace: &TempWorkspace, kind: &str, name: &str) {
    let before = tree_snapshot(&workspace.path().join("src"));
    let result = call_add(workspace.path(), kind, name, false);
    assert!(!result.ok, "{kind}.{name} unexpectedly succeeded");
    assert_eq!(result.diagnostics.unwrap()[0]["code"], "already_exists");
    assert_eq!(tree_snapshot(&workspace.path().join("src")), before);
}
