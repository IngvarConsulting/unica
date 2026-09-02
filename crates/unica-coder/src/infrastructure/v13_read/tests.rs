use super::{
    project_known_suffix, project_typed_payload, resolve_platform_xml_target,
    review_set_after_canonical_role_read, review_set_before_owner_proof, LogicalViewReadAuthority,
    MetadataAddress, SourceTarget, TargetKindPolicy, PLATFORM_XML_8_3_27_FORMAT_2_20,
};
use crate::application::result_store::ViewCursorStore;
use crate::application::v13::find::FindRequest;
use crate::application::v13::view::{ViewRequest, ViewService};
use crate::domain::address::QualifiedAddress;
use crate::domain::cancellation::CancellationToken;
use crate::domain::code_intelligence::ProviderDeadline;
use crate::domain::platform_profile::PlatformProfile;
use crate::domain::project_sources::SourceSetKind;
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::logical_tree::route_logical_address;
use crate::infrastructure::platform::filesystem::{
    supports_retained_root_replacement_test, RetainedDirectoryCapability,
};
use crate::infrastructure::source_revision::SourceRevisionService;
use crate::infrastructure::v13_find::{ActorFindSource, WorkspaceFindIndexBuilder};
use crate::infrastructure::v13_read_port::{
    review_clear_revision_identity_hooks, review_set_revision_identity_hooks, ProviderReadAuthority,
};
use serde::Deserialize;
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

fn configuration_payload(
    reader: &ProviderReadAuthority,
) -> Result<serde_json::Value, crate::application::v13::view::ViewError> {
    reader.configuration_payload_with_checkpoint(&mut || Ok(()))
}

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
        &cancellation,
        "main",
        "actor-fixture-main",
        SourceSetKind::Configuration,
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
fn actor_owned_configuration_support_and_home_page_sidecars_are_retained() {
    let fixture = RealReaderFixture::new();
    fs::create_dir_all(fixture.source.join("Ext")).unwrap();
    fs::copy(
        fixture_path("platform_8_3_27/support-edit-bin-only/src/Ext/ParentConfigurations.bin"),
        fixture.source.join("Ext/ParentConfigurations.bin"),
    )
    .unwrap();
    fs::copy(
        fixture_path("unica_mcp_script_parity/cf-info/Ext/HomePageWorkArea.xml"),
        fixture.source.join("Ext/HomePageWorkArea.xml"),
    )
    .unwrap();
    let cancellation = CancellationToken::new();
    let source_root = Arc::new(RetainedDirectoryCapability::open(&fixture.source).unwrap());
    let revisions = Arc::new(
        SourceRevisionService::new_reconciling_for_test(&fixture.context, &fixture.source).unwrap(),
    );
    let authority = LogicalViewReadAuthority::new(
        &cancellation,
        "main",
        "actor-fixture-main",
        SourceSetKind::Configuration,
        revisions,
        source_root,
        PlatformProfile::v8_3_27(),
    );
    let route = route_logical_address(
        &QualifiedAddress::parse("main:Configuration").unwrap(),
        PlatformProfile::v8_3_27(),
    )
    .unwrap();
    let admitted = authority.typed_payload(&route).unwrap();
    assert_eq!(admitted["support"]["state"], "supported");
    assert_eq!(admitted["homePage"]["template"], "TwoColumns");

    let replacement = fixture.root.path().join("replacement");
    fs::create_dir_all(&replacement).unwrap();
    fs::write(
        replacement.join("Configuration.xml"),
        fixture_text("xdto/enterprise-data-minimal/Configuration.xml"),
    )
    .unwrap();
    fs::write(
        fixture.root.path().join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: replacement\n",
    )
    .unwrap();

    let after_remap = authority.typed_payload(&route).unwrap();
    assert_eq!(after_remap["support"], admitted["support"]);
    assert_eq!(after_remap["homePage"], admitted["homePage"]);
}

#[test]
fn retained_home_page_distinguishes_missing_from_malformed_and_wrong_root() {
    let fixture = RealReaderFixture::new();
    fs::create_dir_all(fixture.source.join("Ext")).unwrap();
    let root = Arc::new(RetainedDirectoryCapability::open(&fixture.source).unwrap());
    let revisions = Arc::new(
        SourceRevisionService::new_reconciling_for_test(&fixture.context, &fixture.source).unwrap(),
    );
    let reader = ProviderReadAuthority::new(
        "main",
        "actor-fixture-main",
        SourceSetKind::Configuration,
        root,
        revisions,
    );
    let sidecar = fixture.source.join("Ext/HomePageWorkArea.xml");

    let missing = configuration_payload(&reader).unwrap();
    assert!(missing["homePage"].is_null());

    fs::write(&sidecar, "<broken>").unwrap();
    let malformed = configuration_payload(&reader).unwrap_err();
    assert_eq!(malformed.code(), "provider_unavailable");

    fs::write(
        &sidecar,
        r#"<NotHomePage xmlns="http://v8.1c.ru/8.3/xcf/extrnprops" version="2.20"/>"#,
    )
    .unwrap();
    let wrong_root = configuration_payload(&reader).unwrap_err();
    assert_eq!(wrong_root.code(), "provider_unavailable");

    fs::copy(
        fixture_path("unica_mcp_script_parity/cf-info/Ext/HomePageWorkArea.xml"),
        &sidecar,
    )
    .unwrap();
    let valid = configuration_payload(&reader).unwrap();
    assert_eq!(valid["homePage"]["template"], "TwoColumns");
}

#[test]
fn actor_supplied_extension_kind_preserves_extension_support_semantics() {
    let fixture = RealReaderFixture::new();
    let root = Arc::new(RetainedDirectoryCapability::open(&fixture.source).unwrap());
    let revisions = Arc::new(
        SourceRevisionService::new_reconciling_for_test(&fixture.context, &fixture.source).unwrap(),
    );
    let reader = ProviderReadAuthority::new(
        "extension",
        "actor-fixture-extension",
        SourceSetKind::Extension,
        root,
        revisions,
    );

    let payload = configuration_payload(&reader).unwrap();
    assert_eq!(payload["support"]["state"], "extension");
}

#[test]
fn actor_owned_typed_form_reader_never_follows_a_source_set_remap() {
    let fixture = RealReaderFixture::new();
    let service = fixture.view_service();
    let admitted = service
        .view(ViewRequest::new("main:Report.ParityReport.Form.MainForm.Item.Goods").unwrap());
    assert!(admitted.ok, "{:?}", admitted.diagnostics);

    let replacement = fixture.root.path().join("replacement");
    fs::create_dir_all(replacement.join("Reports/ParityReport/Forms/MainForm/Ext")).unwrap();
    fs::write(
        replacement.join("Configuration.xml"),
        replace_child_objects(
            &fixture_text("xdto/enterprise-data-minimal/Configuration.xml"),
            "<Report>ParityReport</Report>",
        ),
    )
    .unwrap();
    fs::copy(
        fixture.source.join("Reports/ParityReport.xml"),
        replacement.join("Reports/ParityReport.xml"),
    )
    .unwrap();
    fs::copy(
        fixture
            .source
            .join("Reports/ParityReport/Forms/MainForm.xml"),
        replacement.join("Reports/ParityReport/Forms/MainForm.xml"),
    )
    .unwrap();
    fs::write(
        replacement.join("Reports/ParityReport/Forms/MainForm/Ext/Form.xml"),
        r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20"><ChildItems><InputField name="Replacement"/></ChildItems></Form>"#,
    )
    .unwrap();
    fs::write(
        fixture.root.path().join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: replacement\n",
    )
    .unwrap();

    let replacement_only = service
        .view(ViewRequest::new("main:Report.ParityReport.Form.MainForm.Item.Replacement").unwrap());
    assert!(!replacement_only.ok, "{:?}", replacement_only.data);
    let retained = service
        .view(ViewRequest::new("main:Report.ParityReport.Form.MainForm.Item.Goods").unwrap());
    assert!(retained.ok, "{:?}", retained.diagnostics);
    assert_eq!(retained.rev, admitted.rev);
}

#[test]
fn actor_owned_module_reader_never_follows_a_source_set_remap() {
    let fixture = RealReaderFixture::new();
    let service = fixture.view_service();
    let address = "main:CommonModule.РеактивныйСервер.Method";
    let admitted = service.view(ViewRequest::new(address).unwrap());
    assert!(admitted.ok, "{:?}", admitted.diagnostics);
    assert!(admitted.data.as_ref().unwrap()["items"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["at"] == "main:CommonModule.РеактивныйСервер.Method.InternalService"));

    let replacement = fixture.root.path().join("replacement");
    fs::create_dir_all(replacement.join("CommonModules/РеактивныйСервер/Ext")).unwrap();
    fs::write(
        replacement.join("Configuration.xml"),
        replace_child_objects(
            &fixture_text("xdto/enterprise-data-minimal/Configuration.xml"),
            "<CommonModule>РеактивныйСервер</CommonModule>",
        ),
    )
    .unwrap();
    fs::copy(
        fixture.source.join("CommonModules/РеактивныйСервер.xml"),
        replacement.join("CommonModules/РеактивныйСервер.xml"),
    )
    .unwrap();
    fs::write(
        replacement.join("CommonModules/РеактивныйСервер/Ext/Module.bsl"),
        "Процедура ReplacementOnly() Экспорт\nКонецПроцедуры\n",
    )
    .unwrap();
    fs::write(
        fixture.root.path().join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: replacement\n",
    )
    .unwrap();

    let retained = service.view(ViewRequest::new(address).unwrap());
    assert!(retained.ok, "{:?}", retained.diagnostics);
    assert_eq!(retained.rev, admitted.rev);
    let items = retained.data.as_ref().unwrap()["items"].as_array().unwrap();
    assert!(items
        .iter()
        .any(|item| item["at"] == "main:CommonModule.РеактивныйСервер.Method.InternalService"));
    assert!(!items
        .iter()
        .any(|item| item["at"] == "main:CommonModule.РеактивныйСервер.Method.ReplacementOnly"));
}

#[test]
fn every_typed_reader_remains_on_the_admitted_root_after_source_set_remap() {
    let fixture = RealReaderFixture::new();
    let service = fixture.view_service();
    let addresses = [
        "main:Catalog.Items.Attribute.Code",
        "main:Role.SalesReader.Right.Catalog_Products.RLS.View",
        "main:Subsystem.Sales.Interface",
        "main:Report.ParityReport.Template.MainSchema.DataSet",
        "main:Report.ParityReport.Template.Print.Area",
        "main:XDTOPackage.EnterpriseData_1_17_3.Type",
    ];
    for address in addresses {
        let result = service.view(ViewRequest::new(address).unwrap());
        assert!(result.ok, "admission {address}: {:?}", result.diagnostics);
    }

    let replacement = fixture.root.path().join("replacement");
    fs::create_dir_all(&replacement).unwrap();
    fs::write(
        replacement.join("Configuration.xml"),
        fixture_text("xdto/enterprise-data-minimal/Configuration.xml"),
    )
    .unwrap();
    fs::write(
        fixture.root.path().join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: replacement\n",
    )
    .unwrap();

    for address in addresses {
        let result = service.view(ViewRequest::new(address).unwrap());
        assert!(
            result.ok,
            "retained read {address}: {} {:?}",
            result.summary, result.diagnostics
        );
    }
}

#[test]
fn configuration_root_branch_counts_match_every_reachable_collection() {
    let fixture = RealReaderFixture::new();
    let cancellation = CancellationToken::new();
    let source_root = Arc::new(RetainedDirectoryCapability::open(&fixture.source).unwrap());
    let revisions = Arc::new(
        SourceRevisionService::new_reconciling_for_test(&fixture.context, &fixture.source).unwrap(),
    );
    let authority = LogicalViewReadAuthority::new(
        &cancellation,
        "main",
        "actor-fixture-main",
        SourceSetKind::Configuration,
        revisions,
        source_root,
        PlatformProfile::v8_3_27(),
    );
    let service = ViewService::new(authority, ViewCursorStore::default());

    let root = service.view(ViewRequest::new("main:Configuration").unwrap());
    let branches = root.data.as_ref().unwrap()["branches"].as_array().unwrap();
    assert!(!branches.is_empty());
    for branch in branches {
        let at = branch["at"].as_str().unwrap();
        let collection = service.view(ViewRequest::new(at).unwrap());
        assert!(
            collection.ok,
            "{at}: {} {:?}",
            collection.summary, collection.diagnostics
        );
        let items = collection.data.as_ref().unwrap()["items"]
            .as_array()
            .unwrap();
        assert_eq!(
            branch["count"].as_u64().unwrap(),
            items.len() as u64,
            "{at}"
        );
        assert!(
            items.iter().all(|item| item.get("at").is_some()),
            "addressable branch rows must keep canonical addresses: {at}"
        );
    }
}

#[test]
fn module_capability_parents_expose_canonical_module_collections() {
    let fixture = RealReaderFixture::new();
    fixture.install_accepted_profile_sources();
    let service = fixture.view_service();
    let cases = [
        (
            "main:Configuration",
            "main:Module",
            4,
            vec![
                "main:Module.ManagedApplication",
                "main:Module.OrdinaryApplication",
                "main:Module.Session",
                "main:Module.ExternalConnection",
            ],
        ),
        (
            "main:Document.Заказ",
            "main:Document.Заказ.Module",
            2,
            vec![
                "main:Document.Заказ.Module.Object",
                "main:Document.Заказ.Module.Manager",
            ],
        ),
        (
            "main:Document.Заказ.Form.ФормаДокумента",
            "main:Document.Заказ.Form.ФормаДокумента.Module",
            1,
            vec!["main:Document.Заказ.Form.ФормаДокумента.Module.Form"],
        ),
    ];

    for (parent, branch, count, expected) in cases {
        let parent_result = service.view(ViewRequest::new(parent).unwrap());
        assert!(parent_result.ok, "{parent}: {parent_result:?}");
        let parent_data = parent_result.data.unwrap();
        let parent_branches = parent_data["branches"].as_array().unwrap();
        assert_eq!(
            parent_branches
                .iter()
                .map(|item| item["at"].as_str().unwrap())
                .collect::<std::collections::HashSet<_>>()
                .len(),
            parent_branches.len(),
            "{parent} exposed duplicate branch addresses: {parent_data}",
        );
        assert!(
            parent_branches.iter().any(|item| {
                item["at"] == branch && item["count"].as_u64() == Some(count as u64)
            }),
            "{parent} did not expose {branch}: {parent_data}"
        );

        let branch_result = service.view(ViewRequest::new(branch).unwrap());
        assert!(branch_result.ok, "{branch}: {branch_result:?}");
        let branch_data = branch_result.data.unwrap();
        let items = branch_data["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["at"].as_str().unwrap().to_string())
            .collect::<Vec<_>>();
        assert_eq!(items, expected, "{branch}");
        assert_eq!(items.len(), count, "{branch}");
    }
}

#[test]
fn configuration_runtime_modules_are_read_from_the_shared_ext_layout() {
    let fixture = RealReaderFixture::new();
    let cases = [
        (
            "ManagedApplication",
            "ManagedApplicationModule",
            "ManagedFromExt",
        ),
        (
            "OrdinaryApplication",
            "OrdinaryApplicationModule",
            "OrdinaryFromExt",
        ),
        ("Session", "SessionModule", "SessionFromExt"),
        (
            "ExternalConnection",
            "ExternalConnectionModule",
            "ExternalFromExt",
        ),
    ];
    for (_, file, method) in cases {
        write(
            &fixture.source.join(format!("Ext/{file}.bsl")),
            &format!("Procedure {method}()\nEndProcedure\n"),
        );
        write(
            &fixture.source.join(format!("{file}.bsl")),
            "Procedure WrongRootLayout()\nEndProcedure\n",
        );
    }
    let service = fixture.view_service();

    for (role, _, method) in cases {
        let at = format!("main:Module.{role}.Method.{method}");
        let result = service.view(ViewRequest::new(&at).unwrap());
        assert!(
            result.ok,
            "{at}: {} {:?}",
            result.summary, result.diagnostics
        );
        assert_eq!(result.data.as_ref().unwrap()["at"], at);
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AcceptedAddressProfile {
    valid_addresses: Vec<AcceptedAddressCase>,
}

#[derive(Debug, Deserialize)]
struct AcceptedAddressCase {
    case: String,
    input: String,
}

#[test]
fn every_accepted_profile_address_has_a_real_non_skipping_view() {
    let fixture = RealReaderFixture::new();
    fixture.install_accepted_profile_sources();
    let service = fixture.view_service();
    let profile: AcceptedAddressProfile = serde_json::from_str(include_str!(
        "../../../../../tests/fixtures/v013/address-profile-8.3.27.json"
    ))
    .unwrap();

    for case in profile.valid_addresses {
        let expected_at = QualifiedAddress::parse(&case.input).unwrap().to_string();
        let result = service.view(ViewRequest::new(&case.input).unwrap());
        assert!(
            result.ok,
            "{} ({expected_at}): {} {:?}",
            case.case, result.summary, result.diagnostics
        );
        assert_eq!(
            result.data.as_ref().unwrap()["at"],
            expected_at,
            "{}",
            case.case
        );
    }
}

#[test]
fn capability_bound_configuration_reader_preserves_complete_cf_info_semantics() {
    let fixture = RealReaderFixture::new();
    let cancellation = CancellationToken::new();
    let root = Arc::new(RetainedDirectoryCapability::open(&fixture.source).unwrap());
    let revisions = Arc::new(
        SourceRevisionService::new_reconciling_for_test(&fixture.context, &fixture.source).unwrap(),
    );
    let reader = ProviderReadAuthority::new(
        "main",
        "actor-fixture-main",
        SourceSetKind::Configuration,
        root,
        revisions,
    );
    reader
        .exact_revision(
            crate::domain::code_intelligence::ProviderDeadline::from_budget(
                std::time::Duration::from_secs(7),
            ),
            &cancellation,
        )
        .unwrap();

    let payload = configuration_payload(&reader).unwrap();
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
        &cancellation,
        "main",
        "actor-fixture-main",
        SourceSetKind::Configuration,
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
    assert_eq!(
        data["at"],
        "main:Role.SalesReader.Right.Catalog_Products.RLS.View"
    );
    assert_eq!(data["kind"], "RLS");
    assert_eq!(data["title"], "View");
}

#[test]
fn role_merges_access_by_canonical_object_and_keeps_rls_under_that_right() {
    let fixture = RealReaderFixture::new();
    let service = fixture.view_service();

    let root = service.view(ViewRequest::new("main:Role.SalesReader").unwrap());
    assert!(root.ok, "{root:?}");
    let root_data = root.data.unwrap();
    assert_eq!(
        root_data["branches"],
        json!([{"at": "main:Role.SalesReader.Right", "count": 2}]),
    );

    let rights = service.view(ViewRequest::new("main:Role.SalesReader.Right").unwrap());
    assert!(rights.ok, "{rights:?}");
    let items = rights.data.unwrap()["items"].as_array().unwrap().clone();
    assert_eq!(items.len(), 2);
    assert_eq!(
        items
            .iter()
            .map(|item| item["at"].as_str().unwrap())
            .collect::<std::collections::HashSet<_>>()
            .len(),
        2,
    );

    let alias = service.view(ViewRequest::new("main:Role.SalesReader.Right.Products").unwrap());
    assert!(alias.ok, "{alias:?}");
    let alias_data = alias.data.unwrap();
    assert_eq!(
        alias_data["at"],
        "main:Role.SalesReader.Right.Catalog_Products"
    );
    assert_eq!(alias_data["props"]["allowedCount"], 2);
    assert_eq!(alias_data["props"]["deniedCount"], 1);
    assert_eq!(
        alias_data["branches"],
        json!([{
            "at": "main:Role.SalesReader.Right.Catalog_Products.RLS",
            "count": 1,
        }]),
    );

    let rls_branch =
        service.view(ViewRequest::new("main:Role.SalesReader.Right.Catalog_Products.RLS").unwrap());
    assert!(rls_branch.ok, "{rls_branch:?}");
    let rls_items = rls_branch.data.unwrap()["items"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(rls_items.len(), 1);
    assert_eq!(
        rls_items
            .iter()
            .map(|item| item["at"].as_str().unwrap())
            .collect::<std::collections::HashSet<_>>()
            .len(),
        1,
    );

    let rls = service
        .view(ViewRequest::new("main:Role.SalesReader.Right.Catalog_Products.RLS.View").unwrap());
    assert!(rls.ok, "{rls:?}");
    assert_eq!(
        rls.data.unwrap()["at"],
        "main:Role.SalesReader.Right.Catalog_Products.RLS.View"
    );
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
fn pure_event_source_resolver_returns_relative_platform_and_nested_form_identities() {
    use crate::infrastructure::event_projection::resolve_property_event_source;
    use crate::infrastructure::logical_event_source::{
        resolve_event_source, LogicalEventSource, PropertyEventOwnerKind,
    };

    let platform = resolve_event_source(
        "main",
        SourceSetKind::Configuration,
        PlatformProfile::v8_3_27(),
        &QualifiedAddress::parse("main:Document.Заказ.Module.Object.Event.BeforeWrite").unwrap(),
    )
    .unwrap();
    let LogicalEventSource::Platform(platform) = platform else {
        panic!("platform event must resolve to a platform module")
    };
    assert_eq!(
        platform.module_at.to_string(),
        "main:Document.Заказ.Module.Object"
    );
    assert_eq!(
        platform.module_target.as_str(),
        "Document.Заказ.ObjectModule"
    );
    assert_eq!(
        platform.module_relative,
        PathBuf::from("Documents/Заказ/Ext/ObjectModule.bsl")
    );
    assert_eq!(
        platform.descriptor_requirements,
        vec![PathBuf::from("Documents/Заказ.xml")]
    );

    let fixture = RealReaderFixture::new();
    let property_at = QualifiedAddress::parse(
        "main:Report.ParityReport.Form.MainForm.Item.Goods.Item.Quantity.Event.OnChange",
    )
    .unwrap();
    let form_xml = fs::read_to_string(
        fixture
            .source
            .join("Reports/ParityReport/Forms/MainForm/Ext/Form.xml"),
    )
    .unwrap();
    let property = resolve_property_event_source(
        "main",
        SourceSetKind::Configuration,
        PlatformProfile::v8_3_27(),
        &property_at,
        &form_xml,
    )
    .unwrap();
    assert_eq!(
        property.form_xml_relative,
        PathBuf::from("Reports/ParityReport/Forms/MainForm/Ext/Form.xml")
    );
    assert_eq!(
        property.module_relative,
        PathBuf::from("Reports/ParityReport/Forms/MainForm/Ext/Form/Module.bsl")
    );
    assert_eq!(
        property
            .owner_chain
            .iter()
            .map(|owner| owner.kind)
            .collect::<Vec<_>>(),
        vec![
            PropertyEventOwnerKind::Form,
            PropertyEventOwnerKind::Table,
            PropertyEventOwnerKind::Column,
        ]
    );
    assert_eq!(
        property.descriptor_requirements,
        vec![
            PathBuf::from("Reports/ParityReport.xml"),
            PathBuf::from("Reports/ParityReport/Forms/MainForm.xml"),
        ]
    );

    let external = resolve_event_source(
        "epf",
        SourceSetKind::ExternalProcessor,
        PlatformProfile::v8_3_27(),
        &QualifiedAddress::parse(
            "epf:ExternalDataProcessor.Import.Module.Object.Event.BeforeWrite",
        )
        .unwrap(),
    )
    .unwrap();
    let LogicalEventSource::Platform(external) = external else {
        panic!("external object event must resolve to its object module")
    };
    assert_eq!(
        external.module_relative,
        PathBuf::from("Import/Ext/ObjectModule.bsl")
    );
    assert_eq!(
        external.descriptor_requirements,
        vec![PathBuf::from("Import.xml")]
    );
}

#[test]
fn pure_event_source_resolver_requires_form_evidence_for_every_item_depth() {
    use crate::infrastructure::logical_event_source::resolve_event_source;

    for raw in [
        "main:Report.ParityReport.Form.MainForm.Item.Field.Event.OnChange",
        "main:Report.ParityReport.Form.MainForm.Item.Rows.Item.Column.Event.OnChange",
        "main:Report.ParityReport.Form.MainForm.Item.Tabs.Item.Page.Item.Field.Event.OnChange",
    ] {
        let error = resolve_event_source(
            "main",
            SourceSetKind::Configuration,
            PlatformProfile::v8_3_27(),
            &QualifiedAddress::parse(raw).unwrap(),
        )
        .unwrap_err();
        assert_eq!(error.code(), "provider_unavailable", "{raw}: {error}");
    }
}

#[test]
fn form_evidence_resolver_returns_complete_typed_owner_chains_at_arbitrary_depth() {
    use crate::infrastructure::event_projection::{
        project_property_event, resolve_property_event_source,
    };
    use crate::infrastructure::logical_event_source::PropertyEventOwnerKind;
    use crate::infrastructure::v13_read::event_node_value;

    let fixture = RealReaderFixture::new();
    fixture.install_main_form_sources(
        r#"<?xml version="1.0" encoding="utf-8"?>
<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20">
  <ChildItems>
    <InputField name="Field"><Events><Event name="OnChange">FieldChanged</Event></Events></InputField>
    <Table name="Rows"><Events><Event name="Selection">RowsSelected</Event></Events><ChildItems>
      <InputField name="Column"><Events><Event name="OnChange">ColumnChanged</Event></Events></InputField>
    </ChildItems></Table>
    <UsualGroup name="Group"><ChildItems>
      <InputField name="GroupedField"><Events><Event name="OnChange">GroupedFieldChanged</Event></Events></InputField>
    </ChildItems></UsualGroup>
    <Pages name="Tabs"><ChildItems><Page name="FirstPage"><ChildItems>
      <InputField name="PageField"><Events><Event name="OnChange">PageFieldChanged</Event></Events></InputField>
    </ChildItems></Page></ChildItems></Pages>
  </ChildItems>
</Form>"#,
        "",
    );
    let form_xml = fs::read_to_string(
        fixture
            .source
            .join("Reports/ParityReport/Forms/MainForm/Ext/Form.xml"),
    )
    .unwrap();
    let cases = [
        (
            "main:Report.ParityReport.Form.MainForm.Item.Field.Event.OnChange",
            vec![
                PropertyEventOwnerKind::Form,
                PropertyEventOwnerKind::Element,
            ],
            vec![
                "main:Report.ParityReport.Form.MainForm",
                "main:Report.ParityReport.Form.MainForm.Item.Field",
            ],
        ),
        (
            "main:Report.ParityReport.Form.MainForm.Item.Rows.Event.Selection",
            vec![
                PropertyEventOwnerKind::Form,
                PropertyEventOwnerKind::Table,
            ],
            vec![
                "main:Report.ParityReport.Form.MainForm",
                "main:Report.ParityReport.Form.MainForm.Item.Rows",
            ],
        ),
        (
            "main:Report.ParityReport.Form.MainForm.Item.Rows.Item.Column.Event.OnChange",
            vec![
                PropertyEventOwnerKind::Form,
                PropertyEventOwnerKind::Table,
                PropertyEventOwnerKind::Column,
            ],
            vec![
                "main:Report.ParityReport.Form.MainForm",
                "main:Report.ParityReport.Form.MainForm.Item.Rows",
                "main:Report.ParityReport.Form.MainForm.Item.Rows.Item.Column",
            ],
        ),
        (
            "main:Report.ParityReport.Form.MainForm.Item.Group.Item.GroupedField.Event.OnChange",
            vec![
                PropertyEventOwnerKind::Form,
                PropertyEventOwnerKind::Element,
                PropertyEventOwnerKind::Element,
            ],
            vec![
                "main:Report.ParityReport.Form.MainForm",
                "main:Report.ParityReport.Form.MainForm.Item.Group",
                "main:Report.ParityReport.Form.MainForm.Item.Group.Item.GroupedField",
            ],
        ),
        (
            "main:Report.ParityReport.Form.MainForm.Item.Tabs.Item.FirstPage.Item.PageField.Event.OnChange",
            vec![
                PropertyEventOwnerKind::Form,
                PropertyEventOwnerKind::Element,
                PropertyEventOwnerKind::Element,
                PropertyEventOwnerKind::Element,
            ],
            vec![
                "main:Report.ParityReport.Form.MainForm",
                "main:Report.ParityReport.Form.MainForm.Item.Tabs",
                "main:Report.ParityReport.Form.MainForm.Item.Tabs.Item.FirstPage",
                "main:Report.ParityReport.Form.MainForm.Item.Tabs.Item.FirstPage.Item.PageField",
            ],
        ),
    ];
    let service = fixture.view_service();
    for (raw, expected_kinds, expected_addresses) in cases {
        let at = QualifiedAddress::parse(raw).unwrap();
        let source = resolve_property_event_source(
            "main",
            SourceSetKind::Configuration,
            PlatformProfile::v8_3_27(),
            &at,
            &form_xml,
        )
        .unwrap();
        assert_eq!(
            source
                .owner_chain
                .iter()
                .map(|owner| owner.kind)
                .collect::<Vec<_>>(),
            expected_kinds,
            "{raw}"
        );
        assert_eq!(
            source
                .owner_chain
                .iter()
                .map(|owner| owner.at.to_string())
                .collect::<Vec<_>>(),
            expected_addresses,
            "{raw}"
        );
        let pure = project_property_event(&source, &form_xml, Some("")).unwrap();
        let physical = service.view(ViewRequest::new(raw).unwrap());
        assert!(
            physical.ok,
            "{raw}: {} {:?}",
            physical.summary, physical.diagnostics
        );
        assert_eq!(physical.data.as_ref().unwrap()["props"]["state"], "missing");
        assert_eq!(physical.data.unwrap(), event_node_value(&pure));
    }
}

#[test]
fn pure_event_source_resolver_fails_closed_for_unproved_event_layouts() {
    use crate::infrastructure::logical_event_source::resolve_event_source;

    for (kind, raw) in [
        (
            SourceSetKind::Extension,
            "main:Document.Заказ.Module.Object.Event.BeforeWrite",
        ),
        (
            SourceSetKind::Configuration,
            "main:WebSocketClient.Телефония.Module.WebSocketClient.Event.OnMessage",
        ),
        (
            SourceSetKind::Configuration,
            "main:CommonModule.ЗаказыСервер.Event.BeforeStart",
        ),
        (
            SourceSetKind::Configuration,
            "main:EventSubscription.Изменение.Event.Change",
        ),
        (
            SourceSetKind::Configuration,
            "main:Report.ParityReport.Form.MainForm.Item.Rows.Event.Selection",
        ),
    ] {
        let error = resolve_event_source(
            "main",
            kind,
            PlatformProfile::v8_3_27(),
            &QualifiedAddress::parse(raw).unwrap(),
        )
        .unwrap_err();
        assert_eq!(error.code(), "provider_unavailable", "{raw}: {error}");
    }
}

#[test]
fn pure_event_projectors_match_physical_view_service_event_nodes() {
    use crate::infrastructure::event_projection::{
        project_platform_event, project_property_event, resolve_property_event_source,
    };
    use crate::infrastructure::logical_event_source::{resolve_event_source, LogicalEventSource};
    use crate::infrastructure::v13_read::event_node_value;

    let profile = PlatformProfile::v8_3_27();
    let platform_at =
        QualifiedAddress::parse("main:Document.Заказ.Module.Object.Event.BeforeWrite").unwrap();
    for source in [
        None,
        Some("Процедура ПередЗаписью(Отказ, РежимЗаписи, РежимПроведения)\nКонецПроцедуры"),
        Some("Функция ПередЗаписью(Отказ, РежимЗаписи, РежимПроведения)\nКонецФункции"),
    ] {
        let fixture = RealReaderFixture::new();
        fixture.install_accepted_profile_sources();
        if let Some(source) = source {
            write(
                &fixture.source.join("Documents/Заказ/Ext/ObjectModule.bsl"),
                source,
            );
        }
        let LogicalEventSource::Platform(resolved) =
            resolve_event_source("main", SourceSetKind::Configuration, profile, &platform_at)
                .unwrap()
        else {
            panic!("platform event must resolve to a module source")
        };
        let capability = profile.module_prefix_capability(&platform_at).unwrap();
        let pure = project_platform_event(&resolved, capability, source).unwrap();
        let physical = fixture
            .view_service()
            .view(ViewRequest::new(&platform_at.to_string()).unwrap());
        assert!(
            physical.ok,
            "{} {:?}",
            physical.summary, physical.diagnostics
        );
        assert_eq!(physical.data.unwrap(), event_node_value(&pure));
    }

    let fixture = RealReaderFixture::new();
    let property_at = QualifiedAddress::parse(
        "main:Report.ParityReport.Form.MainForm.Item.Goods.Item.Quantity.Event.OnChange",
    )
    .unwrap();
    let form_xml = fs::read_to_string(
        fixture
            .source
            .join("Reports/ParityReport/Forms/MainForm/Ext/Form.xml"),
    )
    .unwrap();
    let resolved = resolve_property_event_source(
        "main",
        SourceSetKind::Configuration,
        profile,
        &property_at,
        &form_xml,
    )
    .unwrap();
    let module_bsl = fs::read_to_string(fixture.source.join(&resolved.module_relative)).unwrap();
    let pure = project_property_event(&resolved, &form_xml, Some(&module_bsl)).unwrap();
    let physical = fixture
        .view_service()
        .view(ViewRequest::new(&property_at.to_string()).unwrap());
    assert!(
        physical.ok,
        "{} {:?}",
        physical.summary, physical.diagnostics
    );
    assert_eq!(physical.data.unwrap(), event_node_value(&pure));
}

#[test]
fn pure_form_projector_matches_every_physical_owner_and_all_event_states() {
    use crate::infrastructure::event_projection::{
        project_property_event, resolve_property_event_source,
    };
    use crate::infrastructure::v13_read::event_node_value;

    let fixture = RealReaderFixture::new();
    fixture.install_main_form_sources(
        r#"<?xml version="1.0" encoding="utf-8"?>
<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20">
  <ChildItems>
    <InputField name="Field"/>
    <Table name="Rows"><DataPath>Rows</DataPath>
      <Events><Event name="Selection">MissingSelection</Event></Events>
      <ChildItems><InputField name="Column"><Events><Event name="OnChange">WrongColumn</Event></Events></InputField></ChildItems>
    </Table>
  </ChildItems>
  <Events><Event name="OnOpen">FormOpen</Event></Events>
  <Commands><Command name="Run" id="1"><Action>RunAction</Action></Command></Commands>
</Form>"#,
        concat!(
            "&AtClient\nProcedure FormOpen(Cancel)\nEndProcedure\n\n",
            "Procedure WrongColumn()\nEndProcedure\n\n",
            "&AtClient\nProcedure RunAction(Command)\nEndProcedure\n",
        ),
    );
    let form_path = fixture
        .source
        .join("Reports/ParityReport/Forms/MainForm/Ext/Form.xml");
    let module_path = fixture
        .source
        .join("Reports/ParityReport/Forms/MainForm/Ext/Form/Module.bsl");
    let form_xml = fs::read_to_string(form_path).unwrap();
    let module_bsl = fs::read_to_string(module_path).unwrap();
    let service = fixture.view_service();
    for (event_at, state) in [
        (
            "main:Report.ParityReport.Form.MainForm.Event.OnOpen",
            "implemented",
        ),
        (
            "main:Report.ParityReport.Form.MainForm.Item.Field.Event.OnChange",
            "available",
        ),
        (
            "main:Report.ParityReport.Form.MainForm.Item.Rows.Event.Selection",
            "missing",
        ),
        (
            "main:Report.ParityReport.Form.MainForm.Item.Rows.Item.Column.Event.OnChange",
            "invalid",
        ),
        (
            "main:Report.ParityReport.Form.MainForm.Command.Run.Event.Execute",
            "implemented",
        ),
    ] {
        let at = QualifiedAddress::parse(event_at).unwrap();
        let source = resolve_property_event_source(
            "main",
            SourceSetKind::Configuration,
            PlatformProfile::v8_3_27(),
            &at,
            &form_xml,
        )
        .unwrap();
        let pure = project_property_event(&source, &form_xml, Some(&module_bsl)).unwrap();
        let physical = service.view(ViewRequest::new(event_at).unwrap());
        assert!(
            physical.ok,
            "{} {:?}",
            physical.summary, physical.diagnostics
        );
        assert_eq!(physical.data.as_ref().unwrap()["props"]["state"], state);
        assert_eq!(physical.data.unwrap(), event_node_value(&pure));
    }
}

#[test]
fn production_form_command_execute_has_one_semantic_identity_across_view_and_find() {
    let fixture = RealReaderFixture::new();
    fixture.install_main_form_sources(
        r#"<?xml version="1.0" encoding="utf-8"?>
<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20">
  <Commands>
    <Command name="Zero" id="1"/>
    <Command name="Missing" id="2"><Action>MissingAction</Action></Command>
    <Command name="Implemented" id="3"><Action>ImplementedAction</Action></Command>
    <Command name="Duplicate" id="4"><Action>FirstAction</Action><Action>SecondAction</Action></Command>
  </Commands>
</Form>"#,
        "&НаКлиенте\nПроцедура ImplementedAction(Команда)\nКонецПроцедуры\n",
    );
    let service = fixture.view_service();
    let form_at = "main:Report.ParityReport.Form.MainForm";
    let expected_contexts = json!([
        "thinClient",
        "webClient",
        "thickClientManaged",
        "mobileClient",
        "mobileAppClient"
    ]);
    for (command, state, handler, has_can) in [
        ("Zero", "available", Some("ОбработкаКоманды"), true),
        ("Missing", "missing", Some("MissingAction"), true),
        (
            "Implemented",
            "implemented",
            Some("ImplementedAction"),
            false,
        ),
        ("Duplicate", "invalid", None, false),
    ] {
        let command_at = format!("{form_at}.Command.{command}");
        let event_at = format!("{command_at}.Event.Execute");
        let collection = service.view(ViewRequest::new(&format!("{command_at}.Event")).unwrap());
        assert!(
            collection.ok,
            "{command}: {} {:?}",
            collection.summary, collection.diagnostics
        );
        let items = collection.data.as_ref().unwrap()["items"]
            .as_array()
            .unwrap();
        assert_eq!(items.len(), 1, "{command}: {items:?}");
        assert_eq!(items[0]["at"], event_at);
        assert_eq!(items[0]["props"]["eventId"], "Execute");
        assert_eq!(items[0]["props"]["state"], state);
        assert_eq!(items[0]["props"]["contexts"], expected_contexts);
        if let Some(handler) = handler {
            assert_eq!(items[0]["props"]["handler"], handler);
            let expected_signature = format!("Процедура {handler}(Команда)");
            assert_eq!(items[0]["props"]["signature"], expected_signature);
        }
        assert_eq!(
            items[0].get("can").is_some(),
            has_can,
            "{command}: {}",
            items[0]
        );
        if has_can {
            assert_eq!(items[0]["can"].as_array().unwrap().len(), 1);
            assert_eq!(items[0]["can"][0]["op"], "event.implement");
            assert_eq!(items[0]["can"][0]["args"], json!({"at": event_at}));
        }
        if state == "missing" {
            assert!(items[0]["props"]["implementationAt"].is_null());
        }
        if state == "implemented" {
            assert_eq!(
                items[0]["props"]["implementationAt"],
                format!("{form_at}.Module.Form.Method.ImplementedAction")
            );
        }

        let direct = service.view(ViewRequest::new(&event_at).unwrap());
        assert!(
            direct.ok,
            "{command}: {} {:?}",
            direct.summary, direct.diagnostics
        );
        assert_eq!(direct.data.as_ref().unwrap(), &items[0]);
    }

    let authority = fixture.operation_read_authority();
    let index = WorkspaceFindIndexBuilder::default()
        .build(
            &[ActorFindSource::new("main", &authority)],
            crate::domain::code_intelligence::ProviderDeadline::from_budget(
                crate::application::v13::LOGICAL_READ_OPERATION_BUDGET,
            ),
            &fixture.cancellation,
        )
        .unwrap();
    for command in ["Zero", "Missing", "Implemented", "Duplicate"] {
        let event_at = format!("{form_at}.Command.{command}.Event.Execute");
        let found = index.find(
            FindRequest::new(&event_at)
                .unwrap()
                .with_limit(100)
                .unwrap(),
        );
        assert!(!found.is_nearest(), "{event_at}: {found:?}");
        assert_eq!(
            found
                .candidates()
                .iter()
                .filter(|candidate| candidate.at() == event_at)
                .count(),
            1,
            "{event_at}: {found:?}"
        );
    }
}

#[test]
fn production_form_command_execute_requires_an_exact_client_directive() {
    let fixture = RealReaderFixture::new();
    let module_bsl = concat!(
        "&НаКлиенте\n",
        "Процедура RussianClientAction(Команда)\n",
        "КонецПроцедуры\n\n",
        "&AtClient\n",
        "Процедура EnglishClientAction(Команда)\n",
        "КонецПроцедуры\n\n",
        "&наклиенте\n",
        "Процедура RussianLowerAction(Команда)\n",
        "КонецПроцедуры\n\n",
        "&нАкЛиЕнТе\n",
        "Процедура RussianMixedAction(Команда)\n",
        "КонецПроцедуры\n\n",
        "&atclient\n",
        "Процедура EnglishLowerAction(Команда)\n",
        "КонецПроцедуры\n\n",
        "&aTcLiEnT\n",
        "Процедура EnglishMixedAction(Команда)\n",
        "КонецПроцедуры\n\n",
        "  &AtClient  \r\n",
        "Процедура WhitespaceCrLfAction(Команда)\r\n",
        "КонецПроцедуры\r\n\r\n",
        "Процедура MissingDirectiveAction(Команда)\n",
        "КонецПроцедуры\n\n",
        "&наклиентенасервере\n",
        "Процедура RussianClientServerLowerAction(Команда)\n",
        "КонецПроцедуры\n\n",
        "&aTcLiEnTaTsErVeR\n",
        "Процедура EnglishClientServerMixedAction(Команда)\n",
        "КонецПроцедуры\n\n",
        "#Если Клиент Тогда\n",
        "Процедура GuardOnlyAction(Команда)\n",
        "КонецПроцедуры\n",
        "#КонецЕсли\n",
    );
    fixture.install_main_form_sources(
        r#"<?xml version="1.0" encoding="utf-8"?>
<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20">
  <Commands>
    <Command name="RussianClient" id="1"><Action>RussianClientAction</Action></Command>
    <Command name="EnglishClient" id="2"><Action>EnglishClientAction</Action></Command>
    <Command name="RussianLower" id="3"><Action>RussianLowerAction</Action></Command>
    <Command name="RussianMixed" id="4"><Action>RussianMixedAction</Action></Command>
    <Command name="EnglishLower" id="5"><Action>EnglishLowerAction</Action></Command>
    <Command name="EnglishMixed" id="6"><Action>EnglishMixedAction</Action></Command>
    <Command name="WhitespaceCrLf" id="7"><Action>WhitespaceCrLfAction</Action></Command>
    <Command name="MissingDirective" id="8"><Action>MissingDirectiveAction</Action></Command>
    <Command name="RussianClientServerLower" id="9"><Action>RussianClientServerLowerAction</Action></Command>
    <Command name="EnglishClientServerMixed" id="10"><Action>EnglishClientServerMixedAction</Action></Command>
    <Command name="GuardOnly" id="11"><Action>GuardOnlyAction</Action></Command>
  </Commands>
</Form>"#,
        module_bsl,
    );
    let service = fixture.view_service();
    let form_at = "main:Report.ParityReport.Form.MainForm";
    for (command, handler, state) in [
        ("RussianClient", "RussianClientAction", "implemented"),
        ("EnglishClient", "EnglishClientAction", "implemented"),
        ("RussianLower", "RussianLowerAction", "implemented"),
        ("RussianMixed", "RussianMixedAction", "implemented"),
        ("EnglishLower", "EnglishLowerAction", "implemented"),
        ("EnglishMixed", "EnglishMixedAction", "implemented"),
        ("WhitespaceCrLf", "WhitespaceCrLfAction", "implemented"),
        ("MissingDirective", "MissingDirectiveAction", "invalid"),
        (
            "RussianClientServerLower",
            "RussianClientServerLowerAction",
            "invalid",
        ),
        (
            "EnglishClientServerMixed",
            "EnglishClientServerMixedAction",
            "invalid",
        ),
        ("GuardOnly", "GuardOnlyAction", "invalid"),
    ] {
        let command_at = format!("{form_at}.Command.{command}");
        let event_at = format!("{command_at}.Event.Execute");
        let collection = service.view(ViewRequest::new(&format!("{command_at}.Event")).unwrap());
        assert!(
            collection.ok,
            "{command}: {} {:?}",
            collection.summary, collection.diagnostics
        );
        let items = collection.data.as_ref().unwrap()["items"]
            .as_array()
            .unwrap();
        assert_eq!(items.len(), 1, "{command}: {items:?}");
        assert_eq!(items[0]["at"], event_at);
        assert_eq!(items[0]["props"]["eventId"], "Execute");
        assert_eq!(items[0]["props"]["state"], state, "{command}: {}", items[0]);
        assert_eq!(
            items[0]["props"]["implementationAt"],
            format!("{form_at}.Module.Form.Method.{handler}")
        );
        assert!(items[0].get("can").is_none(), "{command}: {}", items[0]);

        let direct = service.view(ViewRequest::new(&event_at).unwrap());
        assert!(
            direct.ok,
            "{command}: {} {:?}",
            direct.summary, direct.diagnostics
        );
        assert_eq!(direct.data.as_ref().unwrap(), &items[0]);
    }

    let authority = fixture.operation_read_authority();
    let index = WorkspaceFindIndexBuilder::default()
        .build(
            &[ActorFindSource::new("main", &authority)],
            crate::domain::code_intelligence::ProviderDeadline::from_budget(
                crate::application::v13::LOGICAL_READ_OPERATION_BUDGET,
            ),
            &fixture.cancellation,
        )
        .unwrap();
    for command in [
        "RussianClient",
        "EnglishClient",
        "RussianLower",
        "RussianMixed",
        "EnglishLower",
        "EnglishMixed",
        "WhitespaceCrLf",
        "MissingDirective",
        "RussianClientServerLower",
        "EnglishClientServerMixed",
        "GuardOnly",
    ] {
        let event_at = format!("{form_at}.Command.{command}.Event.Execute");
        let found = index.find(
            FindRequest::new(&event_at)
                .unwrap()
                .with_limit(100)
                .unwrap(),
        );
        assert!(!found.is_nearest(), "{event_at}: {found:?}");
        assert_eq!(
            found
                .candidates()
                .iter()
                .filter(|candidate| candidate.at() == event_at)
                .count(),
            1,
            "{event_at}: {found:?}"
        );
    }
}

#[test]
fn production_borrowed_command_defaults_after_and_base_form_only_has_no_can() {
    let borrowed = RealReaderFixture::new();
    borrowed.install_main_form_sources(
        r#"<?xml version="1.0" encoding="utf-8"?>
<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20">
  <Commands><Command name="Borrowed" id="1"/></Commands>
  <BaseForm version="2.20"/>
</Form>"#,
        "",
    );
    let event_at = "main:Report.ParityReport.Form.MainForm.Command.Borrowed.Event.Execute";
    let event = borrowed
        .view_service()
        .view(ViewRequest::new(event_at).unwrap());
    assert!(event.ok, "{} {:?}", event.summary, event.diagnostics);
    let event = event.data.as_ref().unwrap();
    assert_eq!(event["props"]["state"], "available");
    assert_eq!(event["can"][0]["op"], "event.implement");
    assert_eq!(
        event["can"][0]["args"],
        json!({"at": event_at, "callType": "After"})
    );

    let base_only = RealReaderFixture::new();
    base_only.install_main_form_sources(
        r#"<?xml version="1.0" encoding="utf-8"?>
<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20">
  <BaseForm version="2.20"/>
</Form>"#,
        "",
    );
    let event = base_only
        .view_service()
        .view(ViewRequest::new("main:Report.ParityReport.Form.MainForm.Event.OnOpen").unwrap());
    assert!(event.ok, "{} {:?}", event.summary, event.diagnostics);
    let event = event.data.as_ref().unwrap();
    assert_eq!(event["props"]["state"], "available");
    assert!(event.get("can").is_none(), "{event}");
}

#[test]
fn production_form_empty_handlers_are_invalid_for_every_property_owner() {
    let regular = RealReaderFixture::new();
    regular.install_main_form_sources(
        r#"<?xml version="1.0" encoding="utf-8"?>
<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20">
  <ChildItems>
    <InputField name="Field"><Events><Event name="OnChange">   </Event></Events></InputField>
    <Table name="Rows">
      <DataPath>Rows</DataPath>
      <Events><Event name="Selection"></Event></Events>
      <ChildItems>
        <InputField name="Column"><Events><Event name="OnChange">
        </Event></Events></InputField>
      </ChildItems>
    </Table>
  </ChildItems>
  <Events><Event name="OnOpen"> </Event></Events>
  <Commands><Command name="Regular" id="1"><Action>   </Action></Command></Commands>
</Form>"#,
        "",
    );
    let service = regular.view_service();
    for event_at in [
        "main:Report.ParityReport.Form.MainForm.Event.OnOpen",
        "main:Report.ParityReport.Form.MainForm.Item.Field.Event.OnChange",
        "main:Report.ParityReport.Form.MainForm.Item.Rows.Event.Selection",
        "main:Report.ParityReport.Form.MainForm.Item.Rows.Item.Column.Event.OnChange",
        "main:Report.ParityReport.Form.MainForm.Command.Regular.Event.Execute",
    ] {
        let event = service.view(ViewRequest::new(event_at).unwrap());
        assert!(
            event.ok,
            "{event_at}: {} {:?}",
            event.summary, event.diagnostics
        );
        let event = event.data.as_ref().unwrap();
        assert_eq!(event["props"]["state"], "invalid", "{event_at}: {event}");
        assert!(event.get("can").is_none(), "{event_at}: {event}");
        assert!(
            event["props"]
                .as_object()
                .unwrap()
                .contains_key("implementationAt"),
            "{event_at}: {event}"
        );
    }

    let borrowed = RealReaderFixture::new();
    borrowed.install_main_form_sources(
        r#"<?xml version="1.0" encoding="utf-8"?>
<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20">
  <Commands><Command name="Borrowed" id="1"><Action callType="After">   </Action></Command></Commands>
  <BaseForm version="2.20"/>
</Form>"#,
        "",
    );
    let event_at = "main:Report.ParityReport.Form.MainForm.Command.Borrowed.Event.Execute";
    let event = borrowed
        .view_service()
        .view(ViewRequest::new(event_at).unwrap());
    assert!(event.ok, "{} {:?}", event.summary, event.diagnostics);
    let event = event.data.as_ref().unwrap();
    assert_eq!(event["props"]["state"], "invalid", "{event}");
    assert!(event.get("can").is_none(), "{event}");
    assert!(event["props"]
        .as_object()
        .unwrap()
        .contains_key("implementationAt"));
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
        &cancellation,
        "main",
        "actor-fixture-main",
        SourceSetKind::Configuration,
        revisions,
        source_root,
        PlatformProfile::v8_3_27(),
    );
    let service = ViewService::new(authority, ViewCursorStore::default());
    let cases = [
        ("configuration", "main:Configuration", "Configuration"),
        ("metadata", "main:Catalog.Items", "Catalog"),
        ("form", "main:Report.ParityReport.Form.MainForm", "Form"),
        (
            "role-rls",
            "main:Role.SalesReader.Right.Catalog_Products.RLS",
            "RLS",
        ),
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
    let configuration_path = fixture.source.join("Configuration.xml");
    let configuration = fs::read_to_string(&configuration_path).unwrap();
    fs::write(
        &configuration_path,
        configuration.replacen(
            "</ChildObjects>",
            "<WebSocketClient>Телефония</WebSocketClient></ChildObjects>",
            1,
        ),
    )
    .unwrap();
    let cancellation = CancellationToken::new();
    let source_root = Arc::new(RetainedDirectoryCapability::open(&fixture.source).unwrap());
    let revisions = Arc::new(
        SourceRevisionService::new_reconciling_for_test(&fixture.context, &fixture.source).unwrap(),
    );
    let authority = LogicalViewReadAuthority::new(
        &cancellation,
        "main",
        "actor-fixture-main",
        SourceSetKind::Configuration,
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
fn extension_platform_event_does_not_advertise_unproved_interception() {
    let fixture = RealReaderFixture::new();
    fixture.install_accepted_profile_sources();
    let cancellation = CancellationToken::new();
    let source_root = Arc::new(RetainedDirectoryCapability::open(&fixture.source).unwrap());
    let revisions = Arc::new(
        SourceRevisionService::new_reconciling_for_test(&fixture.context, &fixture.source).unwrap(),
    );
    let authority = LogicalViewReadAuthority::new(
        &cancellation,
        "main",
        "actor-fixture-extension",
        SourceSetKind::Extension,
        revisions,
        source_root,
        PlatformProfile::v8_3_27(),
    );

    let result = ViewService::new(authority, ViewCursorStore::default())
        .view(ViewRequest::new("main:Document.Заказ.Module.Object.Event.BeforeWrite").unwrap());

    assert!(result.ok, "{} {:?}", result.summary, result.diagnostics);
    let data = result.data.as_ref().unwrap();
    assert_eq!(data["props"]["state"], "available");
    assert!(data.get("can").is_none(), "{data}");
}

#[test]
fn extension_root_platform_modules_are_owned_by_the_extension_root() {
    let fixture = RealReaderFixture::new();
    write(
        &fixture.source.join("Ext/ManagedApplicationModule.bsl"),
        "Procedure ПередНачаломРаботыСистемы(Отказ)\nEndProcedure\n\nProcedure РасширениеПриСтарте()\nEndProcedure\n",
    );
    let cancellation = CancellationToken::new();
    let source_root = Arc::new(RetainedDirectoryCapability::open(&fixture.source).unwrap());
    let revisions = Arc::new(
        SourceRevisionService::new_reconciling_for_test(&fixture.context, &fixture.source).unwrap(),
    );
    let authority = LogicalViewReadAuthority::new(
        &cancellation,
        "main",
        "actor-fixture-extension-root",
        SourceSetKind::Extension,
        revisions,
        source_root,
        PlatformProfile::v8_3_27(),
    );

    let module = ViewService::new(authority, ViewCursorStore::default())
        .view(ViewRequest::new("main:Module.ManagedApplication").unwrap());
    assert!(module.ok, "{module:?}");

    let source_root = Arc::new(RetainedDirectoryCapability::open(&fixture.source).unwrap());
    let revisions = Arc::new(
        SourceRevisionService::new_reconciling_for_test(&fixture.context, &fixture.source).unwrap(),
    );
    let authority = LogicalViewReadAuthority::new(
        &cancellation,
        "main",
        "actor-fixture-extension-root-find",
        SourceSetKind::Extension,
        revisions,
        source_root,
        PlatformProfile::v8_3_27(),
    );
    let index = WorkspaceFindIndexBuilder::default()
        .build(
            &[ActorFindSource::new("main", &authority)],
            crate::domain::code_intelligence::ProviderDeadline::from_budget(
                crate::application::v13::LOGICAL_READ_OPERATION_BUDGET,
            ),
            &cancellation,
        )
        .unwrap();
    for expected in [
        "main:Module.ManagedApplication",
        "main:Module.ManagedApplication.Method.РасширениеПриСтарте",
    ] {
        let found = index.find(FindRequest::new(expected).unwrap());
        assert!(
            !found.is_nearest()
                && found
                    .candidates()
                    .iter()
                    .any(|candidate| candidate.at() == expected),
            "extension root module identity is missing from find: {expected}: {found:?}"
        );
    }
}

#[test]
fn logical_reader_parity_contract_is_complete() {
    crate::infrastructure::source_revision::tests::retained_revision_authority_contract_is_complete(
    );
    object_commands_are_registered_inline_without_descriptor_files();
    template_bodies_are_read_only_when_the_template_node_is_addressed();
    add_in_templates_stop_addressing_at_the_template_without_reading_the_payload();
    configuration_level_rights_are_readable_role_objects();
    actor_owned_reader_never_follows_a_source_set_remap_after_admission();
    actor_owned_configuration_support_and_home_page_sidecars_are_retained();
    actor_owned_typed_form_reader_never_follows_a_source_set_remap();
    actor_owned_module_reader_never_follows_a_source_set_remap();
    every_typed_reader_remains_on_the_admitted_root_after_source_set_remap();
    actor_supplied_extension_kind_preserves_extension_support_semantics();
    configuration_root_branch_counts_match_every_reachable_collection();
    module_capability_parents_expose_canonical_module_collections();
    configuration_runtime_modules_are_read_from_the_shared_ext_layout();
    every_accepted_profile_address_has_a_real_non_skipping_view();
    real_typed_readers_cover_every_task14_profile_without_skipping();
    every_reader_rejects_an_extra_unconsumed_address_tail();
    form_table_column_event_consumes_arbitrary_depth_and_preserves_owner_address();
    form_projection_uses_a_positive_nested_scalar_allowlist();
    role_merges_access_by_canonical_object_and_keeps_rls_under_that_right();
    role_right_projection_never_serializes_an_unbounded_rights_array_into_props();
    retained_home_page_distinguishes_missing_from_malformed_and_wrong_root();
    unsupported_view_filter_is_a_typed_bad_value_instead_of_a_noop();
    module_body_context_filter_excludes_at_client_source_from_server_slice();
    module_method_public_filter_returns_only_export_methods();
    typed_projection_never_leaks_provider_or_physical_slots();
    typed_projection_rejects_unknown_provider_payload_instead_of_dumping_it();
    missing_owner_module_branch_is_not_invented_but_registered_owner_without_bsl_is_kept();
    real_external_sources_are_traversable_without_configuration_xml_and_hide_root_runtime_modules();
    external_inventory_skips_runtime_sidecar_and_fails_closed_on_malformed_or_ambiguous_owner();
    retained_external_inventory_is_cancellable_and_has_an_aggregate_byte_bound();
    production_authorities_reach_all_profile_module_capabilities_from_real_parent_inventories();
    ambiguous_short_role_alias_is_rejected_and_canonical_aliases_work();
    review_role_canonical_encoding_cannot_collapse_distinct_kind_name_pairs();
    review_rejects_direct_typed_owner_absent_from_configuration_inventory();
    review_rejects_orphan_nested_module_owners_not_registered_by_parent();
    registered_physical_child_with_wrong_descriptor_fails_direct_and_parent_navigation();
    registered_top_level_owner_without_descriptor_fails_kind_branch_and_direct_view();
    orphan_and_missing_physical_children_fail_closed_across_reader_families();
    external_parent_childobjects_are_the_only_nested_owner_authority();
    unregistered_top_level_descriptors_cannot_enter_any_typed_reader_family();
    review_revision_authority_cannot_be_swapped_after_named_identity_validation();
    operation_lease_rejects_named_root_replacement_before_node_read();
    review_rejects_revision_change_during_post_fence_owner_proof();
    cursor_retry_rejects_revision_change_during_role_canonicalization();
    review_production_read_port_has_no_nocancel_inventory_entrypoint();
    review_role_rejects_non_platform_metadata_node_kinds();
    operation_lease_find_traversal_scans_once_then_confirms_once();
    websocket_client_source_view_is_an_explicit_provider_gap();
    extension_platform_event_does_not_advertise_unproved_interception();
    extension_root_platform_modules_are_owned_by_the_extension_root();
    crate::application::invocation::tests::assert_operation_budget_survives_handoff_and_completes_once(
        crate::application::v13::LOGICAL_READ_OPERATION_BUDGET,
    );
}

#[test]
fn find_walks_every_real_addressable_reader_family_without_parallel_xml_semantics() {
    let fixture = RealReaderFixture::new();
    fixture.install_accepted_profile_sources();
    let authority = fixture.operation_read_authority();
    let index = WorkspaceFindIndexBuilder::default()
        .build(
            &[ActorFindSource::new("main", &authority)],
            crate::domain::code_intelligence::ProviderDeadline::from_budget(
                crate::application::v13::LOGICAL_READ_OPERATION_BUDGET,
            ),
            &fixture.cancellation,
        )
        .unwrap();
    let expected_addresses = [
        "main:Configuration",
        "main:Catalog",
        "main:Catalog.Items",
        "main:Catalog.Items.TabularSection.Lines.Attribute.Quantity",
        "main:Report.ParityReport.Form.MainForm",
        "main:Report.ParityReport.Template.MainSchema",
        "main:Report.ParityReport.Form.MainForm.Item.Goods.Item.Quantity.Event.OnChange",
        "main:Report.ParityReport.Form.MainForm.Event.OnOpen",
        "main:Role.SalesReader.Right.Catalog_Products.RLS.View",
        "main:Subsystem.Sales.Interface",
        "main:Report.ParityReport.Template.MainSchema.DataSet.MainData.Field.Code",
        "main:Report.ParityReport.Template.MainSchema.DataSet.MainData.Parameter.Period",
        "main:Report.ParityReport.Template.Print.Area.Header.Parameter.Title",
        "main:XDTOPackage.EnterpriseData_1_17_3.Type.Документ_ЗаказКлиента.Property.Идентификаторы",
        "main:CommonModule.РеактивныйСервер.Method.InternalService",
        "main:CommonModule.РеактивныйСервер.Region.ПрограммныйИнтерфейс",
        "main:Report.ParityReport.Form.MainForm.Module.Form.Method.OnOpen",
    ];
    for expected in expected_addresses {
        let found = index.find(FindRequest::new(expected).unwrap());
        assert!(
            !found.is_nearest()
                && found
                    .candidates()
                    .iter()
                    .any(|candidate| candidate.at() == expected),
            "missing real logical identity {expected}: {found:?}",
        );
    }
}

#[test]
pub(crate) fn operation_lease_find_traversal_scans_once_then_confirms_once() {
    let fixture = RealReaderFixture::new();
    fixture.install_accepted_profile_sources();
    let root = Arc::new(RetainedDirectoryCapability::open(&fixture.source).unwrap());
    let revisions = Arc::new(
        SourceRevisionService::new_reconciling_for_test(&fixture.context, &fixture.source).unwrap(),
    );
    let deadline = crate::domain::code_intelligence::ProviderDeadline::from_budget(
        crate::application::v13::LOGICAL_READ_OPERATION_BUDGET,
    );
    let lease = revisions
        .begin_retained_operation(&root, deadline, &fixture.cancellation)
        .unwrap();
    assert_eq!(revisions.retained_scan_count(), 1);
    let authority = LogicalViewReadAuthority::with_read_authority(
        &fixture.cancellation,
        ProviderReadAuthority::new_with_revision_lease(
            "main",
            "actor-fixture-main",
            SourceSetKind::Configuration,
            Arc::clone(&root),
            lease.clone(),
        ),
        PlatformProfile::v8_3_27(),
        deadline,
    );
    let built = WorkspaceFindIndexBuilder::default()
        .build_with_revision(
            &[ActorFindSource::new("main", &authority)],
            deadline,
            &fixture.cancellation,
        )
        .unwrap();
    for expected in [
        "main:Catalog.Items.TabularSection.Lines.Attribute.Quantity",
        "main:Report.ParityReport.Form.MainForm.Item.Goods.Item.Quantity.Event.OnChange",
        "main:Role.SalesReader.Right.Catalog_Products.RLS.View",
        "main:Report.ParityReport.Template.MainSchema.DataSet.MainData.Field.Code",
        "main:XDTOPackage.EnterpriseData_1_17_3.Type.Документ_ЗаказКлиента.Property.Идентификаторы",
        "main:CommonModule.РеактивныйСервер.Method.InternalService",
    ] {
        let found = built.index.find(FindRequest::new(expected).unwrap());
        assert!(
            !found.is_nearest()
                && found
                    .candidates()
                    .iter()
                    .any(|candidate| candidate.at() == expected),
            "the scan-count proof did not traverse {expected}: {found:?}"
        );
    }
    assert_eq!(
        revisions.retained_scan_count(),
        1,
        "node snapshots and reads must use the fixed operation revision"
    );
    revisions
        .confirm_retained_operation(&root, &lease, deadline, &fixture.cancellation)
        .unwrap();
    assert_eq!(
        revisions.retained_scan_count(),
        2,
        "the only second corpus scan is final operation confirmation"
    );
}

#[test]
fn operation_lease_rejects_named_root_replacement_before_node_read() {
    if !supports_retained_root_replacement_test() {
        return;
    }
    let fixture = RealReaderFixture::new();
    let root = Arc::new(RetainedDirectoryCapability::open(&fixture.source).unwrap());
    let revisions = Arc::new(
        SourceRevisionService::new_reconciling_for_test(&fixture.context, &fixture.source).unwrap(),
    );
    let deadline = crate::domain::code_intelligence::ProviderDeadline::from_budget(
        crate::application::v13::LOGICAL_READ_OPERATION_BUDGET,
    );
    let lease = revisions
        .begin_retained_operation(&root, deadline, &fixture.cancellation)
        .unwrap();
    let authority = LogicalViewReadAuthority::with_read_authority(
        &fixture.cancellation,
        ProviderReadAuthority::new_with_revision_lease(
            "main",
            "actor-fixture-main",
            SourceSetKind::Configuration,
            root,
            lease,
        ),
        PlatformProfile::v8_3_27(),
        deadline,
    );
    let saved = fixture.root.path().join("retained-a");
    let replacement = fixture.root.path().join("replacement-b");
    fs::create_dir_all(&replacement).unwrap();
    fs::write(
        replacement.join("Configuration.xml"),
        r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Replacement</Name></Properties><ChildObjects/></Configuration></MetaDataObject>"#,
    )
    .unwrap();
    fs::rename(&fixture.source, &saved).unwrap();
    fs::rename(&replacement, &fixture.source).unwrap();

    let result = ViewService::new(authority, ViewCursorStore::default())
        .view(ViewRequest::new("main:Configuration").unwrap());

    assert!(!result.ok, "a replaced capability name escaped: {result:?}");
    assert_eq!(result.diagnostics[0]["code"], "provider_unavailable");
}

#[test]
pub(crate) fn find_uses_each_typed_readers_real_export_path_without_publishing_it_in_view_props() {
    let fixture = RealReaderFixture::new();
    let authority = fixture.read_authority();
    let index = WorkspaceFindIndexBuilder::default()
        .build(
            &[ActorFindSource::new("main", &authority)],
            crate::domain::code_intelligence::ProviderDeadline::from_budget(
                crate::application::v13::LOGICAL_READ_OPERATION_BUDGET,
            ),
            &fixture.cancellation,
        )
        .unwrap();
    let cases = [
        (
            "Reports/ParityReport/Forms/MainForm.xml",
            "main:Report.ParityReport.Form.MainForm",
        ),
        (
            "Reports/ParityReport/Forms/MainForm/Ext/Form.xml",
            "main:Report.ParityReport.Form.MainForm.Item.Goods",
        ),
        (
            "Roles/SalesReader/Ext/Rights.xml",
            "main:Role.SalesReader.Right.Catalog_Products.RLS.View",
        ),
        (
            "Subsystems/Sales/Ext/CommandInterface.xml",
            "main:Subsystem.Sales.Interface",
        ),
        (
            "Reports/ParityReport/Templates/MainSchema/Ext/Template.xml",
            "main:Report.ParityReport.Template.MainSchema.DataSet.MainData.Field.Code",
        ),
        (
            "Reports/ParityReport/Templates/Print/Ext/Template.xml",
            "main:Report.ParityReport.Template.Print.Area.Header.Parameter.Title",
        ),
        (
            "XDTOPackages/EnterpriseData_1_17_3/Ext/Package.bin",
            "main:XDTOPackage.EnterpriseData_1_17_3.Type.Документ_ЗаказКлиента.Property.Идентификаторы",
        ),
        (
            "CommonModules/РеактивныйСервер/Ext/Module.bsl",
            "main:CommonModule.РеактивныйСервер.Method.InternalService",
        ),
    ];
    for (path, expected_at) in cases {
        let result = index.find(FindRequest::new(path).unwrap().with_limit(64).unwrap());
        assert!(
            !result.is_nearest()
                && result.candidates().iter().any(|candidate| {
                    candidate.at() == expected_at && candidate.reason() == "exportPath"
                }),
            "missing {expected_at} for {path}: {result:?}"
        );
    }
}

#[test]
fn missing_owner_module_branch_is_not_invented_but_registered_owner_without_bsl_is_kept() {
    let fixture = RealReaderFixture::new();
    let service = fixture.view_service();

    let missing = service.view(ViewRequest::new("main:Document.Missing.Module").unwrap());
    assert!(
        !missing.ok,
        "missing owner produced a module branch: {missing:?}"
    );
    assert_eq!(missing.diagnostics[0]["code"], "not_found");

    let registered = service.view(ViewRequest::new("main:Catalog.Items.Module").unwrap());
    assert!(registered.ok, "{:?}", registered.diagnostics);
    let items = registered.data.as_ref().unwrap()["items"]
        .as_array()
        .unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["at"], "main:Catalog.Items.Module.Object");
    assert_eq!(items[1]["at"], "main:Catalog.Items.Module.Manager");
}

#[test]
fn real_external_sources_are_traversable_without_configuration_xml_and_hide_root_runtime_modules() {
    let fixture = RealExternalReaderFixture::new();
    for (source, kind, owner) in [
        (
            "artifact_processor",
            SourceSetKind::ExternalProcessor,
            "ExternalDataProcessor.Import",
        ),
        (
            "artifact_report",
            SourceSetKind::ExternalReport,
            "ExternalReport.Sales",
        ),
    ] {
        let authority = fixture.read_authority(source, kind);
        let service = ViewService::new(authority, ViewCursorStore::default());
        let address = format!("{source}:Configuration");
        let root = service.view(ViewRequest::new(&address).unwrap());
        assert!(root.ok, "{source} root failed: {:?}", root.diagnostics);
        let root = root.data.unwrap();
        let owner_branch = root["branches"]
            .as_array()
            .unwrap()
            .iter()
            .find(|branch| branch["at"] == format!("{source}:{}", owner.split('.').next().unwrap()))
            .unwrap();
        assert_eq!(owner_branch["count"], 2);
        assert!(
            root["branches"]
                .as_array()
                .unwrap()
                .iter()
                .all(|branch| branch["at"] != format!("{source}:Module")),
            "external source root exposed configuration runtime modules: {root:?}",
        );
    }

    let epf = fixture.read_authority("artifact_processor", SourceSetKind::ExternalProcessor);
    let erf = fixture.read_authority("artifact_report", SourceSetKind::ExternalReport);
    let index = WorkspaceFindIndexBuilder::default()
        .build(
            &[
                ActorFindSource::new("artifact_processor", &epf),
                ActorFindSource::new("artifact_report", &erf),
            ],
            crate::domain::code_intelligence::ProviderDeadline::from_budget(
                crate::application::v13::LOGICAL_READ_OPERATION_BUDGET,
            ),
            &fixture.cancellation,
        )
        .unwrap();
    for expected in [
        "artifact_processor:ExternalDataProcessor.Import",
        "artifact_processor:ExternalDataProcessor.Import.Form.Main",
        "artifact_processor:ExternalDataProcessor.Import.Command.Run",
        "artifact_processor:ExternalDataProcessor.Import.Module.Object",
        "artifact_report:ExternalReport.Sales",
        "artifact_report:ExternalReport.Sales.Form.Main",
        "artifact_report:ExternalReport.Sales.Command.Run",
        "artifact_report:ExternalReport.Sales.Module.Object",
    ] {
        let found = index.find(FindRequest::new(expected).unwrap().with_limit(64).unwrap());
        assert!(
            !found.is_nearest()
                && found
                    .candidates()
                    .iter()
                    .any(|candidate| candidate.at() == expected),
            "missing real external identity {expected}: {found:?}",
        );
    }

    let fabricated_configuration_path = index.find(
        FindRequest::new("Configuration.xml")
            .unwrap()
            .with_limit(64)
            .unwrap(),
    );
    assert!(
        fabricated_configuration_path
            .candidates()
            .iter()
            .all(|candidate| candidate.reason() != "exportPath"),
        "external source sets must not advertise a configuration export path: \
         {fabricated_configuration_path:?}",
    );
}

#[test]
fn external_inventory_skips_runtime_sidecar_and_fails_closed_on_malformed_or_ambiguous_owner() {
    let malformed_fixture = RealExternalReaderFixture::new();
    fs::write(malformed_fixture.processor.join("Broken.xml"), "<broken>").unwrap();
    let malformed = ViewService::new(
        malformed_fixture.read_authority("artifact_processor", SourceSetKind::ExternalProcessor),
        ViewCursorStore::default(),
    )
    .view(ViewRequest::new("artifact_processor:Configuration").unwrap());
    assert!(!malformed.ok);
    assert_eq!(malformed.diagnostics[0]["code"], "provider_unavailable");

    let ambiguous_fixture = RealExternalReaderFixture::new();
    fs::copy(
        ambiguous_fixture.processor.join("Import.xml"),
        ambiguous_fixture.processor.join("Alias.xml"),
    )
    .unwrap();
    let ambiguous = ViewService::new(
        ambiguous_fixture.read_authority("artifact_processor", SourceSetKind::ExternalProcessor),
        ViewCursorStore::default(),
    )
    .view(ViewRequest::new("artifact_processor:Configuration").unwrap());
    assert!(!ambiguous.ok);
    assert_eq!(ambiguous.diagnostics[0]["code"], "provider_unavailable");
    assert!(ambiguous.diagnostics[0]["message"]
        .as_str()
        .unwrap()
        .contains("ambiguous"));
}

#[test]
fn retained_external_inventory_is_cancellable_and_has_an_aggregate_byte_bound() {
    let cancelled_fixture = RealExternalReaderFixture::new();
    let root = Arc::new(RetainedDirectoryCapability::open(&cancelled_fixture.processor).unwrap());
    let revisions = Arc::new(
        SourceRevisionService::new_reconciling_for_test(
            &cancelled_fixture.context,
            &cancelled_fixture.processor,
        )
        .unwrap(),
    );
    let reader = ProviderReadAuthority::new(
        "artifact_processor",
        "actor-external-cancel",
        SourceSetKind::ExternalProcessor,
        root,
        revisions,
    );
    let cancellation = CancellationToken::new();
    let mut checkpoints = 0_usize;
    let error = reader
        .configuration_payload_with_checkpoint(&mut || {
            checkpoints += 1;
            if checkpoints == 8 {
                cancellation.cancel();
            }
            if cancellation.is_cancelled() {
                Err(crate::application::v13::view::ViewError::new(
                    "cancelled",
                    "external inventory cancelled",
                ))
            } else {
                Ok(())
            }
        })
        .unwrap_err();
    assert_eq!(error.code(), "cancelled");

    let bounded_fixture = RealExternalReaderFixture::new();
    for index in 0..5 {
        let name = format!("Large{index}");
        write_external_artifact(&bounded_fixture.processor, "ExternalDataProcessor", &name);
        let path = bounded_fixture.processor.join(format!("{name}.xml"));
        let descriptor = fs::read_to_string(&path).unwrap();
        let padding = format!("<!--{}-->", "x".repeat(7 * 1024 * 1024));
        fs::write(
            &path,
            descriptor.replace("</MetaDataObject>", &format!("{padding}</MetaDataObject>")),
        )
        .unwrap();
    }
    let root = Arc::new(RetainedDirectoryCapability::open(&bounded_fixture.processor).unwrap());
    let revisions = Arc::new(
        SourceRevisionService::new_reconciling_for_test(
            &bounded_fixture.context,
            &bounded_fixture.processor,
        )
        .unwrap(),
    );
    let reader = ProviderReadAuthority::new(
        "artifact_processor",
        "actor-external-bounded",
        SourceSetKind::ExternalProcessor,
        root,
        revisions,
    );
    let error = configuration_payload(&reader).unwrap_err();
    assert_eq!(error.code(), "provider_unavailable");
    assert!(error.to_string().contains("read limit"));
}

#[test]
pub(crate) fn production_authorities_reach_all_profile_module_capabilities_from_real_parent_inventories(
) {
    let main_fixture = RealReaderFixture::new();
    main_fixture.install_module_matrix_sources();
    let external_fixture = RealExternalReaderFixture::new();
    let main_navigation = main_fixture.view_service();
    for address in [
        "main:WebSocketClient",
        "main:WebSocketClient.Телефония",
        "main:WebSocketClient.Телефония.Module",
    ] {
        let viewed = main_navigation.view(ViewRequest::new(address).unwrap());
        assert!(viewed.ok, "{address}: {:?}", viewed.diagnostics);
    }
    let main = main_fixture.operation_read_authority();
    let epf = external_fixture.operation_read_authority("epf", SourceSetKind::ExternalProcessor);
    let erf = external_fixture.operation_read_authority("erf", SourceSetKind::ExternalReport);
    let expected = serde_json::from_str::<ModuleCapabilityFixture>(include_str!(
        "../../../../../tests/fixtures/v013/address-profile-8.3.27.json"
    ))
    .unwrap()
    .module_capabilities
    .into_iter()
    .filter(|case| case.exists)
    .map(|case| case.at)
    .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(expected.len(), 25);

    let index = WorkspaceFindIndexBuilder::default()
        .build(
            &[
                ActorFindSource::new("main", &main),
                ActorFindSource::new("epf", &epf),
                ActorFindSource::new("erf", &erf),
            ],
            crate::domain::code_intelligence::ProviderDeadline::from_budget(
                crate::application::v13::LOGICAL_READ_OPERATION_BUDGET,
            ),
            &main_fixture.cancellation,
        )
        .unwrap();
    for expected_at in expected {
        let found = index.find(
            FindRequest::new(&expected_at)
                .unwrap()
                .with_limit(100)
                .unwrap(),
        );
        assert!(
            !found.is_nearest()
                && found
                    .candidates()
                    .iter()
                    .any(|candidate| candidate.at() == expected_at),
            "missing production module identity {expected_at}: {found:?}",
        );
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ModuleCapabilityFixture {
    module_capabilities: Vec<ModuleCapabilityCase>,
}

#[derive(Deserialize)]
struct ModuleCapabilityCase {
    at: String,
    exists: bool,
}

#[test]
pub(crate) fn one_find_reads_each_module_source_once_per_actor_revision() {
    let fixture = RealReaderFixture::new();
    let authority = fixture.read_authority();
    WorkspaceFindIndexBuilder::default()
        .build(
            &[ActorFindSource::new("main", &authority)],
            crate::domain::code_intelligence::ProviderDeadline::from_budget(
                crate::application::v13::LOGICAL_READ_OPERATION_BUDGET,
            ),
            &fixture.cancellation,
        )
        .unwrap();
    assert_eq!(
        authority.module_source_read_count("CommonModule.РеактивныйСервер.Module"),
        1,
        "one actor-owned revision must parse each module source once",
    );
    assert_eq!(
        authority.configuration_payload_read_count(),
        1,
        "one actor-owned revision must parse its source-set inventory once",
    );

    let module = fixture
        .source
        .join("CommonModules/РеактивныйСервер/Ext/Module.bsl");
    let mut changed = fs::read_to_string(&module).unwrap();
    changed.push_str("\nProcedure AfterRevisionChange()\nEndProcedure\n");
    fs::write(&module, changed).unwrap();
    WorkspaceFindIndexBuilder::default()
        .build(
            &[ActorFindSource::new("main", &authority)],
            crate::domain::code_intelligence::ProviderDeadline::from_budget(
                crate::application::v13::LOGICAL_READ_OPERATION_BUDGET,
            ),
            &fixture.cancellation,
        )
        .unwrap();
    assert_eq!(
        authority.module_source_read_count("CommonModule.РеактивныйСервер.Module"),
        2,
        "a changed exact revision must receive a new projection",
    );
    assert_eq!(authority.configuration_payload_read_count(), 2);

    let second_authority = fixture.read_authority();
    WorkspaceFindIndexBuilder::default()
        .build(
            &[ActorFindSource::new("main", &second_authority)],
            crate::domain::code_intelligence::ProviderDeadline::from_budget(
                crate::application::v13::LOGICAL_READ_OPERATION_BUDGET,
            ),
            &fixture.cancellation,
        )
        .unwrap();
    assert_eq!(
        second_authority.module_source_read_count("CommonModule.РеактивныйСервер.Module"),
        1,
        "a second actor must own an independent operation-local projection",
    );
    assert_eq!(second_authority.configuration_payload_read_count(), 1);
}

#[test]
pub(crate) fn one_find_parses_each_metadata_descriptor_once_per_actor_revision() {
    let fixture = RealReaderFixture::new();
    let authority = fixture.read_authority();
    let index = WorkspaceFindIndexBuilder::default()
        .build(
            &[ActorFindSource::new("main", &authority)],
            crate::domain::code_intelligence::ProviderDeadline::from_budget(
                crate::application::v13::LOGICAL_READ_OPERATION_BUDGET,
            ),
            &fixture.cancellation,
        )
        .unwrap();
    for expected in [
        "main:Catalog.Items",
        "main:Catalog.Items.TabularSection.Lines",
        "main:Catalog.Items.TabularSection.Lines.Attribute.Quantity",
    ] {
        let found = index.find(FindRequest::new(expected).unwrap());
        assert!(
            !found.is_nearest()
                && found
                    .candidates()
                    .iter()
                    .any(|candidate| candidate.at() == expected),
            "the descriptor-count proof did not traverse {expected}: {found:?}"
        );
    }
    // Owner proof reads the descriptor once and the typed projection once
    // more; every logical address projected from that owner shares the parse.
    assert_eq!(
        authority.metadata_descriptor_read_count("Catalog.Items"),
        2,
        "one actor-owned revision must parse each metadata descriptor once per authority",
    );
}

#[test]
fn ambiguous_short_role_alias_is_rejected_and_canonical_aliases_work() {
    let payload = json!({
        "name": "SalesReader",
        "allowed": [
            {"kind": "Catalog", "objects": [{"name": "Orders", "rights": []}]},
            {"kind": "Document", "objects": [{"name": "Orders", "rights": []}]}
        ],
        "denied": []
    });
    let short = QualifiedAddress::parse("main:Role.SalesReader.Right.Orders").unwrap();
    let route = route_logical_address(&short, PlatformProfile::v8_3_27()).unwrap();
    let error = project_typed_payload(&route, payload.clone()).unwrap_err();
    assert_eq!(error.code(), "bad_value");
    assert!(error.to_string().contains("Catalog_Orders"));
    assert!(error.to_string().contains("Document_Orders"));

    for canonical in ["Catalog_Orders", "Document_Orders"] {
        let at =
            QualifiedAddress::parse(&format!("main:Role.SalesReader.Right.{canonical}")).unwrap();
        let route = route_logical_address(&at, PlatformProfile::v8_3_27()).unwrap();
        let projected = project_typed_payload(&route, payload.clone()).unwrap();
        assert_eq!(
            projected.at(),
            format!("main:Role.SalesReader.Right.{canonical}")
        );
    }
}

struct RealExternalReaderFixture {
    _root: tempfile::TempDir,
    context: WorkspaceContext,
    processor: PathBuf,
    report: PathBuf,
    cancellation: CancellationToken,
}

impl RealExternalReaderFixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let cache = root.path().join("cache");
        let processor = root.path().join("processor");
        let report = root.path().join("report");
        fs::create_dir_all(&cache).unwrap();
        fs::create_dir_all(&processor).unwrap();
        fs::create_dir_all(&report).unwrap();
        write_external_artifact(&processor, "ExternalDataProcessor", "Import");
        write_external_artifact(&processor, "ExternalDataProcessor", "Импорт");
        write_external_artifact(&report, "ExternalReport", "Sales");
        write_external_artifact(&report, "ExternalReport", "Продажи");
        fs::copy(
            fixture_path("platform_8_3_27/staged_dump_roots/ConfigDumpInfo.xml"),
            processor.join("ConfigDumpInfo.xml"),
        )
        .unwrap();
        fs::copy(
            fixture_path("platform_8_3_27/staged_dump_roots/ConfigDumpInfo.xml"),
            report.join("ConfigDumpInfo.xml"),
        )
        .unwrap();
        fs::write(
            root.path().join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: artifact_processor\n    type: EXTERNAL_PROCESSOR\n    path: processor\n  - name: artifact_report\n    type: EXTERNAL_REPORT\n    path: report\n",
        )
        .unwrap();
        let canonical_root = fs::canonicalize(root.path()).unwrap();
        let processor = fs::canonicalize(processor).unwrap();
        let report = fs::canonicalize(report).unwrap();
        let context = WorkspaceContext {
            cwd: canonical_root.clone(),
            workspace_root: canonical_root,
            cache_root: cache,
            workspace_epoch: 1,
        };
        Self {
            _root: root,
            context,
            processor,
            report,
            cancellation: CancellationToken::new(),
        }
    }

    fn read_authority(
        &self,
        source_set: &str,
        kind: SourceSetKind,
    ) -> LogicalViewReadAuthority<'_> {
        let source = if kind == SourceSetKind::ExternalProcessor {
            &self.processor
        } else {
            &self.report
        };
        let root = Arc::new(RetainedDirectoryCapability::open(source).unwrap());
        let revisions = Arc::new(
            SourceRevisionService::new_reconciling_for_test(&self.context, source).unwrap(),
        );
        LogicalViewReadAuthority::new(
            &self.cancellation,
            source_set,
            format!("actor-{source_set}"),
            kind,
            revisions,
            root,
            PlatformProfile::v8_3_27(),
        )
    }

    fn operation_read_authority(
        &self,
        source_set: &str,
        kind: SourceSetKind,
    ) -> LogicalViewReadAuthority<'_> {
        let source = if kind == SourceSetKind::ExternalProcessor {
            &self.processor
        } else {
            &self.report
        };
        let root = Arc::new(RetainedDirectoryCapability::open(source).unwrap());
        let revisions = Arc::new(
            SourceRevisionService::new_reconciling_for_test(&self.context, source).unwrap(),
        );
        let deadline =
            ProviderDeadline::from_budget(crate::application::v13::LOGICAL_READ_OPERATION_BUDGET);
        let lease = revisions
            .begin_retained_operation(&root, deadline, &self.cancellation)
            .unwrap();
        LogicalViewReadAuthority::with_read_authority(
            &self.cancellation,
            ProviderReadAuthority::new_with_revision_lease(
                source_set,
                format!("actor-{source_set}"),
                kind,
                root,
                lease,
            ),
            PlatformProfile::v8_3_27(),
            deadline,
        )
    }
}

fn write_external_artifact(source: &Path, kind: &str, name: &str) {
    let (form_name, command_name) = match name {
        "Импорт" => ("Основная", "Выполнить"),
        "Продажи" => ("Основная", "Сформировать"),
        _ => ("Main", "Run"),
    };
    write(
        &source.join(format!("{name}.xml")),
        &format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20">
  <{kind} uuid="10000000-0000-4000-8000-000000000010">
    <Properties><Name>{name}</Name></Properties>
    <ChildObjects><Form>{form_name}</Form><Command uuid="10000000-0000-4000-8000-000000000012"><Properties><Name>{command_name}</Name></Properties></Command></ChildObjects>
  </{kind}>
</MetaDataObject>"#,
        ),
    );
    write(
        &source.join(format!("{name}/Forms/{form_name}.xml")),
        &format!(
            r#"<?xml version="1.0" encoding="UTF-8"?><MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Form uuid="10000000-0000-4000-8000-000000000011"><Properties><Name>{form_name}</Name><FormType>Managed</FormType></Properties></Form></MetaDataObject>"#
        ),
    );
    write(
        &source.join(format!("{name}/Forms/{form_name}/Ext/Form.xml")),
        r#"<?xml version="1.0" encoding="UTF-8"?><Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20"><ChildItems/></Form>"#,
    );
    write(
        &source.join(format!("{name}/Commands/{command_name}/Ext/CommandModule.bsl")),
        "&AtClient\nProcedure CommandProcessing(CommandParameter, CommandExecuteParameters)\nEndProcedure\n",
    );
    write(
        &source.join(format!("{name}/Ext/ObjectModule.bsl")),
        "Procedure Execute()\nEndProcedure\n",
    );
}

fn write_metadata_owner(
    source: &Path,
    directory: &str,
    kind: &str,
    name: &str,
    child_objects: &str,
) {
    write(
        &source.join(format!("{directory}/{name}.xml")),
        &format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20">
  <{kind} uuid="10000000-0000-4000-8000-000000000020">
    <Properties><Name>{name}</Name></Properties>
    <ChildObjects>{child_objects}</ChildObjects>
  </{kind}>
</MetaDataObject>"#,
        ),
    );
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
        write(
            &source.join("Catalogs/Items/Forms/ItemForm.xml"),
            r#"<?xml version="1.0" encoding="UTF-8"?><MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Form uuid="10000000-0000-4000-8000-000000000031"><Properties><Name>ItemForm</Name><FormType>Managed</FormType></Properties></Form></MetaDataObject>"#,
        );
        write(
            &source.join("Catalogs/Items/Forms/ItemForm/Ext/Form.xml"),
            r#"<?xml version="1.0" encoding="UTF-8"?><Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20"><ChildItems/></Form>"#,
        );
        write(
            &source.join("Catalogs/Items/Templates/Print.xml"),
            r#"<?xml version="1.0" encoding="UTF-8"?><MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Template uuid="10000000-0000-4000-8000-000000000032"><Properties><Name>Print</Name><TemplateType>SpreadsheetDocument</TemplateType></Properties></Template></MetaDataObject>"#,
        );
        copy_fixture(
            "platform_8_3_27/mxl/Template.xml",
            &source.join("Catalogs/Items/Templates/Print/Ext/Template.xml"),
        );
        write(
            &source.join("Catalogs/Items/Commands/Refresh/Ext/CommandModule.bsl"),
            "&AtClient\nProcedure CommandProcessing(CommandParameter, CommandExecuteParameters)\nEndProcedure\n",
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
            form,
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

    fn read_authority(&self) -> LogicalViewReadAuthority<'_> {
        let source_root = Arc::new(RetainedDirectoryCapability::open(&self.source).unwrap());
        let revisions = Arc::new(
            SourceRevisionService::new_reconciling_for_test(&self.context, &self.source).unwrap(),
        );
        LogicalViewReadAuthority::new(
            &self.cancellation,
            "main",
            "actor-fixture-main",
            SourceSetKind::Configuration,
            revisions,
            source_root,
            PlatformProfile::v8_3_27(),
        )
    }

    fn operation_read_authority(&self) -> LogicalViewReadAuthority<'_> {
        let root = Arc::new(RetainedDirectoryCapability::open(&self.source).unwrap());
        let revisions = Arc::new(
            SourceRevisionService::new_reconciling_for_test(&self.context, &self.source).unwrap(),
        );
        let deadline =
            ProviderDeadline::from_budget(crate::application::v13::LOGICAL_READ_OPERATION_BUDGET);
        let lease = revisions
            .begin_retained_operation(&root, deadline, &self.cancellation)
            .unwrap();
        LogicalViewReadAuthority::with_read_authority(
            &self.cancellation,
            ProviderReadAuthority::new_with_revision_lease(
                "main",
                "actor-fixture-main",
                SourceSetKind::Configuration,
                root,
                lease,
            ),
            PlatformProfile::v8_3_27(),
            deadline,
        )
    }

    fn view_service(&self) -> ViewService<LogicalViewReadAuthority<'_>> {
        ViewService::new(self.read_authority(), ViewCursorStore::default())
    }

    fn install_main_form_sources(&self, form_xml: &str, module_bsl: &str) {
        write(
            &self
                .source
                .join("Reports/ParityReport/Forms/MainForm/Ext/Form.xml"),
            form_xml,
        );
        write(
            &self
                .source
                .join("Reports/ParityReport/Forms/MainForm/Ext/Form/Module.bsl"),
            module_bsl,
        );
    }

    fn install_module_matrix_sources(&self) {
        let configuration_path = self.source.join("Configuration.xml");
        let configuration = fs::read_to_string(&configuration_path).unwrap();
        let registrations = [
            ("Document", "Заказ"),
            ("Document", "ЕщеНеВыгружен"),
            ("InformationRegister", "Цены"),
            ("Constant", "ОсновнаяВалюта"),
            ("CommonModule", "ЗаказыСервер"),
            ("CommonForm", "Подбор"),
            ("CommonCommand", "ОткрытьНастройки"),
            ("HTTPService", "API"),
            ("WebService", "Обмен"),
            ("IntegrationService", "Шина"),
            ("Bot", "Помощник"),
            ("WebSocketClient", "Телефония"),
        ]
        .into_iter()
        .map(|(kind, name)| format!("\n\t\t\t<{kind}>{name}</{kind}>"))
        .collect::<String>();
        fs::write(
            &configuration_path,
            configuration.replacen(
                "</ChildObjects>",
                &format!("{registrations}\n\t\t</ChildObjects>"),
                1,
            ),
        )
        .unwrap();

        write_metadata_owner(
            &self.source,
            "Documents",
            "Document",
            "Заказ",
            "<Form>ФормаДокумента</Form><Command uuid=\"10000000-0000-4000-8000-000000000022\"><Properties><Name>ПровестиИЗакрыть</Name></Properties></Command>",
        );
        write_metadata_owner(&self.source, "Documents", "Document", "ЕщеНеВыгружен", "");
        write_metadata_owner(
            &self.source,
            "InformationRegisters",
            "InformationRegister",
            "Цены",
            "",
        );
        write_metadata_owner(&self.source, "Constants", "Constant", "ОсновнаяВалюта", "");
        let common = fixture_text(
            "platform_8_3_27/support-edit-bin-only/src/CommonModules/РеактивныйСервер.xml",
        )
        .replace("РеактивныйСервер", "ЗаказыСервер");
        write(&self.source.join("CommonModules/ЗаказыСервер.xml"), &common);
        write_metadata_owner(&self.source, "CommonForms", "CommonForm", "Подбор", "");
        write(
            &self.source.join("CommonForms/Подбор/Ext/Form.xml"),
            r#"<?xml version="1.0" encoding="UTF-8"?><Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20"><ChildItems/></Form>"#,
        );
        write_metadata_owner(
            &self.source,
            "CommonCommands",
            "CommonCommand",
            "ОткрытьНастройки",
            "",
        );
        for (directory, kind, name) in [
            ("HTTPServices", "HTTPService", "API"),
            ("WebServices", "WebService", "Обмен"),
            ("IntegrationServices", "IntegrationService", "Шина"),
            ("Bots", "Bot", "Помощник"),
        ] {
            write_metadata_owner(&self.source, directory, kind, name, "");
        }
        write(
            &self.source.join("Documents/Заказ/Forms/ФормаДокумента.xml"),
            r#"<?xml version="1.0" encoding="UTF-8"?><MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Form uuid="10000000-0000-4000-8000-000000000021"><Properties><Name>ФормаДокумента</Name><FormType>Managed</FormType></Properties></Form></MetaDataObject>"#,
        );
        write(
            &self
                .source
                .join("Documents/Заказ/Forms/ФормаДокумента/Ext/Form.xml"),
            r#"<?xml version="1.0" encoding="UTF-8"?><Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20"><ChildItems/></Form>"#,
        );
        write(
            &self
                .source
                .join("Documents/Заказ/Commands/ПровестиИЗакрыть/Ext/CommandModule.bsl"),
            "&AtClient\nProcedure CommandProcessing(CommandParameter, CommandExecuteParameters)\nEndProcedure\n",
        );
    }

    fn install_accepted_profile_sources(&self) {
        let configuration_path = self.source.join("Configuration.xml");
        let configuration = fs::read_to_string(&configuration_path).unwrap();
        fs::write(
            configuration_path,
            configuration.replace(
                "</ChildObjects>",
                "\n<Document>Заказ</Document>\n<Role>Кладовщик</Role>\n<Report>Продажи</Report>\n</ChildObjects>",
            ),
        )
        .unwrap();

        write(
            &self.source.join("Documents/Заказ.xml"),
            r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:xs="http://www.w3.org/2001/XMLSchema" version="2.20">
  <Document uuid="10000000-0000-4000-8000-000000000001">
    <Properties><Name>Заказ</Name></Properties>
    <ChildObjects>
      <Attribute><Properties><Name>Контрагент</Name><Type><v8:Type>xs:string</v8:Type></Type></Properties></Attribute>
      <TabularSection><Properties><Name>Товары</Name></Properties><ChildObjects>
        <Attribute><Properties><Name>Количество</Name><Type><v8:Type>xs:decimal</v8:Type></Type></Properties></Attribute>
      </ChildObjects></TabularSection>
      <Form>ФормаДокумента</Form>
    </ChildObjects>
  </Document>
</MetaDataObject>"#,
        );
        write(
            &self.source.join("Documents/Заказ/Forms/ФормаДокумента.xml"),
            r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Form uuid="10000000-0000-4000-8000-000000000002"><Properties><Name>ФормаДокумента</Name><FormType>Managed</FormType></Properties></Form></MetaDataObject>"#,
        );
        write(
            &self
                .source
                .join("Documents/Заказ/Forms/ФормаДокумента/Ext/Form.xml"),
            r#"<?xml version="1.0" encoding="UTF-8"?>
<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:xs="http://www.w3.org/2001/XMLSchema" version="2.20">
  <ChildItems>
    <InputField name="Склад" id="1"/>
    <Table name="Товары" id="2"><ChildItems><InputField name="Количество" id="3"><Events><Event name="Change">КоличествоИзменение</Event></Events></InputField></ChildItems></Table>
  </ChildItems>
  <Attributes><Attribute name="Объект" id="1"><Type><v8:Type>xs:string</v8:Type></Type></Attribute></Attributes>
  <Parameters><Parameter name="Режим"><Type><v8:Type>xs:string</v8:Type></Type></Parameter></Parameters>
  <Commands><Command name="Пересчитать" id="1"/></Commands>
</Form>"#,
        );

        write(
            &self.source.join("Roles/Кладовщик.xml"),
            r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Role uuid="10000000-0000-4000-8000-000000000003"><Properties><Name>Кладовщик</Name></Properties></Role></MetaDataObject>"#,
        );
        write(
            &self.source.join("Roles/Кладовщик/Ext/Rights.xml"),
            r#"<?xml version="1.0" encoding="UTF-8"?>
<Rights xmlns="http://v8.1c.ru/8.2/roles" version="2.20"><object><name>Catalog.Товары</name><right><name>Read</name><value>true</value><restrictionByCondition><condition>Товары.Владелец = &amp;ТекущийПользователь</condition></restrictionByCondition></right></object></Rights>"#,
        );

        let report = fixture_text("unica_mcp_script_parity/form-remove/ParityReport.xml")
            .replace("ParityReport", "Продажи");
        write(
            &self.source.join("Reports/Продажи.xml"),
            &replace_child_objects(
                &report,
                "<Template>ОсновнаяСхема</Template><Template>Печать</Template>",
            ),
        );
        let schema_descriptor = fixture_text(
            "unica_mcp_script_parity/template-remove/ParityReport/Templates/MainSchema.xml",
        )
        .replace("MainSchema", "ОсновнаяСхема");
        write(
            &self
                .source
                .join("Reports/Продажи/Templates/ОсновнаяСхема.xml"),
            &schema_descriptor,
        );
        let dcs = fixture_text("unica_mcp_script_parity/dcs-validate/BadPrefix.xml")
            .replace("MainData", "Продажи")
            .replace("Code", "Сумма")
            .replace(
                "xmlns:bad=\"http://example.com\">bad:CatalogRef.X",
                ">xs:decimal",
            )
            .replace(
                "\n</DataCompositionSchema>",
                "\n<parameter><name>Период</name><valueType><v8:Type>xs:dateTime</v8:Type></valueType></parameter>\n</DataCompositionSchema>",
            );
        write(
            &self
                .source
                .join("Reports/Продажи/Templates/ОсновнаяСхема/Ext/Template.xml"),
            &dcs,
        );
        let print_descriptor = schema_descriptor
            .replace("ОсновнаяСхема", "Печать")
            .replace("DataCompositionSchema", "SpreadsheetDocument");
        write(
            &self.source.join("Reports/Продажи/Templates/Печать.xml"),
            &print_descriptor,
        );
        let mxl = fixture_text("platform_8_3_27/mxl/Template.xml")
            .replace(
                "\n\t<templateMode>true</templateMode>",
                "\n\t<namedItem xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xsi:type=\"NamedItemCells\"><name>Шапка</name><area><type>Rows</type><beginRow>0</beginRow><endRow>0</endRow><beginColumn>-1</beginColumn><endColumn>-1</endColumn></area></namedItem>\n\t<templateMode>true</templateMode>",
            )
            .replace(
                "\n\t\t\t<empty>true</empty>",
                "\n\t\t\t<c><c><f>0</f><parameter>Заголовок</parameter></c></c>",
            );
        write(
            &self
                .source
                .join("Reports/Продажи/Templates/Печать/Ext/Template.xml"),
            &mxl,
        );
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

#[test]
fn review_rejects_revision_change_during_post_fence_owner_proof() {
    let fixture = RealReaderFixture::new();
    let descriptor = fixture.source.join("Catalogs/Items.xml");
    review_set_before_owner_proof(move || {
        let mut text = fs::read_to_string(&descriptor).unwrap();
        text.push('\n');
        fs::write(&descriptor, text).unwrap();
    });

    let service = fixture.view_service();
    let result = service.view(ViewRequest::new("main:Catalog.Items").unwrap());

    assert!(!result.ok, "mixed-revision read escaped: {result:?}");
    assert_eq!(result.diagnostics[0]["code"], "stale_cursor");
}

#[test]
fn cursor_retry_rejects_revision_change_during_role_canonicalization() {
    let fixture = RealReaderFixture::new();
    let rights_path = fixture.source.join("Roles/SalesReader/Ext/Rights.xml");
    let rights = fs::read_to_string(&rights_path).unwrap().replacen(
        "</Rights>",
        "<object><name>Document.Orders</name><right><name>Read</name><value>true</value></right></object></Rights>",
        1,
    );
    fs::write(&rights_path, &rights).unwrap();
    let service = fixture.view_service();
    let first = service.view(
        ViewRequest::new("main:Role.SalesReader.Right")
            .unwrap()
            .with_limit(1)
            .unwrap(),
    );
    assert!(first.ok, "{:?}", first.diagnostics);
    let cursor = first.cursor.expect("two role objects require a cursor");
    let changed_path = rights_path.clone();
    review_set_after_canonical_role_read(move || {
        let mut changed = fs::read_to_string(&changed_path).unwrap();
        changed.push('\n');
        fs::write(&changed_path, changed).unwrap();
    });

    let replay = service.view(
        ViewRequest::new("main:Role.SalesReader.Right")
            .unwrap()
            .with_limit(1)
            .unwrap()
            .with_cursor(cursor),
    );

    assert!(
        !replay.ok,
        "cursor page crossed a post-canonical read mutation"
    );
    assert_eq!(replay.diagnostics[0]["code"], "stale_cursor");
}

#[test]
pub(crate) fn configuration_level_rights_are_readable_role_objects() {
    let fixture = RealReaderFixture::new();
    let rights_path = fixture.source.join("Roles/SalesReader/Ext/Rights.xml");
    let rights = fs::read_to_string(&rights_path).unwrap().replacen(
        "</Rights>",
        "<object><name>Configuration.CorpusConfiguration</name><right><name>Administration</name><value>true</value></right><right><name>ThinClient</name><value>true</value></right></object></Rights>",
        1,
    );
    fs::write(&rights_path, rights).unwrap();
    let service = fixture.view_service();

    let role = service.view(ViewRequest::new("main:Role.SalesReader").unwrap());
    assert!(role.ok, "{:?}", role.diagnostics);
    let right = service.view(
        ViewRequest::new("main:Role.SalesReader.Right.Configuration_CorpusConfiguration").unwrap(),
    );
    assert!(right.ok, "{:?}", right.diagnostics);
    let data = right.data.as_ref().unwrap();
    assert_eq!(data["kind"], "Right");
    assert_eq!(data["props"]["objectKind"], "Configuration");
    assert_eq!(data["props"]["allowedCount"], 2);

    let authority = fixture.read_authority();
    let index = WorkspaceFindIndexBuilder::default()
        .build(
            &[ActorFindSource::new("main", &authority)],
            crate::domain::code_intelligence::ProviderDeadline::from_budget(
                crate::application::v13::LOGICAL_READ_OPERATION_BUDGET,
            ),
            &fixture.cancellation,
        )
        .unwrap();
    let expected = "main:Role.SalesReader.Right.Configuration_CorpusConfiguration";
    let found = index.find(FindRequest::new(expected).unwrap());
    assert!(
        !found.is_nearest()
            && found
                .candidates()
                .iter()
                .any(|candidate| candidate.at() == expected),
        "configuration right is missing from find: {found:?}"
    );
}

#[test]
pub(crate) fn object_commands_are_registered_inline_without_descriptor_files() {
    let fixture = RealReaderFixture::new();
    let descriptor = fixture.source.join("Catalogs/Items.xml");
    let owner = fs::read_to_string(&descriptor).unwrap();
    // The owner's own `ChildObjects` closes last; nested tabular sections
    // close theirs earlier.
    let close = owner.rfind("</ChildObjects>").unwrap();
    let owner = format!(
        "{}{}{}",
        &owner[..close],
        r#"<Command uuid="10000000-0000-4000-8000-000000000079"><Properties><Name>Inline</Name><Synonym><v8:item><v8:lang>ru</v8:lang><v8:content>Инлайн</v8:content></v8:item></Synonym></Properties></Command>"#,
        &owner[close..]
    );
    fs::write(&descriptor, owner).unwrap();
    write(
        &fixture.source.join("Catalogs/Items/Commands/Inline/Ext/CommandModule.bsl"),
        "&AtClient\nProcedure CommandProcessing(CommandParameter, CommandExecuteParameters)\nEndProcedure\n",
    );
    let service = fixture.view_service();

    let owner = service.view(ViewRequest::new("main:Catalog.Items").unwrap());
    assert!(owner.ok, "{:?}", owner.diagnostics);
    // The shared fixture registers `Refresh` by text and the test adds an
    // inline definition; neither has a `Commands/<Name>.xml` on disk.
    for (at, title) in [
        ("main:Catalog.Items.Command.Refresh", "Refresh"),
        ("main:Catalog.Items.Command.Inline", "Инлайн"),
    ] {
        let command = service.view(ViewRequest::new(at).unwrap());
        assert!(command.ok, "{at}: {:?}", command.diagnostics);
        let data = command.data.as_ref().unwrap();
        assert_eq!(data["kind"], "Command", "{at}");
        assert_eq!(data["title"], title, "{at}");
    }

    let authority = fixture.read_authority();
    let index = WorkspaceFindIndexBuilder::default()
        .build(
            &[ActorFindSource::new("main", &authority)],
            crate::domain::code_intelligence::ProviderDeadline::from_budget(
                crate::application::v13::LOGICAL_READ_OPERATION_BUDGET,
            ),
            &fixture.cancellation,
        )
        .unwrap();
    for expected in [
        "main:Catalog.Items.Command.Refresh",
        "main:Catalog.Items.Command.Inline",
    ] {
        let found = index.find(FindRequest::new(expected).unwrap());
        assert!(
            !found.is_nearest()
                && found
                    .candidates()
                    .iter()
                    .any(|candidate| candidate.at() == expected),
            "inline command identity is missing from find: {expected}: {found:?}"
        );
    }
}

#[test]
pub(crate) fn template_bodies_are_read_only_when_the_template_node_is_addressed() {
    let fixture = RealReaderFixture::new();
    let body = fixture
        .source
        .join("Reports/ParityReport/Templates/Print/Ext/Template.xml");
    let readable = fs::read(&body).unwrap();
    // An unreadable spreadsheet body must not take the owner or the template
    // collection down with it: only the template node opens the body.
    fs::write(
        &body,
        "<document xmlns=\"http://v8.1c.ru/8.2/data/spreadsheet\"><unclosed>",
    )
    .unwrap();
    let service = fixture.view_service();
    for at in [
        "main:Report.ParityReport",
        "main:Report.ParityReport.Template",
    ] {
        let result = service.view(ViewRequest::new(at).unwrap());
        assert!(result.ok, "{at}: {:?}", result.diagnostics);
    }
    let node = service.view(ViewRequest::new("main:Report.ParityReport.Template.Print").unwrap());
    assert!(
        !node.ok,
        "an unreadable template body escaped through its own node"
    );
    assert_eq!(node.diagnostics[0]["code"], "provider_unavailable");

    // A readable body still publishes its interior on the template node.
    fs::write(&body, readable).unwrap();
    let service = fixture.view_service();
    let node = service.view(ViewRequest::new("main:Report.ParityReport.Template.Print").unwrap());
    assert!(node.ok, "{:?}", node.diagnostics);
    let branches = node.data.as_ref().unwrap()["branches"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(
        branches
            .iter()
            .any(|branch| branch["at"] == "main:Report.ParityReport.Template.Print.Area"),
        "{branches:?}"
    );
}

#[test]
pub(crate) fn add_in_templates_stop_addressing_at_the_template_without_reading_the_payload() {
    let fixture = RealReaderFixture::new();
    let descriptor = fixture.source.join("Catalogs/Items.xml");
    let owner = fs::read_to_string(&descriptor).unwrap();
    let close = owner.rfind("</ChildObjects>").unwrap();
    let owner = format!(
        "{}<Template>Driver</Template>{}",
        &owner[..close],
        &owner[close..]
    );
    fs::write(&descriptor, owner).unwrap();
    write(
        &fixture.source.join("Catalogs/Items/Templates/Driver.xml"),
        r#"<?xml version="1.0" encoding="UTF-8"?><MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Template uuid="10000000-0000-4000-8000-000000000081"><Properties><Name>Driver</Name><TemplateType>AddIn</TemplateType></Properties></Template></MetaDataObject>"#,
    );
    // The payload is an opaque archive; make it something no XML reader could parse.
    fs::create_dir_all(fixture.source.join("Catalogs/Items/Templates/Driver/Ext")).unwrap();
    fs::write(
        fixture
            .source
            .join("Catalogs/Items/Templates/Driver/Ext/Template.bin"),
        [0x50, 0x4b, 0x03, 0x04, 0xff, 0x00, 0x80],
    )
    .unwrap();
    let service = fixture.view_service();
    let node = service.view(ViewRequest::new("main:Catalog.Items.Template.Driver").unwrap());
    assert!(node.ok, "{:?}", node.diagnostics);
    let data = node.data.as_ref().unwrap();
    assert_eq!(data["kind"], "Template");
    assert!(
        data.get("branches").is_none(),
        "an add-in template has no addressable interior: {data}"
    );

    let authority = fixture.read_authority();
    let index = WorkspaceFindIndexBuilder::default()
        .build(
            &[ActorFindSource::new("main", &authority)],
            crate::domain::code_intelligence::ProviderDeadline::from_budget(
                crate::application::v13::LOGICAL_READ_OPERATION_BUDGET,
            ),
            &fixture.cancellation,
        )
        .unwrap();
    let expected = "main:Catalog.Items.Template.Driver";
    let found = index.find(FindRequest::new(expected).unwrap());
    assert!(
        !found.is_nearest()
            && found
                .candidates()
                .iter()
                .any(|candidate| candidate.at() == expected),
        "add-in template identity is missing from find: {found:?}"
    );
}

#[test]
fn review_rejects_orphan_nested_module_owners_not_registered_by_parent() {
    let fixture = RealReaderFixture::new();
    write(
        &fixture.source.join("Catalogs/Items/Forms/Orphan.xml"),
        r#"<?xml version="1.0" encoding="UTF-8"?><MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Form uuid="10000000-0000-4000-8000-000000000077"><Properties><Name>Orphan</Name><FormType>Managed</FormType></Properties></Form></MetaDataObject>"#,
    );
    write(
        &fixture.source.join("Catalogs/Items/Commands/Orphan/Ext/CommandModule.bsl"),
        "&AtClient\nProcedure CommandProcessing(CommandParameter, CommandExecuteParameters)\nEndProcedure\n",
    );
    let service = fixture.view_service();

    for address in [
        "main:Catalog.Items.Form.Orphan.Module",
        "main:Catalog.Items.Command.Orphan.Module",
    ] {
        let result = service.view(ViewRequest::new(address).unwrap());
        assert!(
            !result.ok,
            "orphan owner was invented for {address}: {result:?}"
        );
        assert_eq!(result.diagnostics[0]["code"], "not_found");
    }
}

#[test]
fn review_rejects_direct_typed_owner_absent_from_configuration_inventory() {
    let fixture = RealReaderFixture::new();
    write(
        &fixture.source.join("Roles/Orphan.xml"),
        r#"<?xml version="1.0" encoding="UTF-8"?><MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Role uuid="10000000-0000-4000-8000-000000000079"><Properties><Name>Orphan</Name></Properties></Role></MetaDataObject>"#,
    );
    write(
        &fixture.source.join("Roles/Orphan/Ext/Rights.xml"),
        r#"<?xml version="1.0" encoding="UTF-8"?><Rights xmlns="http://v8.1c.ru/8.2/roles" version="2.20"/>"#,
    );
    let service = fixture.view_service();

    let result = service.view(ViewRequest::new("main:Role.Orphan").unwrap());
    assert!(
        !result.ok,
        "unregistered role entered the logical tree: {result:?}"
    );
    assert_eq!(result.diagnostics[0]["code"], "not_found");
}

#[test]
fn registered_physical_child_with_wrong_descriptor_fails_direct_and_parent_navigation() {
    let fixture = RealReaderFixture::new();
    let owner_path = fixture.source.join("Reports/ParityReport.xml");
    let owner = fs::read_to_string(&owner_path).unwrap().replacen(
        "</ChildObjects>",
        "<Form>Wrong</Form></ChildObjects>",
        1,
    );
    fs::write(owner_path, owner).unwrap();
    write(
        &fixture.source.join("Reports/ParityReport/Forms/Wrong.xml"),
        r#"<?xml version="1.0" encoding="UTF-8"?><MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Command><Properties><Name>Wrong</Name></Properties></Command></MetaDataObject>"#,
    );
    let service = fixture.view_service();

    for at in [
        "main:Report.ParityReport.Form.Wrong",
        "main:Report.ParityReport",
    ] {
        let result = service.view(ViewRequest::new(at).unwrap());
        assert!(!result.ok, "wrong child descriptor escaped through {at}");
        assert_eq!(result.diagnostics[0]["code"], "provider_unavailable");
    }
}

#[test]
fn registered_top_level_owner_without_descriptor_fails_kind_branch_and_direct_view() {
    let fixture = RealReaderFixture::new();
    let configuration = fixture.source.join("Configuration.xml");
    let owner = fs::read_to_string(&configuration).unwrap().replacen(
        "</ChildObjects>",
        "<Role>Missing</Role></ChildObjects>",
        1,
    );
    fs::write(configuration, owner).unwrap();
    let service = fixture.view_service();

    for at in ["main:Configuration", "main:Role", "main:Role.Missing"] {
        let result = service.view(ViewRequest::new(at).unwrap());
        assert!(
            !result.ok,
            "missing top-level descriptor escaped through {at}"
        );
        assert_eq!(
            result.diagnostics[0]["code"], "provider_unavailable",
            "{at}"
        );
    }
}

#[test]
fn orphan_and_missing_physical_children_fail_closed_across_reader_families() {
    let orphan = RealReaderFixture::new();
    for (directory, kind, name) in [
        ("Forms", "Form", "OrphanForm"),
        ("Templates", "Template", "OrphanDcs"),
        ("Templates", "Template", "OrphanMxl"),
        ("Commands", "Command", "OrphanCommand"),
    ] {
        write(
            &orphan
                .source
                .join(format!("Reports/ParityReport/{directory}/{name}.xml")),
            &format!(
                r#"<?xml version="1.0" encoding="UTF-8"?><MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><{kind}><Properties><Name>{name}</Name><TemplateType>DataCompositionSchema</TemplateType></Properties></{kind}></MetaDataObject>"#
            ),
        );
    }
    let service = orphan.view_service();
    for at in [
        "main:Report.ParityReport.Form.OrphanForm",
        "main:Report.ParityReport.Template.OrphanDcs.DataSet",
        "main:Report.ParityReport.Template.OrphanMxl.Area",
        "main:Report.ParityReport.Command.OrphanCommand",
    ] {
        let result = service.view(ViewRequest::new(at).unwrap());
        assert!(!result.ok, "orphan physical child escaped through {at}");
        assert_eq!(result.diagnostics[0]["code"], "not_found", "{at}");
    }

    let missing = RealReaderFixture::new();
    let owner_path = missing.source.join("Reports/ParityReport.xml");
    let owner = fs::read_to_string(&owner_path).unwrap().replacen(
        "</ChildObjects>",
        "<Form>MissingForm</Form><Template>MissingDcs</Template><Template>MissingMxl</Template></ChildObjects>",
        1,
    );
    fs::write(owner_path, owner).unwrap();
    let service = missing.view_service();
    for at in [
        "main:Report.ParityReport.Form.MissingForm",
        "main:Report.ParityReport.Template.MissingDcs.DataSet",
        "main:Report.ParityReport.Template.MissingMxl.Area",
    ] {
        let result = service.view(ViewRequest::new(at).unwrap());
        assert!(
            !result.ok,
            "missing registered descriptor escaped through {at}"
        );
        assert_eq!(
            result.diagnostics[0]["code"], "provider_unavailable",
            "{at}"
        );
    }
}

#[test]
fn unregistered_top_level_descriptors_cannot_enter_any_typed_reader_family() {
    let fixture = RealReaderFixture::new();
    write_metadata_owner(&fixture.source, "Catalogs", "Catalog", "Orphan", "");
    write_metadata_owner(&fixture.source, "Subsystems", "Subsystem", "Orphan", "");
    write_metadata_owner(&fixture.source, "XDTOPackages", "XDTOPackage", "Orphan", "");
    write_metadata_owner(&fixture.source, "CommonForms", "CommonForm", "Orphan", "");
    write(
        &fixture.source.join("CommonForms/Orphan/Ext/Form.xml"),
        r#"<?xml version="1.0" encoding="UTF-8"?><Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20"><ChildItems/></Form>"#,
    );
    let service = fixture.view_service();
    for at in [
        "main:Catalog.Orphan",
        "main:Catalog.Orphan.Module",
        "main:Subsystem.Orphan",
        "main:Subsystem.Orphan.Interface",
        "main:XDTOPackage.Orphan",
        "main:CommonForm.Orphan",
    ] {
        let result = service.view(ViewRequest::new(at).unwrap());
        assert!(
            !result.ok,
            "unregistered top-level owner escaped through {at}"
        );
        assert_eq!(result.diagnostics[0]["code"], "not_found", "{at}");
    }
}

#[test]
fn external_parent_childobjects_are_the_only_nested_owner_authority() {
    let fixture = RealExternalReaderFixture::new();
    write(
        &fixture.processor.join("Импорт/Forms/Orphan.xml"),
        r#"<?xml version="1.0" encoding="UTF-8"?><MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Form><Properties><Name>Orphan</Name></Properties></Form></MetaDataObject>"#,
    );
    let authority = fixture.read_authority("artifact_processor", SourceSetKind::ExternalProcessor);
    let service = ViewService::new(authority, ViewCursorStore::default());
    let orphan = service.view(
        ViewRequest::new("artifact_processor:ExternalDataProcessor.Импорт.Form.Orphan").unwrap(),
    );
    assert!(!orphan.ok, "orphan external form became addressable");
    assert_eq!(orphan.diagnostics[0]["code"], "not_found");

    let descriptor = fixture.processor.join("Импорт.xml");
    let owner = fs::read_to_string(&descriptor).unwrap().replacen(
        "</ChildObjects>",
        "<Form>Missing</Form></ChildObjects>",
        1,
    );
    fs::write(descriptor, owner).unwrap();
    let authority = fixture.read_authority("artifact_processor", SourceSetKind::ExternalProcessor);
    let service = ViewService::new(authority, ViewCursorStore::default());
    let parent =
        service.view(ViewRequest::new("artifact_processor:ExternalDataProcessor.Импорт").unwrap());
    assert!(
        !parent.ok,
        "external parent published a missing registered child"
    );
    assert_eq!(parent.diagnostics[0]["code"], "provider_unavailable");
}

#[test]
fn review_revision_authority_cannot_be_swapped_after_named_identity_validation() {
    let fixture = RealReaderFixture::new();
    let replacement = fixture.root.path().join("replacement");
    fs::create_dir_all(&replacement).unwrap();
    fs::write(
        replacement.join("Configuration.xml"),
        r#"<?xml version="1.0" encoding="UTF-8"?><MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>ReplacementConfiguration</Name></Properties><ChildObjects/></Configuration></MetaDataObject>"#,
    )
    .unwrap();
    let saved = fixture.root.path().join("retained-a");
    let source_before = fixture.source.clone();
    let replacement_before = replacement.clone();
    let saved_before = saved.clone();
    let source_after = fixture.source.clone();
    let replacement_after = replacement;
    let saved_after = saved;
    review_set_revision_identity_hooks(
        move || {
            if saved_before.exists() {
                fs::rename(&source_before, &replacement_before).unwrap();
                fs::rename(&saved_before, &source_before).unwrap();
            }
        },
        move || {
            fs::rename(&source_after, &saved_after).unwrap();
            fs::rename(&replacement_after, &source_after).unwrap();
        },
    );

    let service = fixture.view_service();
    let result = service.view(ViewRequest::new("main:Configuration").unwrap());
    review_clear_revision_identity_hooks();

    assert!(
        !result.ok,
        "revision came from replacement B while retained bytes came from A: {result:?}"
    );
}

#[test]
fn review_role_canonical_encoding_cannot_collapse_distinct_kind_name_pairs() {
    let fixture = RealReaderFixture::new();
    fs::write(
        fixture.source.join("Roles/SalesReader/Ext/Rights.xml"),
        r#"<?xml version="1.0" encoding="UTF-8"?>
<Rights xmlns="http://v8.1c.ru/8.2/roles" version="2.20">
  <object><name>Catalog.Orders_X</name><right><name>Read</name><value>true</value></right></object>
  <object><name>Catalog_Orders.X</name><right><name>Read</name><value>true</value></right></object>
</Rights>"#,
    )
    .unwrap();
    let service = fixture.view_service();

    let result = service.view(ViewRequest::new("main:Role.SalesReader.Right").unwrap());
    assert!(!result.ok, "invalid platform kinds must fail closed");
    assert_eq!(result.diagnostics[0]["code"], "provider_unavailable");
    assert!(result.diagnostics[0]["message"]
        .as_str()
        .is_some_and(|message| message.contains("Catalog_Orders")));
}

#[test]
fn review_role_rejects_non_platform_metadata_node_kinds() {
    let fixture = RealReaderFixture::new();
    fs::write(
        fixture.source.join("Roles/SalesReader/Ext/Rights.xml"),
        r#"<?xml version="1.0" encoding="UTF-8"?>
<Rights xmlns="http://v8.1c.ru/8.2/roles" version="2.20">
  <object><name>WebSocketClient.Telephony</name><right><name>Read</name><value>true</value></right></object>
</Rights>"#,
    )
    .unwrap();
    let service = fixture.view_service();

    let result = service.view(ViewRequest::new("main:Role.SalesReader.Right").unwrap());
    assert!(
        !result.ok,
        "non-platform role object kind escaped: {result:?}"
    );
    assert_eq!(result.diagnostics[0]["code"], "provider_unavailable");
}

#[test]
fn review_production_read_port_has_no_nocancel_inventory_entrypoint() {
    let source = include_str!("../v13_read_port.rs");
    assert!(
        !source.contains("configuration_payload_with_checkpoint(&mut || Ok(()))"),
        "production-compiled read port retains a no-op cancellation bypass"
    );
}
