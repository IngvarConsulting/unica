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
use crate::infrastructure::v13_read_port::ProviderReadAuthority;
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
fn actor_owned_reader_never_follows_a_source_set_remap_after_admission() {
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
    let service = ViewService::new(authority, ViewCursorStore::default());
    let admitted = service.view(ViewRequest::new("main:Configuration").unwrap());
    assert!(admitted.ok, "{:?}", admitted.diagnostics);
    assert_eq!(
        admitted.data.as_ref().unwrap()["props"]["name"],
        "CorpusConfiguration"
    );

    let replacement = fixture.root.path().join("replacement");
    fs::create_dir_all(&replacement).unwrap();
    fs::write(
        replacement.join("Configuration.xml"),
        fixture_text("xdto/enterprise-data-minimal/Configuration.xml").replace(
            "<Name>CorpusConfiguration</Name>",
            "<Name>ReplacementConfiguration</Name>",
        ),
    )
    .unwrap();
    fs::write(
        fixture.root.path().join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: replacement\n",
    )
    .unwrap();

    let after_remap = service.view(ViewRequest::new("main:Configuration").unwrap());
    assert!(after_remap.ok, "{:?}", after_remap.diagnostics);
    assert_eq!(after_remap.rev, admitted.rev);
    assert_eq!(
        after_remap.data.as_ref().unwrap()["props"]["name"],
        "CorpusConfiguration",
        "a retained reader must never combine replacement bytes with the admitted source revision",
    );
}

#[test]
fn configuration_root_branch_count_matches_the_reachable_catalog_collection() {
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
    let service = ViewService::new(authority, ViewCursorStore::default());

    let root = service.view(ViewRequest::new("main:Configuration").unwrap());
    let catalog_branch = root.data.as_ref().unwrap()["branches"]
        .as_array()
        .unwrap()
        .iter()
        .find(|branch| branch["at"] == "main:Catalog")
        .expect("configuration root must expose its registered Catalog branch");
    assert_eq!(catalog_branch["count"], 1);
}

#[test]
fn capability_bound_configuration_reader_preserves_complete_cf_info_semantics() {
    let fixture = RealReaderFixture::new();
    let cancellation = CancellationToken::new();
    let root = Arc::new(RetainedDirectoryCapability::open(&fixture.source).unwrap());
    let revisions = Arc::new(
        SourceRevisionService::new_reconciling_for_test(&fixture.context, &fixture.source).unwrap(),
    );
    let reader = ProviderReadAuthority::new("main", "actor-fixture-main", root, revisions);
    reader
        .exact_revision(
            crate::domain::code_intelligence::ProviderDeadline::from_budget(
                std::time::Duration::from_secs(7),
            ),
            &cancellation,
        )
        .unwrap();

    let payload = reader.configuration_payload().unwrap();
    assert_eq!(
        payload["properties"]["defaultRunMode"],
        "ManagedApplication"
    );
    assert_eq!(payload["support"]["state"], "notSupported");
    assert_eq!(payload["totalObjects"], 6);
    assert_eq!(
        payload["childObjects"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["count"].as_u64().unwrap())
            .sum::<u64>(),
        6,
    );
    assert_eq!(payload["registeredObjects"].as_array().unwrap().len(), 6);
}

#[test]
fn metadata_kind_branch_lists_registered_objects_with_canonical_addresses() {
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
    let service = ViewService::new(authority, ViewCursorStore::default());

    let branch = service.view(ViewRequest::new("main:Catalog").unwrap());
    assert!(branch.ok, "{} {:?}", branch.summary, branch.diagnostics);
    assert_eq!(
        branch.data.as_ref().unwrap()["items"],
        json!([{
            "at": "main:Catalog.Items",
            "kind": "Catalog",
            "title": "Items"
        }]),
    );
}

#[test]
fn metadata_tabular_section_attribute_consumes_the_complete_suffix() {
    let fixture = RealReaderFixture::new();
    let service = fixture.view_service();

    let result = service.view(
        ViewRequest::new("main:Catalog.Items.TabularSection.Lines.Attribute.Quantity").unwrap(),
    );

    assert!(result.ok, "{} {:?}", result.summary, result.diagnostics);
    let data = result.data.as_ref().unwrap();
    assert_eq!(
        data["at"],
        "main:Catalog.Items.TabularSection.Lines.Attribute.Quantity"
    );
    assert_eq!(data["kind"], "Attribute");
    assert_eq!(data["title"], "Quantity");
}

#[test]
fn role_right_rls_consumes_the_complete_suffix() {
    let fixture = RealReaderFixture::new();
    let service = fixture.view_service();

    let result =
        service.view(ViewRequest::new("main:Role.SalesReader.Right.Products.RLS.View").unwrap());

    assert!(result.ok, "{} {:?}", result.summary, result.diagnostics);
    let data = result.data.as_ref().unwrap();
    assert_eq!(data["at"], "main:Role.SalesReader.Right.Products.RLS.View");
    assert_eq!(data["kind"], "RLS");
    assert_eq!(data["title"], "View");
}

#[test]
fn form_table_column_event_consumes_arbitrary_depth_and_preserves_owner_address() {
    let fixture = RealReaderFixture::new();
    let service = fixture.view_service();
    let event_at = "main:Report.ParityReport.Form.MainForm.Item.Goods.Item.Quantity.Event.OnChange";

    let event = service.view(ViewRequest::new(event_at).unwrap());
    assert!(event.ok, "{} {:?}", event.summary, event.diagnostics);
    let event_data = event.data.as_ref().unwrap();
    assert_eq!(event_data["at"], event_at);
    assert_eq!(event_data["kind"], "Event");

    let method = service.view(
        ViewRequest::new(
            "main:Report.ParityReport.Form.MainForm.Module.Form.Method.QuantityOnChange",
        )
        .unwrap(),
    );
    assert!(method.ok, "{} {:?}", method.summary, method.diagnostics);
    let method_data = method.data.as_ref().unwrap();
    assert!(
        method_data["props"]["handles"]
            .as_array()
            .is_some_and(|handles| handles.iter().any(|handle| {
                handle["owner"] == "column"
                    && handle["at"]
                        == "main:Report.ParityReport.Form.MainForm.Item.Goods.Item.Quantity"
                    && handle["event"] == "OnChange"
            })),
        "{method_data}"
    );
}

#[test]
fn dcs_dataset_field_and_parameter_consume_the_complete_suffix() {
    let fixture = RealReaderFixture::new();
    let service = fixture.view_service();
    for (at, kind, title) in [
        (
            "main:Report.ParityReport.Template.MainSchema.DataSet.MainData.Field.Code",
            "Field",
            "Code",
        ),
        (
            "main:Report.ParityReport.Template.MainSchema.DataSet.MainData.Parameter.Period",
            "Parameter",
            "Period",
        ),
    ] {
        let result = service.view(ViewRequest::new(at).unwrap());
        assert!(
            result.ok,
            "{at}: {} {:?}",
            result.summary, result.diagnostics
        );
        let data = result.data.as_ref().unwrap();
        assert_eq!(data["at"], at);
        assert_eq!(data["kind"], kind);
        assert_eq!(data["title"], title);
    }
}

#[test]
fn mxl_area_parameter_consumes_the_complete_suffix() {
    let fixture = RealReaderFixture::new();
    let service = fixture.view_service();
    let at = "main:Report.ParityReport.Template.Print.Area.Header.Parameter.Title";

    let result = service.view(ViewRequest::new(at).unwrap());

    assert!(result.ok, "{} {:?}", result.summary, result.diagnostics);
    let data = result.data.as_ref().unwrap();
    assert_eq!(data["at"], at);
    assert_eq!(data["kind"], "Parameter");
    assert_eq!(data["title"], "Title");
}

#[test]
fn unsupported_view_filter_is_a_typed_bad_value_instead_of_a_noop() {
    let fixture = RealReaderFixture::new();
    let result =
        fixture
            .view_service()
            .view(ViewRequest::new("main:Configuration").unwrap().with_filter(
                serde_json::Map::from_iter([("mystery".to_string(), json!(true))]),
            ));

    assert!(!result.ok);
    assert_eq!(result.diagnostics[0]["code"], "bad_value");
}

#[test]
fn module_body_context_filter_excludes_at_client_source_from_server_slice() {
    let fixture = RealReaderFixture::new();
    let result = fixture.view_service().view(
        ViewRequest::new("main:Report.ParityReport.Form.MainForm.Module.Form.Body")
            .unwrap()
            .with_filter(serde_json::Map::from_iter([(
                "context".to_string(),
                json!("server"),
            )])),
    );

    assert!(result.ok, "{} {:?}", result.summary, result.diagnostics);
    assert_eq!(result.data.as_ref().unwrap()["items"], json!([]));
}

#[test]
fn module_method_public_filter_returns_only_export_methods() {
    let fixture = RealReaderFixture::new();
    let result = fixture.view_service().view(
        ViewRequest::new("main:CommonModule.РеактивныйСервер.Method")
            .unwrap()
            .with_filter(serde_json::Map::from_iter([(
                "public".to_string(),
                json!(true),
            )])),
    );

    assert!(result.ok, "{} {:?}", result.summary, result.diagnostics);
    let items = result.data.as_ref().unwrap()["items"].as_array().unwrap();
    assert_eq!(items.len(), 1, "{items:?}");
    assert_eq!(items[0]["props"]["export"], true);
}

#[test]
fn every_reader_rejects_an_extra_unconsumed_address_tail() {
    let cases = [
        (
            "main:XDTOPackage.EnterpriseData_1_17_3.Type.Order.Property.Id.Event.Change",
            json!({
                "targetNamespace": "urn:test",
                "types": [{"name": "Order", "properties": [{"name": "Id"}]}],
                "globalProperties": [],
                "counts": {},
                "imports": [],
                "findings": []
            }),
        ),
        (
            "main:Subsystem.Sales.Interface.main.Command.Open.Event.Change",
            json!({
                "name": "Sales",
                "content": [],
                "children": [],
                "commandInterface": {
                    "visibility": [{"command": "Open", "visible": true}],
                    "placement": []
                }
            }),
        ),
    ];
    for (at, payload) in cases {
        let address = QualifiedAddress::parse(at).unwrap();
        let route = route_logical_address(&address, PlatformProfile::v8_3_27()).unwrap();
        let error = project_typed_payload(&route, payload).unwrap_err();
        assert_eq!(error.code(), "not_found", "{at}");
    }

    let fixture = RealReaderFixture::new();
    let module = fixture.view_service().view(
        ViewRequest::new(
            "main:Report.ParityReport.Form.MainForm.Module.Form.Method.OnOpen.Body.source.Event.Change",
        )
        .unwrap(),
    );
    assert!(!module.ok, "{:?}", module.data);
    assert_eq!(module.diagnostics[0]["code"], "not_found");
}

#[test]
fn form_projection_uses_a_positive_nested_scalar_allowlist() {
    let at = QualifiedAddress::parse("main:CommonForm.Test.Item.Secret").unwrap();
    let route = route_logical_address(&at, PlatformProfile::v8_3_27()).unwrap();
    let view = project_typed_payload(
        &route,
        json!({
            "name": "Test",
            "elements": [{
                "name": "Secret",
                "tag": "[Input]",
                "visible": true,
                "enabled": true,
                "readOnly": false,
                "mysteryProviderScalar": "must-not-leak",
                "events": [],
                "children": []
            }],
            "attributes": [],
            "parameters": [],
            "commands": [],
            "events": []
        }),
    )
    .unwrap();

    let serialized = serde_json::to_string(&view).unwrap();
    assert!(
        !serialized.contains("mysteryProviderScalar"),
        "{serialized}"
    );
    assert!(!serialized.contains("must-not-leak"), "{serialized}");
}

#[test]
fn role_right_projection_never_serializes_an_unbounded_rights_array_into_props() {
    let at = QualifiedAddress::parse("main:Role.Test.Right.Products").unwrap();
    let route = route_logical_address(&at, PlatformProfile::v8_3_27()).unwrap();
    let rights = (0..500)
        .map(|index| json!({"name": format!("Right{index}"), "restricted": false}))
        .collect::<Vec<_>>();
    let view = project_typed_payload(
        &route,
        json!({
            "name": "Test",
            "allowed": [{"kind": "Catalog", "objects": [{"name": "Products", "rights": rights}]}],
            "denied": [],
            "restrictedObjects": [],
            "templates": [],
            "totals": {"allowed": 500, "denied": 0}
        }),
    )
    .unwrap();
    let serialized = serde_json::to_value(view).unwrap();

    assert!(serialized["props"].get("rights").is_none(), "{serialized}");
    assert!(serialized["props"]
        .as_object()
        .unwrap()
        .values()
        .all(|value| value.as_str().is_none_or(|text| text.len() <= 2_048)));
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
    cancellation: CancellationToken,
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
        let catalog = with_root_version(&fixture_text(
            "platform_8_3_27/meta_info/edge/catalog-child-kinds.xml",
        ))
        .replace(
            "<TabularSection><Properties><Name>Lines</Name></Properties><ChildObjects/></TabularSection>",
            "<TabularSection><Properties><Name>Lines</Name></Properties><ChildObjects><Attribute><Properties><Name>Quantity</Name><Type><v8:Type>xs:decimal</v8:Type></Type></Properties></Attribute></ChildObjects></TabularSection>",
        );
        write(&source.join("Catalogs/Items.xml"), &catalog);
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
        let form = r#"<?xml version="1.0" encoding="utf-8"?>
<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20">
  <ChildItems>
    <Table name="Goods">
      <ChildItems>
        <InputField name="Quantity">
          <Events><Event name="OnChange">QuantityOnChange</Event></Events>
        </InputField>
      </ChildItems>
    </Table>
  </ChildItems>
  <Events><Event name="OnOpen">OnOpen</Event></Events>
</Form>"#;
        write(
            &source.join("Reports/ParityReport/Forms/MainForm/Ext/Form.xml"),
            &form,
        );
        write(
            &source.join("Reports/ParityReport/Forms/MainForm/Ext/Form/Module.bsl"),
            "&AtClient\nProcedure OnOpen()\nEndProcedure\n\n&AtClient\nProcedure QuantityOnChange()\nEndProcedure\n",
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
        let dcs = fixture_text("unica_mcp_script_parity/dcs-validate/BadPrefix.xml")
            .replace(
                "xmlns:bad=\"http://example.com\">bad:CatalogRef.X",
                ">xs:string",
            )
            .replace(
                "\n</DataCompositionSchema>",
                "\n\t<parameter><name>Period</name><valueType><v8:Type>xs:dateTime</v8:Type></valueType></parameter>\n</DataCompositionSchema>",
            );
        write(
            &source.join("Reports/ParityReport/Templates/MainSchema/Ext/Template.xml"),
            &dcs,
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
        let mxl = fixture_text("platform_8_3_27/mxl/Template.xml")
            .replace(
                "\n\t<templateMode>true</templateMode>",
                "\n\t<namedItem xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xsi:type=\"NamedItemCells\"><name>Header</name><area><type>Rows</type><beginRow>0</beginRow><endRow>0</endRow><beginColumn>-1</beginColumn><endColumn>-1</endColumn></area></namedItem>\n\t<templateMode>true</templateMode>",
            )
            .replace(
                "\n\t\t\t<empty>true</empty>",
                "\n\t\t\t<c><c><f>0</f><parameter>Title</parameter></c></c>",
            );
        write(
            &source.join("Reports/ParityReport/Templates/Print/Ext/Template.xml"),
            &mxl,
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
        let common_module_path = source.join("CommonModules/РеактивныйСервер/Ext/Module.bsl");
        let mut common_module = fs::read_to_string(&common_module_path).unwrap();
        common_module.push_str("\nПроцедура InternalService()\nКонецПроцедуры\n");
        fs::write(common_module_path, common_module).unwrap();
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
            cancellation: CancellationToken::new(),
        }
    }

    fn view_service(&self) -> ViewService<LogicalViewReadAuthority<'_>> {
        let source_root = Arc::new(RetainedDirectoryCapability::open(&self.source).unwrap());
        let revisions = Arc::new(
            SourceRevisionService::new_reconciling_for_test(&self.context, &self.source).unwrap(),
        );
        let authority = LogicalViewReadAuthority::new(
            &self.context,
            &self.cancellation,
            "main",
            "actor-fixture-main",
            revisions,
            source_root,
            PlatformProfile::v8_3_27(),
        );
        ViewService::new(authority, ViewCursorStore::default())
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
