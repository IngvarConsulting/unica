use super::*;

/// rule: INV.APP.ACTOR-AUTHENTICATED-SOURCE-CAPABILITIES
#[test]
fn actor_owned_reader_never_follows_a_source_set_remap_after_admission() {
    let fixture = RealReaderFixture::new();
    let cancellation = CancellationToken::new();
    let source_root =
        Arc::new(unica_workspace::read::open_source_directory(&fixture.source).unwrap());
    let authority = LogicalViewReadAuthority::new(
        &cancellation,
        "main",
        "actor-fixture-main",
        SourceSetKind::Configuration,
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

/// rule: INV.APP.ACTOR-AUTHENTICATED-SOURCE-CAPABILITIES
#[test]
fn actor_owned_configuration_support_and_home_page_sidecars_are_retained() {
    let fixture = RealReaderFixture::new();
    fs::create_dir_all(fixture.source.join("Ext")).unwrap();
    fs::copy(
        fixture_path(
            "formats/platform_xml/2.20/support-edit-bin-only/src/Ext/ParentConfigurations.bin",
        ),
        fixture.source.join("Ext/ParentConfigurations.bin"),
    )
    .unwrap();
    fs::copy(
        fixture_path("unica_mcp_script_parity/cf-info/Ext/HomePageWorkArea.xml"),
        fixture.source.join("Ext/HomePageWorkArea.xml"),
    )
    .unwrap();
    let cancellation = CancellationToken::new();
    let source_root =
        Arc::new(unica_workspace::read::open_source_directory(&fixture.source).unwrap());
    let authority = LogicalViewReadAuthority::new(
        &cancellation,
        "main",
        "actor-fixture-main",
        SourceSetKind::Configuration,
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

/// rule: INV.SOURCE.OPTIONAL-HOME-PAGE
#[test]
fn retained_home_page_distinguishes_missing_from_malformed_and_wrong_root() {
    let fixture = RealReaderFixture::new();
    fs::create_dir_all(fixture.source.join("Ext")).unwrap();
    let root = Arc::new(unica_workspace::read::open_source_directory(&fixture.source).unwrap());
    let reader = ProviderReadAuthority::new(
        "main",
        "actor-fixture-main",
        SourceSetKind::Configuration,
        root,
    );
    let sidecar = fixture.source.join("Ext/HomePageWorkArea.xml");

    let missing = configuration_payload(&reader).unwrap();
    assert!(missing["homePage"].is_null());

    fs::write(&sidecar, "<broken>").unwrap();
    let malformed = configuration_payload(&reader).unwrap_err();
    assert_eq!(malformed.code().as_str(), "provider_unavailable");

    fs::write(
        &sidecar,
        r#"<NotHomePage xmlns="http://v8.1c.ru/8.3/xcf/extrnprops" version="2.20"/>"#,
    )
    .unwrap();
    let wrong_root = configuration_payload(&reader).unwrap_err();
    assert_eq!(wrong_root.code().as_str(), "provider_unavailable");

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
    let root = Arc::new(unica_workspace::read::open_source_directory(&fixture.source).unwrap());
    let reader = ProviderReadAuthority::new(
        "extension",
        "actor-fixture-extension",
        SourceSetKind::Extension,
        root,
    );

    let payload = configuration_payload(&reader).unwrap();
    assert_eq!(payload["support"]["state"], "extension");
}

/// rule: INV.APP.ACTOR-AUTHENTICATED-SOURCE-CAPABILITIES
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

/// rule: INV.APP.ACTOR-AUTHENTICATED-SOURCE-CAPABILITIES
/// rule: INV.SOURCE.RETAINED-LOGICAL-PUBLICATION
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

/// rule: INV.APP.ACTOR-AUTHENTICATED-SOURCE-CAPABILITIES
/// rule: INV.SOURCE.RETAINED-LOGICAL-PUBLICATION
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
fn read_authority_rejects_named_root_replacement_before_node_read() {
    if !supports_retained_root_replacement_test() {
        unica_process::test_support::skip_without_host_capability(
            unica_process::test_support::HostCapability::RenameUnderOpenHandle,
            "the platform keeps an open handle from being renamed or replaced",
        );
        return;
    }
    let fixture = RealReaderFixture::new();
    let root = Arc::new(unica_workspace::read::open_source_directory(&fixture.source).unwrap());
    let deadline = unica_work_control::provider_deadline::ProviderDeadline::no_deadline();
    let authority = LogicalViewReadAuthority::with_read_authority(
        &fixture.cancellation,
        ProviderReadAuthority::new(
            "main",
            "actor-fixture-main",
            SourceSetKind::Configuration,
            root,
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
