use super::*;

/// rule: INV.SOURCE.ROLE-RIGHT-ADDRESSES
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

/// rule: INV.SOURCE.METADATA-NODE-PROPERTIES
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

/// rule: INV.SOURCE.ROLE-RIGHT-ADDRESSES
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
    assert_eq!(error.code().as_str(), "bad_value");
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

/// rule: INV.APP.DEFERRED-READ
#[test]
fn cursor_retry_uses_saved_role_results_without_recanonicalizing() {
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
    fs::write(&rights_path, b"changed source is no longer valid XML").unwrap();
    let replay = service.view(
        ViewRequest::new("main:Role.SalesReader.Right")
            .unwrap()
            .with_limit(1)
            .unwrap()
            .with_cursor(cursor),
    );

    assert!(replay.ok, "saved cursor must remain usable: {replay:?}");
}

/// rule: INV.SOURCE.CONFIGURATION-RIGHTS-READING
/// rule: INV.SOURCE.ROLE-RIGHT-ADDRESSES
#[test]
pub(crate) fn configuration_level_rights_are_readable_role_objects() {
    let fixture = RealReaderFixture::new();
    let rights_path = fixture.source.join("Roles/SalesReader/Ext/Rights.xml");
    let rights = fs::read_to_string(&rights_path).unwrap().replacen(
        "</Rights>",
        "<object><name>Configuration.CorpusConfiguration</name><right><name>Administration</name><value>true</value></right><right><name>ThinClient</name><value>true</value></right><right><name>FutureConfigurationReadCapability</name><value>true</value></right><right><name>FutureConfigurationWriteCapability</name><value>false</value></right></object></Rights>",
        1,
    );
    fs::write(&rights_path, &rights).unwrap();
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
    assert_eq!(data["props"]["allowedCount"], 3);
    assert_eq!(data["props"]["deniedCount"], 1);
    assert_eq!(fs::read_to_string(&rights_path).unwrap(), rights);

    let authority = fixture.read_authority();
    let index = ReaderReach::new(vec![("main", &authority)]);
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
