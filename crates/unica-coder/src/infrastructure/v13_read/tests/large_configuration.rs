use super::*;

#[test]
fn ordinary_xml_read_over_eight_mib_keeps_complete_configuration_facts() {
    let fixture = RealReaderFixture::new();
    let path = fixture.source.join("Configuration.xml");
    let baseline = fixture
        .view_service()
        .view(ViewRequest::new("main:Configuration").unwrap());
    assert!(baseline.ok, "baseline Configuration view failed");
    let xml = fs::read_to_string(&path).unwrap();
    let inflated = xml.replace(
        "</MetaDataObject>",
        &format!("<!--{}--></MetaDataObject>", "x".repeat(8 * 1024 * 1024)),
    );
    fs::write(path, inflated).unwrap();

    let result = fixture
        .view_service()
        .view(ViewRequest::new("main:Configuration").unwrap());
    assert!(result.ok, "large Configuration view failed");
    assert!(
        result.data.as_ref().unwrap()["props"] == baseline.data.as_ref().unwrap()["props"],
        "Configuration root properties changed"
    );
    assert!(
        result.data.as_ref().unwrap()["branches"] == baseline.data.as_ref().unwrap()["branches"],
        "Configuration root branches changed"
    );
}

#[test]
fn large_configuration_root_keeps_legacy_duplicate_and_namespace_semantics() {
    let fixture = RealReaderFixture::new();
    let path = fixture.source.join("Configuration.xml");
    let xml = fs::read_to_string(&path).unwrap();
    let xml = xml.replacen(
        "</ChildObjects>",
        "<CommonModule>РеактивныйСервер</CommonModule><alien:Catalog xmlns:alien=\"urn:other\">Ghost</alien:Catalog></ChildObjects>",
        1,
    );
    fs::write(&path, &xml).unwrap();

    let reader = fixture.read_authority();
    let legacy_payload = reader
        .read
        .configuration_payload_with_checkpoint(&mut || Ok(()))
        .unwrap();
    let route = route_logical_address(
        &QualifiedAddress::parse("main:Configuration").unwrap(),
        PlatformProfile::v8_3_27(),
    )
    .unwrap();
    let legacy = project_typed_payload(&route, legacy_payload.as_ref().clone()).unwrap();
    let module_branch = super::super::module_branch_for_parent(
        route.at(),
        PlatformProfile::v8_3_27(),
        SourceSetKind::Configuration,
    )
    .expect("Configuration has a profile module branch");
    let expected = serde_json::to_value(legacy.with_branch(module_branch)).unwrap();

    fs::write(
        &path,
        xml.replace(
            "</MetaDataObject>",
            &format!("<!--{}--></MetaDataObject>", "x".repeat(8 * 1024 * 1024)),
        ),
    )
    .unwrap();
    let actual = fixture
        .view_service()
        .view(ViewRequest::new("main:Configuration").unwrap());
    assert!(actual.ok, "large Configuration view failed");
    assert!(
        actual.data.as_ref().unwrap() == &expected,
        "streaming Configuration root differs from the bounded reader"
    );
}

#[test]
fn root_kind_collection_follows_child_objects_order_in_both_readers() {
    let fixture = RealReaderFixture::new();
    let path = fixture.source.join("Configuration.xml");
    let xml = fs::read_to_string(&path).unwrap();
    // `Владельцы` sorts after `Items` by code points but stands first here.
    let xml = xml.replacen("<Catalog>Items</Catalog>", "", 1).replacen(
        "<Catalog>Владельцы</Catalog>",
        "<Catalog>Владельцы</Catalog><Catalog>Items</Catalog>",
        1,
    );
    fs::write(&path, &xml).unwrap();
    let names = |fixture: &RealReaderFixture| {
        let result = fixture
            .view_service()
            .view(ViewRequest::new("main:Catalog").unwrap());
        assert!(result.ok, "Catalog collection failed");
        result.data.unwrap()["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["title"].as_str().unwrap().to_string())
            .collect::<Vec<_>>()
    };
    let bounded = names(&fixture);
    assert_eq!(bounded, ["Владельцы", "Items"]);
    fs::write(
        &path,
        xml.replace(
            "</MetaDataObject>",
            &format!("<!--{}--></MetaDataObject>", "x".repeat(8 * 1024 * 1024)),
        ),
    )
    .unwrap();
    assert_eq!(
        names(&fixture),
        bounded,
        "streaming collection order differs"
    );
}

#[test]
fn large_configuration_root_rejects_malformed_tail() {
    let fixture = RealReaderFixture::new();
    let path = fixture.source.join("Configuration.xml");
    let xml = fs::read_to_string(&path).unwrap();
    fs::write(
        path,
        xml.replace(
            "</MetaDataObject>",
            &format!(
                "<!--{}--><Broken></MetaDataObject>",
                "x".repeat(8 * 1024 * 1024)
            ),
        ),
    )
    .unwrap();
    let result = fixture
        .view_service()
        .view(ViewRequest::new("main:Configuration").unwrap());
    assert!(!result.ok, "malformed XML tail escaped");
    assert_eq!(result.diagnostics[0]["code"], "provider_unavailable");
    assert_eq!(result.diagnostics[0]["detailCode"], "source_unreadable");
}

/// #1119: a correct descriptor larger than the former 8 MiB read limit is
/// read through the public view instead of refused by its size.
/// rule: INV.SOURCE.LARGE-CONFIGURATION-ROOT-VIEW
#[test]
fn a_descriptor_larger_than_eight_mebibytes_is_read_through_public_view() {
    let fixture = RealReaderFixture::new();
    let relative = fixture
        .source
        .read_dir()
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.ends_with("Catalogs"))
        .expect("the real fixture has catalogs");
    let descriptor = fs::read_dir(&relative)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.extension().is_some_and(|extension| extension == "xml"))
        .unwrap();
    let name = descriptor
        .file_stem()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let original = fs::read_to_string(&descriptor).unwrap();
    let padding = format!("<!--{}-->", "x".repeat(9 * 1024 * 1024));
    let at = original.find("<MetaDataObject").unwrap();
    let mut large = original.clone();
    large.insert_str(at, &padding);
    fs::write(&descriptor, &large).unwrap();
    assert!(large.len() > 8 * 1024 * 1024);
    let result = fixture
        .view_service()
        .view(ViewRequest::new(&format!("main:Catalog.{name}")).unwrap());
    assert!(result.ok);
    assert_eq!(result.data.unwrap()["at"], format!("main:Catalog.{name}"));
}
