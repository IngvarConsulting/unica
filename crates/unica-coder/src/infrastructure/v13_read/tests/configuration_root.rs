use super::*;

/// The root projection carries vendor facts, the support state, the closed root properties and the
/// object total, every declared key present even when its value is null.
#[test]
fn configuration_root_carries_vendor_support_and_closed_root_properties() {
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

    let root = service.view(ViewRequest::new("main:Configuration").unwrap());
    let props = root.data.as_ref().unwrap()["props"].as_object().unwrap();
    for key in [
        "format",
        "name",
        "synonym",
        "version",
        "vendor",
        "extensionPurpose",
        "totalObjects",
        "support",
        "properties",
        "homePage",
    ] {
        assert!(
            props.contains_key(key),
            "root props miss `{key}`: {props:?}"
        );
    }
    assert_eq!(props["format"], "2.20");
    assert!(props["totalObjects"].as_u64().unwrap() > 0);
    assert!(props["support"]["state"].is_string());
    let properties = props["properties"].as_object().unwrap();
    for key in [
        "compatibilityMode",
        "defaultRunMode",
        "scriptVariant",
        "defaultLanguage",
        "dataLockControlMode",
        "modalityUseMode",
        "interfaceCompatibilityMode",
        "extensionCompatibilityMode",
        "objectAutonumerationMode",
        "synchronousCallUseMode",
        "databaseTablespacesUseMode",
        "mainWindowMode",
        "comment",
        "namePrefix",
        "updateCatalogAddress",
    ] {
        assert!(properties.contains_key(key), "root properties miss `{key}`");
    }
}

/// The root shows the properties of its set's line with the file's values;
/// a set of an earlier line has none of them.
/// rule: INV.SOURCE.PROFILE-CAPABILITIES
#[test]
fn configuration_root_shows_the_properties_of_its_line() {
    use unica_source_contract::format_identity::PlatformLine;
    for (format, source, line, expected) in [
        (
            "2.20",
            "../../tests/fixtures/formats/platform_xml/2.20/all_kinds/src",
            PlatformLine::new(8, 3, 27),
            None,
        ),
        (
            "2.21",
            "../../tests/fixtures/formats/platform_xml/2.21/all_kinds/src",
            PlatformLine::new(8, 5, 1),
            Some(["NavigationLeft", "Auto", "OpenDataInDialogs", "DontUse"]),
        ),
        (
            "2.22",
            "../../tests/fixtures/formats/platform_xml/2.22/all_kinds/src",
            PlatformLine::new(8, 5, 4),
            Some(["NavigationLeft", "Auto", "OpenDataInDialogs", "DontUse"]),
        ),
    ] {
        // The secure source reader takes only normal path components.
        let source =
            std::fs::canonicalize(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(source))
                .unwrap();
        let cancellation = CancellationToken::new();
        let source_root = Arc::new(unica_workspace::read::open_source_directory(&source).unwrap());
        let authority = LogicalViewReadAuthority::new(
            &cancellation,
            "main",
            "actor-fixture-main",
            SourceSetKind::Configuration,
            source_root,
            PlatformProfile::for_line(line).unwrap(),
        );
        let service = ViewService::new(authority, ViewCursorStore::default());
        let root = service.view(ViewRequest::new("main:Configuration").unwrap());
        let props = root.data.as_ref().unwrap()["props"].as_object().unwrap();
        assert_eq!(props["format"], format);
        assert!(props["caption"].is_null(), "{format}");
        let properties = &props["properties"];
        let observed = [
            "mainWindowInterfaceVariant",
            "clientApplicationTheme",
            "windowsOpenVariant",
            "version85InterfaceMigrationMode",
        ]
        .map(|key| properties[key].as_str());
        assert_eq!(
            observed,
            expected.map(|values| values.map(Some)).unwrap_or([None; 4]),
            "{format}"
        );
    }
}

/// An enumeration value shows its color in a set whose line has it.
/// rule: INV.SOURCE.PROFILE-CAPABILITIES
#[test]
fn an_enumeration_value_shows_its_color_in_the_lines_that_have_it() {
    use unica_source_contract::format_identity::PlatformLine;
    for (source, line, expected) in [
        (
            "../../tests/fixtures/formats/platform_xml/2.20/all_kinds/src",
            PlatformLine::new(8, 3, 27),
            None,
        ),
        (
            "../../tests/fixtures/formats/platform_xml/2.21/all_kinds/src",
            PlatformLine::new(8, 5, 1),
            Some("auto"),
        ),
    ] {
        // The secure source reader takes only normal path components.
        let source =
            std::fs::canonicalize(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(source))
                .unwrap();
        let cancellation = CancellationToken::new();
        let source_root = Arc::new(unica_workspace::read::open_source_directory(&source).unwrap());
        let authority = LogicalViewReadAuthority::new(
            &cancellation,
            "main",
            "actor-fixture-main",
            SourceSetKind::Configuration,
            source_root,
            PlatformProfile::for_line(line).unwrap(),
        );
        let service = ViewService::new(authority, ViewCursorStore::default());
        let value = service.view(
            ViewRequest::new("main:Enum.СпособыУстановкиКурсаВалюты.EnumValue.ЗагрузкаИзИнтернета")
                .unwrap(),
        );
        let props = &value.data.as_ref().unwrap()["props"];
        assert_eq!(
            props.get("color").and_then(|color| color.as_str()),
            expected,
            "{line}"
        );
    }
}

/// A palette colour is a kind of 8.5.1 and later: the root of an earlier
/// line has no branch of it, and in the lines that have it the colour shows
/// its synonym, comment and `#RRGGBB` colour.
/// rule: INV.SOURCE.PROFILE-CAPABILITIES
#[test]
fn a_palette_color_is_shown_in_the_lines_that_have_it() {
    use unica_source_contract::format_identity::PlatformLine;
    for (source, line, has) in [
        (
            "../../tests/fixtures/formats/platform_xml/2.20/all_kinds/src",
            PlatformLine::new(8, 3, 27),
            false,
        ),
        (
            "../../tests/fixtures/formats/platform_xml/2.21/all_kinds/src",
            PlatformLine::new(8, 5, 1),
            true,
        ),
        (
            "../../tests/fixtures/formats/platform_xml/2.22/all_kinds/src",
            PlatformLine::new(8, 5, 4),
            true,
        ),
    ] {
        // The secure source reader takes only normal path components.
        let source =
            std::fs::canonicalize(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(source))
                .unwrap();
        let cancellation = CancellationToken::new();
        let source_root = Arc::new(unica_workspace::read::open_source_directory(&source).unwrap());
        let authority = LogicalViewReadAuthority::new(
            &cancellation,
            "main",
            "actor-fixture-main",
            SourceSetKind::Configuration,
            source_root,
            PlatformProfile::for_line(line).unwrap(),
        );
        let service = ViewService::new(authority, ViewCursorStore::default());
        let root = service.view(ViewRequest::new("main:Configuration").unwrap());
        let branch = root.data.as_ref().unwrap()["branches"]
            .as_array()
            .unwrap()
            .iter()
            .find(|branch| branch["at"] == "main:PaletteColor")
            .cloned();
        if !has {
            assert_eq!(branch, None, "{line}");
            continue;
        }
        assert_eq!(branch.unwrap()["count"], 1, "{line}");
        let color = service.view(ViewRequest::new("main:PaletteColor.Probe").unwrap());
        assert!(
            color.ok,
            "{line}: {} {:?}",
            color.summary, color.diagnostics
        );
        let props = &color.data.as_ref().unwrap()["props"];
        assert_eq!(props["kind"], "PaletteColor", "{line}");
        assert_eq!(props["Synonym"], "Проба", "{line}");
        assert_eq!(props["Color"], "#1A2B3C", "{line}");
        assert_eq!(props["Comment"], "all_kinds", "{line}");
    }
}

/// rule: INV.SOURCE.LOGICAL-BRANCH-COUNT
#[test]
fn configuration_root_branch_counts_match_every_reachable_collection() {
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

/// rule: INV.SOURCE.LOGICAL-BRANCH-COUNT
/// rule: INV.SOURCE.PLATFORM-CAPABILITY-EXISTENCE
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

#[test]
fn every_accepted_profile_address_has_a_real_non_skipping_view() {
    let fixture = RealReaderFixture::new();
    fixture.install_accepted_profile_sources();
    let service = fixture.view_service();
    let profile: AcceptedAddressProfile = serde_json::from_str(include_str!(
        "../../../../../../tests/fixtures/v013/address-profile-8.3.27.json"
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
    let root = Arc::new(unica_workspace::read::open_source_directory(&fixture.source).unwrap());
    let reader = ProviderReadAuthority::new(
        "main",
        "actor-fixture-main",
        SourceSetKind::Configuration,
        root,
    );
    reader
        .exact_revision(
            unica_work_control::provider_deadline::ProviderDeadline::from_budget(
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
    assert_eq!(payload["totalObjects"], 17);
    assert_eq!(
        payload["childObjects"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["count"].as_u64().unwrap())
            .sum::<u64>(),
        17,
    );
    assert_eq!(payload["registeredObjects"].as_array().unwrap().len(), 17);
}

#[test]
fn real_typed_readers_cover_every_task14_profile_without_skipping() {
    let fixture = RealReaderFixture::new();
    let cancellation = CancellationToken::new();
    let source_root =
        Arc::new(unica_workspace::read::open_source_directory(&fixture.source).unwrap());
    let metadata_target = MetadataAddress::parse_logical("Catalog.Items").unwrap();
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

/// rule: INV.SOURCE.PLATFORM-CAPABILITY-EXISTENCE
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
    let source_root =
        Arc::new(unica_workspace::read::open_source_directory(&fixture.source).unwrap());
    let authority = LogicalViewReadAuthority::new(
        &cancellation,
        "main",
        "actor-fixture-extension",
        SourceSetKind::Extension,
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
    let source_root =
        Arc::new(unica_workspace::read::open_source_directory(&fixture.source).unwrap());
    let authority = LogicalViewReadAuthority::new(
        &cancellation,
        "main",
        "actor-fixture-extension-root",
        SourceSetKind::Extension,
        source_root,
        PlatformProfile::v8_3_27(),
    );

    let module = ViewService::new(authority, ViewCursorStore::default())
        .view(ViewRequest::new("main:Module.ManagedApplication").unwrap());
    assert!(module.ok, "{module:?}");

    let source_root =
        Arc::new(unica_workspace::read::open_source_directory(&fixture.source).unwrap());
    let authority = LogicalViewReadAuthority::new(
        &cancellation,
        "main",
        "actor-fixture-extension-root-find",
        SourceSetKind::Extension,
        source_root,
        PlatformProfile::v8_3_27(),
    );
    let index = ReaderReach::new(vec![("main", &authority)]);
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
fn find_walks_every_real_addressable_reader_family_without_parallel_xml_semantics() {
    let fixture = RealReaderFixture::new();
    fixture.install_accepted_profile_sources();
    let authority = fixture.operation_read_authority();
    let index = ReaderReach::new(vec![("main", &authority)]);
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

/// rule: INV.SOURCE.PLATFORM-CAPABILITY-EXISTENCE
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
        "../../../../../../tests/fixtures/v013/address-profile-8.3.27.json"
    ))
    .unwrap()
    .module_capabilities
    .into_iter()
    .filter(|case| case.exists)
    .map(|case| case.at)
    .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(expected.len(), 25);

    let index = ReaderReach::new(vec![("main", &main), ("epf", &epf), ("erf", &erf)]);
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

/// An extension root of 8.5.4 shows its actions with the file's values, a
/// composite one by its parts; an earlier line has none of them.
/// rule: INV.SOURCE.PROFILE-CAPABILITIES
#[test]
fn an_extension_root_shows_the_actions_of_its_line() {
    use unica_source_contract::format_identity::PlatformLine;
    for (source, line, expected) in [
        (
            "../../tests/fixtures/formats/platform_xml/2.21/empty_extension/ext",
            PlatformLine::new(8, 5, 1),
            false,
        ),
        (
            "../../tests/fixtures/formats/platform_xml/2.22/empty_extension/ext",
            PlatformLine::new(8, 5, 4),
            true,
        ),
    ] {
        // The secure source reader takes only normal path components.
        let source =
            std::fs::canonicalize(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(source))
                .unwrap();
        let cancellation = CancellationToken::new();
        let source_root = Arc::new(unica_workspace::read::open_source_directory(&source).unwrap());
        let authority = LogicalViewReadAuthority::new(
            &cancellation,
            "ext",
            "actor-fixture-ext",
            SourceSetKind::Extension,
            source_root,
            PlatformProfile::for_line(line).unwrap(),
        );
        let service = ViewService::new(authority, ViewCursorStore::default());
        let root = service.view(ViewRequest::new("ext:Configuration").unwrap());
        let properties = &root.data.as_ref().unwrap()["props"]["properties"];
        let observed = [
            &properties["actionOnConfigurationObjectMissing"],
            &properties["defaultExtensionPropertyCheckAction"],
            &properties["typeReductionByConfigurationExtensionMode"],
            &properties["actionOnExtendedConfigurationObjectsMissing"],
        ];
        if expected {
            assert_eq!(
                observed,
                [
                    &serde_json::json!("Auto"),
                    &serde_json::json!("Error"),
                    &serde_json::json!("TransformValues"),
                    &serde_json::json!({
                        "adoptedObject":"Error","notifiedObject":"Error",
                        "checkedObject":"Error","extendedObject":"Error"
                    }),
                ]
            );
        } else {
            assert!(observed.iter().all(|value| value.is_null()), "{observed:?}");
        }
    }
}
