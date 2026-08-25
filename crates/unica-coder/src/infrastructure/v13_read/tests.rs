use super::{
    project_known_suffix, project_typed_payload, resolve_platform_xml_target,
    LogicalViewReadAuthority, MetadataAddress, SourceTarget, TargetKindPolicy,
    PLATFORM_XML_8_3_27_FORMAT_2_20,
};
use crate::application::result_store::ViewCursorStore;
use crate::application::v13::view::{ViewRequest, ViewService};
use crate::domain::address::QualifiedAddress;
use crate::domain::cancellation::CancellationToken;
use crate::domain::platform_profile::PlatformProfile;
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::logical_tree::route_logical_address;
use crate::infrastructure::platform::filesystem::RetainedDirectoryCapability;
use crate::infrastructure::source_revision::SourceRevisionService;
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[test]
fn typed_projection_keeps_summary_compact_and_content_address_selected() {
    let summary =
        QualifiedAddress::parse("main:Report.Продажи.Template.Схема.DataSet.Продажи").unwrap();
    let route = route_logical_address(&summary, PlatformProfile::v8_3_27()).unwrap();
    let payload = json!({
        "support": {"state": "Editable"},
        "dataSets": [{"name": "Продажи", "kind": "Query", "query": "SELECT\n  *"}],
        "sourceSet": "main",
        "metadataPath": "Report.Продажи.Template.Схема"
    });
    let view =
        serde_json::to_value(project_typed_payload(&route, payload.clone()).unwrap()).unwrap();
    assert!(!view.to_string().contains("SELECT"));
    assert!(view.get("sourceSet").is_none());
    assert!(view["branches"].as_array().unwrap().iter().any(|branch| {
        branch["at"] == "main:Report.Продажи.Template.Схема.DataSet.Продажи.Query"
    }));

    let query = QualifiedAddress::parse("main:Report.Продажи.Template.Схема.DataSet.Продажи.Query")
        .unwrap();
    let data = project_known_suffix(
        crate::infrastructure::logical_tree::LogicalReader::Dcs,
        &query,
        &payload,
        &query.segments()[2..],
    )
    .unwrap();
    let view = serde_json::to_value(data).unwrap();
    assert_eq!(view["items"][0], json!({"line": 1, "text": "SELECT"}));
    assert!(view["items"][0].get("at").is_none());
}

#[test]
fn typed_projection_never_leaks_provider_or_physical_slots() {
    let at = QualifiedAddress::parse("main:Subsystem.Продажи").unwrap();
    let route = route_logical_address(&at, PlatformProfile::v8_3_27()).unwrap();
    let view = serde_json::to_value(
        project_typed_payload(
            &route,
            json!({
                "name": "Продажи",
                "location": {"path": "/secret/Subsystems/Продажи.xml"},
                "fileExists": true,
                "provider": "raw",
                "content": ["Catalog.Товары"],
                "commandInterface": {"visibility": []}
            }),
        )
        .unwrap(),
    )
    .unwrap();
    let text = view.to_string();
    for forbidden in ["/secret", "fileExists", "provider", "sourceState", "layout"] {
        assert!(!text.contains(forbidden), "leaked {forbidden}: {text}");
    }
}

#[test]
fn typed_projection_rejects_unknown_provider_payload_instead_of_dumping_it() {
    let at = QualifiedAddress::parse("main:Configuration").unwrap();
    let route = route_logical_address(&at, PlatformProfile::v8_3_27()).unwrap();

    let error = project_typed_payload(
        &route,
        json!({"name": "Main", "mysteryProviderPayload": {"raw": "secret"}}),
    )
    .unwrap_err();

    assert_eq!(error.code(), "provider_unavailable");
}

#[test]
fn real_typed_readers_cover_every_task14_profile_without_skipping() {
    let fixture = RealReaderFixture::new();
    let cancellation = CancellationToken::new();
    let source_root = Arc::new(RetainedDirectoryCapability::open(&fixture.source).unwrap());
    let revisions = Arc::new(
        SourceRevisionService::new_reconciling_for_test(&fixture.context, &fixture.source).unwrap(),
    );
    let metadata_target =
        MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "Catalog.Items").unwrap();
    let metadata_resolution = resolve_platform_xml_target(
        &fixture.context,
        &SourceTarget {
            source_set: "main".to_string(),
            metadata_path: Some(metadata_target),
        },
        TargetKindPolicy::Any,
    );
    assert!(metadata_resolution.is_ok(), "{metadata_resolution:?}");
    let authority = LogicalViewReadAuthority::new(
        &fixture.context,
        &cancellation,
        "main",
        "actor-fixture-main",
        revisions,
        source_root,
        PlatformProfile::v8_3_27(),
    );
    let service = ViewService::new(authority, ViewCursorStore::default());
    let cases = [
        ("configuration", "main:Configuration", "Configuration"),
        ("metadata", "main:Catalog.Items", "Catalog"),
        ("form", "main:Report.ParityReport.Form.MainForm", "Form"),
        ("role-rls", "main:Role.SalesReader.RLS", "RLS"),
        ("subsystem", "main:Subsystem.Sales", "Subsystem"),
        ("interface", "main:Subsystem.Sales.Interface", "Interface"),
        (
            "dcs",
            "main:Report.ParityReport.Template.MainSchema.DataSet",
            "DataSet",
        ),
        (
            "mxl",
            "main:Report.ParityReport.Template.Print.Area",
            "Area",
        ),
        (
            "xdto",
            "main:XDTOPackage.EnterpriseData_1_17_3.Type",
            "Type",
        ),
        (
            "module",
            "main:CommonModule.РеактивныйСервер.Method",
            "Method",
        ),
        (
            "form-module-binding",
            "main:Report.ParityReport.Form.MainForm.Module.Form.Method.OnOpen",
            "Method",
        ),
    ];
    for (case, at, kind) in cases {
        let result = service.view(ViewRequest::new(at).unwrap());
        assert!(
            result.ok,
            "{case}: {} {:?}",
            result.summary, result.diagnostics
        );
        let data = result
            .data
            .as_ref()
            .unwrap_or_else(|| panic!("{case}: missing data"));
        assert_eq!(data["kind"], kind, "{case}: {data}");
        assert!(!data
            .to_string()
            .contains(fixture.root.path().to_string_lossy().as_ref()));
        if case == "form-module-binding" {
            assert!(
                data["props"]["handles"]
                    .as_array()
                    .is_some_and(|handles| !handles.is_empty()),
                "{data}"
            );
        }
    }
}

#[test]
fn websocket_client_source_view_is_an_explicit_provider_gap() {
    let fixture = RealReaderFixture::new();
    let cancellation = CancellationToken::new();
    let source_root = Arc::new(RetainedDirectoryCapability::open(&fixture.source).unwrap());
    let revisions = Arc::new(
        SourceRevisionService::new_reconciling_for_test(&fixture.context, &fixture.source).unwrap(),
    );
    let authority = LogicalViewReadAuthority::new(
        &fixture.context,
        &cancellation,
        "main",
        "actor-fixture-main",
        revisions,
        source_root,
        PlatformProfile::v8_3_27(),
    );

    let result = ViewService::new(authority, ViewCursorStore::default())
        .view(ViewRequest::new("main:WebSocketClient.Телефония.Module.WebSocketClient").unwrap());

    assert!(!result.ok);
    assert!(result.data.is_none());
    assert_eq!(result.diagnostics[0]["code"], "provider_unavailable");
}

#[test]
fn logical_reader_parity_contract_is_complete() {
    real_typed_readers_cover_every_task14_profile_without_skipping();
    typed_projection_never_leaks_provider_or_physical_slots();
    typed_projection_rejects_unknown_provider_payload_instead_of_dumping_it();
    websocket_client_source_view_is_an_explicit_provider_gap();
}

struct RealReaderFixture {
    root: tempfile::TempDir,
    context: WorkspaceContext,
    source: PathBuf,
}

impl RealReaderFixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("src");
        let cache = root.path().join("cache");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&cache).unwrap();
        fs::write(
                root.path().join("v8project.yaml"),
                "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
            )
            .unwrap();
        let config = fixture_text("xdto/enterprise-data-minimal/Configuration.xml");
        fs::write(
                source.join("Configuration.xml"),
                replace_child_objects(
                    &config,
                    "\n\t\t\t<Catalog>Items</Catalog>\n\t\t\t<Report>ParityReport</Report>\n\t\t\t<Role>SalesReader</Role>\n\t\t\t<Subsystem>Sales</Subsystem>\n\t\t\t<XDTOPackage>EnterpriseData_1_17_3</XDTOPackage>\n\t\t\t<CommonModule>РеактивныйСервер</CommonModule>\n\t\t",
                ),
            )
            .unwrap();
        write(
            &source.join("Catalogs/Items.xml"),
            &with_root_version(&fixture_text(
                "platform_8_3_27/meta_info/edge/catalog-child-kinds.xml",
            )),
        );
        let report = fixture_text("unica_mcp_script_parity/form-remove/ParityReport.xml");
        fs::create_dir_all(source.join("Reports")).unwrap();
        fs::write(
                source.join("Reports/ParityReport.xml"),
                replace_child_objects(
                    &report,
                    "\n\t\t\t<Form>MainForm</Form>\n\t\t\t<Template>MainSchema</Template>\n\t\t\t<Template>Print</Template>\n\t\t",
                ),
            )
            .unwrap();
        copy_fixture(
            "unica_mcp_script_parity/form-remove/ParityReport/Forms/MainForm.xml",
            &source.join("Reports/ParityReport/Forms/MainForm.xml"),
        );
        let form = fixture_text(
            "unica_mcp_script_parity/form-remove/ParityReport/Forms/MainForm/Ext/Form.xml",
        )
        .replace(
            "\n</Form>",
            "\n\t<Events>\n\t\t<Event name=\"OnOpen\">OnOpen</Event>\n\t</Events>\n</Form>",
        );
        write(
            &source.join("Reports/ParityReport/Forms/MainForm/Ext/Form.xml"),
            &form,
        );
        write(
            &source.join("Reports/ParityReport/Forms/MainForm/Ext/Form/Module.bsl"),
            "&AtClient\nProcedure OnOpen()\nEndProcedure\n",
        );
        write(
            &source.join("Roles/SalesReader.xml"),
            &with_root_version(&fixture_text(
                "unica_mcp_script_parity/role-info/SalesReader.xml",
            )),
        );
        write(
            &source.join("Roles/SalesReader/Ext/Rights.xml"),
            &fixture_text("unica_mcp_script_parity/role-info/SalesReader/Ext/Rights.xml")
                .replace("version=\"2.17\"", "version=\"2.20\""),
        );
        let subsystem = r#"<?xml version="1.0" encoding="utf-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" version="2.20">
  <Subsystem><Properties><Name>Sales</Name><Synonym><v8:item><v8:lang>ru</v8:lang><v8:content>Продажи</v8:content></v8:item></Synonym><IncludeInCommandInterface>true</IncludeInCommandInterface><UseOneCommand>false</UseOneCommand><Content><xr:Item xsi:type="xr:MDObjectRef">Catalog.Items</xr:Item></Content></Properties><ChildObjects/></Subsystem>
</MetaDataObject>"#;
        write(&source.join("Subsystems/Sales.xml"), subsystem);
        copy_fixture(
            "unica_mcp_script_parity/meta-remove/Subsystems/Sales/Ext/CommandInterface.xml",
            &source.join("Subsystems/Sales/Ext/CommandInterface.xml"),
        );
        copy_fixture(
            "unica_mcp_script_parity/template-remove/ParityReport/Templates/MainSchema.xml",
            &source.join("Reports/ParityReport/Templates/MainSchema.xml"),
        );
        copy_fixture(
                "unica_mcp_script_parity/template-remove/ParityReport/Templates/MainSchema/Ext/Template.xml",
                &source.join("Reports/ParityReport/Templates/MainSchema/Ext/Template.xml"),
            );
        let print_descriptor = fixture_text(
            "unica_mcp_script_parity/template-remove/ParityReport/Templates/MainSchema.xml",
        )
        .replace("MainSchema", "Print")
        .replace("DataCompositionSchema", "SpreadsheetDocument");
        write(
            &source.join("Reports/ParityReport/Templates/Print.xml"),
            &print_descriptor,
        );
        copy_fixture(
            "platform_8_3_27/mxl/Template.xml",
            &source.join("Reports/ParityReport/Templates/Print/Ext/Template.xml"),
        );
        copy_fixture(
            "xdto/enterprise-data-minimal/XDTOPackages/EnterpriseData_1_17_3.xml",
            &source.join("XDTOPackages/EnterpriseData_1_17_3.xml"),
        );
        copy_fixture(
            "xdto/enterprise-data-minimal/XDTOPackages/EnterpriseData_1_17_3/Ext/Package.bin",
            &source.join("XDTOPackages/EnterpriseData_1_17_3/Ext/Package.bin"),
        );
        copy_fixture(
            "platform_8_3_27/support-edit-bin-only/src/CommonModules/РеактивныйСервер.xml",
            &source.join("CommonModules/РеактивныйСервер.xml"),
        );
        copy_fixture(
                "platform_8_3_27/support-edit-bin-only/src/CommonModules/РеактивныйСервер/Ext/Module.bsl",
                &source.join("CommonModules/РеактивныйСервер/Ext/Module.bsl"),
            );
        let canonical_root = fs::canonicalize(root.path()).unwrap();
        let source = fs::canonicalize(&source).unwrap();
        let context = WorkspaceContext {
            cwd: canonical_root.clone(),
            workspace_root: canonical_root,
            cache_root: cache,
            workspace_epoch: 1,
        };
        Self {
            root,
            context,
            source,
        }
    }
}

fn fixture_path(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(relative)
}

fn fixture_text(relative: &str) -> String {
    fs::read_to_string(fixture_path(relative)).unwrap()
}

fn copy_fixture(relative: &str, destination: &Path) {
    fs::create_dir_all(destination.parent().unwrap()).unwrap();
    fs::copy(fixture_path(relative), destination).unwrap();
}

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn replace_child_objects(document: &str, body: &str) -> String {
    let start = document.find("<ChildObjects>").unwrap();
    let end = document.find("</ChildObjects>").unwrap() + "</ChildObjects>".len();
    format!(
        "{}<ChildObjects>{body}</ChildObjects>{}",
        &document[..start],
        &document[end..]
    )
}

fn with_root_version(document: &str) -> String {
    let start = document.find("<MetaDataObject").unwrap();
    let end = document[start..].find('>').unwrap() + start;
    if document[start..end].contains("version=") {
        return document.to_string();
    }
    format!("{} version=\"2.20\"{}", &document[..end], &document[end..])
}
