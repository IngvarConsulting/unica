use super::*;

const PACKAGES: &[(&str, &[&str])] = &[
    (
        "DataArtefacts_Files_1_0_0_1",
        &["http://www.1c.ru/1cFresh/Data/Dump/1.0.2.1"],
    ),
    (
        "УправлениеПриложениями",
        &[
            "http://www.1c.ru/SaaS/Messages",
            "http://v8.1c.ru/8.1/data/core",
        ],
    ),
    ("egais_EGCommonV3", &["http://fsrar.ru/WEGAIS/Common"]),
    (
        "egais_ProductRef",
        &[
            "http://fsrar.ru/WEGAIS/Common",
            "http://fsrar.ru/WEGAIS/ClientRef",
        ],
    ),
    ("АгрегацияМОТП", &["http://ismotp.crptech.ru/types"]),
];

fn install(fixture: &RealReaderFixture, name: &str, content: &str) {
    let root = fixture.source.join("Configuration.xml");
    write(
        &root,
        &fs::read_to_string(&root).unwrap().replace(
            "</ChildObjects>",
            &format!("<XDTOPackage>{name}</XDTOPackage></ChildObjects>"),
        ),
    );
    write(&fixture.source.join(format!("XDTOPackages/{name}.xml")), &format!(
        "<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.20\"><XDTOPackage uuid=\"15940000-0000-4000-8000-000000000001\"><Properties><Name>{name}</Name><Namespace>urn:imports:{name}</Namespace></Properties></XDTOPackage></MetaDataObject>"
    ));
    write(&fixture.source.join(format!("XDTOPackages/{name}/Ext/Package.bin")), &format!(
        "<package xmlns=\"http://v8.1c.ru/8.1/xdto\" targetNamespace=\"urn:imports:{name}\">{content}</package>"
    ));
}

#[test]
fn xdto_import_collection_exposes_every_declared_uri_of_the_five_packages() {
    let fixture = RealReaderFixture::new();
    for (name, uris) in PACKAGES {
        let mut content = uris
            .iter()
            .map(|uri| format!("<import namespace=\"{uri}\"/>"))
            .collect::<String>();
        content.push_str(&format!(
            "<objectType name=\"Probe\" xmlns:external=\"{}\" base=\"external:Base\"/>",
            uris[0]
        ));
        install(&fixture, name, &content);
    }
    let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
    let service = fixture.view_service();
    let mut observed = 0;
    for (name, uris) in PACKAGES {
        let at = format!("main:XDTOPackage.{name}");
        let owner = service.view(ViewRequest::new(&at).unwrap());
        assert!(owner.ok, "{:?}", refusal_codes(&owner));
        let owner = owner.data.unwrap();
        let expected_branch = json!({"at":format!("{at}.Import"),"count":uris.len()});
        assert!(owner["branches"]
            .as_array()
            .unwrap()
            .contains(&expected_branch));
        let mut cursor = None;
        let mut items = Vec::new();
        loop {
            let mut request = ViewRequest::new(expected_branch["at"].as_str().unwrap())
                .unwrap()
                .with_limit(1)
                .unwrap();
            if let Some(value) = cursor.take() {
                request = request.with_cursor(value);
            }
            let page = service.view(request);
            assert!(page.ok, "{:?}", refusal_codes(&page));
            let data = page.data.unwrap();
            assert!(data.get("limits").is_none());
            items.extend(data["items"].as_array().unwrap().clone());
            cursor = page.cursor;
            if cursor.is_none() {
                break;
            }
        }
        assert_eq!(
            items,
            uris.iter()
                .map(|uri| json!({"namespace":uri}))
                .collect::<Vec<_>>()
        );
        observed += items.len();
        let namespace = service.view(ViewRequest::new(&format!("{at}.Namespace")).unwrap());
        assert!(namespace.ok);
        assert_eq!(
            namespace.data.unwrap()["props"]["namespace"],
            format!("urn:imports:{name}")
        );
        let external = service.view(ViewRequest::new(&format!("{at}.Type.Probe")).unwrap());
        assert!(external.ok);
        assert_eq!(
            external.data.unwrap()["props"]["base"]["namespace"],
            uris[0]
        );
    }
    assert_eq!(observed, 7);
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&fixture.source),
        before
    );
}

#[test]
fn xdto_import_read_distinguishes_empty_damage_and_unknown_package_syntax() {
    for (content, expected, limited) in [
        ("", json!([]), false),
        (
            "<import namespace=\"urn:one\"/>",
            json!([{"namespace":"urn:one"}]),
            false,
        ),
        (
            "<import namespace=\"urn:one\"/><import namespace=\"urn:one\"/>",
            json!([{"namespace":"urn:one"},{"namespace":"urn:one"}]),
            false,
        ),
        ("<import/>", json!([{"namespace":null}]), true),
        ("<import namespace=\"\"/>", json!([{"namespace":""}]), true),
        (
            "<import namespace=\"urn:one\" future=\"x\"/>",
            json!([{"namespace":"urn:one"}]),
            true,
        ),
        (
            "<import namespace=\"urn:one\"><Nested/></import>",
            json!([{"namespace":"urn:one"}]),
            true,
        ),
        (
            "<foreign:import xmlns:foreign=\"urn:foreign\" namespace=\"urn:unproved\"/>",
            json!([]),
            true,
        ),
        (
            "<import namespace=\"urn:one\"/><Future/>",
            json!([{"namespace":"urn:one"}]),
            true,
        ),
        (
            "<objectType name=\"Broken\" base=\"undefined:Missing\"/>",
            json!([]),
            false,
        ),
        (
            "<import namespace=\"urn:one\"/><objectType name=\"Broken\"><Future/></objectType>",
            json!([{"namespace":"urn:one"}]),
            false,
        ),
    ] {
        let fixture = RealReaderFixture::new();
        install(&fixture, "ImportProbe", content);
        let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
        let service = fixture.view_service();
        for suffix in ["", ".Import"] {
            let result = service
                .view(ViewRequest::new(&format!("main:XDTOPackage.ImportProbe{suffix}")).unwrap());
            assert!(result.ok, "{content}: {:?}", refusal_codes(&result));
            let data = result.data.unwrap();
            assert_eq!(
                data["limits"]
                    .as_array()
                    .is_some_and(|limits| !limits.is_empty()),
                limited,
                "{content}: {suffix}"
            );
            if suffix == ".Import" {
                assert_eq!(data["items"], expected);
            } else {
                assert!(data["branches"].as_array().unwrap().contains(&json!({"at":"main:XDTOPackage.ImportProbe.Import","count":expected.as_array().unwrap().len()})));
            }
        }
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.source),
            before
        );
    }
}

#[test]
fn xdto_import_pages_preserve_large_lists_and_one_indivisible_uri_without_addresses() {
    let fixture = RealReaderFixture::new();
    let mut uris = (0..350)
        .map(|index| format!("urn:imports:{index}"))
        .collect::<Vec<_>>();
    uris.push(format!("urn:{}", "x".repeat(70000)));
    let content = uris
        .iter()
        .map(|uri| format!("<import namespace=\"{uri}\"/>"))
        .collect::<String>();
    install(&fixture, "LargeImports", &content);
    let service = fixture.view_service();
    let mut cursor = None;
    let mut observed = Vec::new();
    loop {
        let mut request = ViewRequest::new("main:XDTOPackage.LargeImports.Import")
            .unwrap()
            .with_limit(50)
            .unwrap();
        if let Some(value) = cursor.take() {
            request = request.with_cursor(value);
        }
        let page = service.view(request);
        assert!(page.ok, "{:?}", refusal_codes(&page));
        let items = page.data.unwrap()["items"].as_array().unwrap().clone();
        for item in items {
            assert_eq!(item.as_object().unwrap().len(), 1);
            observed.push(item["namespace"].as_str().unwrap().to_string());
        }
        cursor = page.cursor;
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(observed, uris);
}

#[test]
fn xdto_imports_keep_expanded_qnames_stable_when_prefix_aliases_change() {
    let fixture = RealReaderFixture::new();
    for (name, prefix) in [("AliasA", "a"), ("AliasB", "different")] {
        install(&fixture,name,&format!("<import namespace=\"urn:external\"/><import namespace=\"urn:unused\"/><objectType xmlns:{prefix}=\"urn:external\" name=\"Probe\" base=\"{prefix}:Base\"/>"));
    }
    let service = fixture.view_service();
    let mut semantic = Vec::new();
    for name in ["AliasA", "AliasB"] {
        let imports =
            service.view(ViewRequest::new(&format!("main:XDTOPackage.{name}.Import")).unwrap());
        assert!(imports.ok);
        let ty =
            service.view(ViewRequest::new(&format!("main:XDTOPackage.{name}.Type.Probe")).unwrap());
        assert!(ty.ok);
        let base = ty.data.unwrap()["props"]["base"].clone();
        semantic.push(json!({"imports":imports.data.unwrap()["items"],"namespace":base["namespace"],"local":base["local"]}));
    }
    assert_eq!(semantic[0], semantic[1]);
    assert_eq!(semantic[0]["local"], "Base");
}

#[test]
fn xdto_imports_are_anonymous_and_do_not_gain_write_sites_or_child_nodes() {
    use unica_xdto::operations::XdtoSite;
    let fixture = RealReaderFixture::new();
    install(
        &fixture,
        "ReadOnlyImports",
        "<import namespace=\"urn:external\"/>",
    );
    let service = fixture.view_service();
    let branch = QualifiedAddress::parse("main:XDTOPackage.ReadOnlyImports.Import").unwrap();
    for site in [
        XdtoSite::XdtoPackage,
        XdtoSite::XdtoType,
        XdtoSite::XdtoProperty,
        XdtoSite::XdtoPropertyContainer,
    ] {
        assert!(!site.matches(&branch));
    }
    assert!(QualifiedAddress::parse("main:XDTOPackage.ReadOnlyImports.Import.I1").is_err());
    for suffix in [
        ".Import.Type.Probe",
        ".Import.Props.namespace",
        ".Import.Import",
    ] {
        let result = service
            .view(ViewRequest::new(&format!("main:XDTOPackage.ReadOnlyImports{suffix}")).unwrap());
        assert!(!result.ok);
    }
}
