use super::{compile_legacy_metadata_fixture, OperationResult, UnicaApplication};
use crate::domain::cancellation::CancellationToken;
use serde_json::{Map, Value};
use std::collections::BTreeMap;
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
    let prerequisites = workspace.path().join("meta-add-prerequisites.json");
    std::fs::write(
        &prerequisites,
        serde_json::to_vec(&serde_json::json!([
            {"type": "Catalog", "name": "MetaAddSource"},
            {"type": "Document", "name": "MetaAddRegistrar"},
            {"type": "ChartOfAccounts", "name": "MetaAddAccounts"},
            {"type": "ChartOfCalculationTypes", "name": "MetaAddCalculationTypes"},
            {"type": "CommonModule", "name": "MetaAddHandlers", "server": true}
        ]))
        .unwrap(),
    )
    .unwrap();
    let compile_args = Map::from_iter([
        (
            "cwd".to_string(),
            Value::String(workspace.path().display().to_string()),
        ),
        (
            "JsonPath".to_string(),
            Value::String(prerequisites.display().to_string()),
        ),
        ("OutputDir".to_string(), Value::String("src".to_string())),
        ("dryRun".to_string(), Value::Bool(false)),
    ]);
    let compiled =
        compile_legacy_metadata_fixture(&compile_args).expect("prerequisite metadata compile");
    assert!(compiled.ok, "{:?}", compiled.errors);
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

fn add_args(workspace: &Path, kind: &str, name: &str, dry_run: bool) -> Map<String, Value> {
    let _ = workspace;
    Map::from_iter([
        ("sourceSet".to_string(), Value::String("main".to_string())),
        ("kind".to_string(), Value::String(kind.to_string())),
        ("name".to_string(), Value::String(name.to_string())),
        ("dryRun".to_string(), Value::Bool(dry_run)),
    ])
}

fn call_add(workspace: &Path, kind: &str, name: &str, dry_run: bool) -> OperationResult {
    let previous = std::env::current_dir().unwrap();
    std::env::set_current_dir(workspace).unwrap();
    let result = UnicaApplication::new()
        .call_tool("unica.meta.add", &add_args(workspace, kind, name, dry_run));
    std::env::set_current_dir(previous).unwrap();
    result.expect("internal meta.add call")
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
    let previous = std::env::current_dir().unwrap();
    std::env::set_current_dir(workspace.path()).unwrap();
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let result = UnicaApplication::new()
        .call_tool_cancellable(
            "unica.meta.add",
            &add_args(workspace.path(), "Catalog", "Cancelled", false),
            cancellation,
        )
        .unwrap();
    std::env::set_current_dir(previous).unwrap();
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

fn assert_partial_is_stable(workspace: &TempWorkspace, kind: &str, name: &str) {
    let before = tree_snapshot(&workspace.path().join("src"));
    let result = call_add(workspace.path(), kind, name, false);
    assert!(!result.ok, "{kind}.{name} unexpectedly succeeded");
    assert_eq!(result.diagnostics.unwrap()[0]["code"], "already_exists");
    assert_eq!(tree_snapshot(&workspace.path().join("src")), before);
}

fn tree_snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn visit(root: &Path, current: &Path, output: &mut BTreeMap<PathBuf, Vec<u8>>) {
        let mut entries = std::fs::read_dir(current)
            .unwrap()
            .map(|entry| entry.unwrap())
            .collect::<Vec<_>>();
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let path = entry.path();
            let metadata = entry.metadata().unwrap();
            if metadata.is_dir() {
                visit(root, &path, output);
            } else if metadata.is_file() {
                output.insert(
                    path.strip_prefix(root).unwrap().to_path_buf(),
                    std::fs::read(path).unwrap(),
                );
            }
        }
    }
    let mut output = BTreeMap::new();
    visit(root, root, &mut output);
    output
}
