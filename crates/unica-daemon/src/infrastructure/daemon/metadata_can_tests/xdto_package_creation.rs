use super::*;

const PACKAGE_AT: &str = "main:XDTOPackage.EmptyPackage";
const DESCRIPTOR: &str = "XDTOPackages/EmptyPackage.xml";
const BODY: &str = "XDTOPackages/EmptyPackage/Ext/Package.bin";
const NAMESPACE: &str = "https://example.invalid/xdto?mode=empty&v=1";

fn create(target: &str) -> Value {
    json!({"at":target,"ops":[{"op":"object.create","args":{
        "values":{"kind":"XDTOPackage","name":"EmptyPackage","namespace":NAMESPACE}
    }}]})
}

#[test]
fn xdto_package_creation_has_a_closed_namespace_contract() {
    let fixture = Fixture::blank();
    let runtime = runtime();
    let at = "main:Configuration";
    let can = contract(&runtime, &fixture, at, "object.create");
    let args = |values: Value| json!({"values":values});
    let metadata_type = json!({"variants":[{"kind":"boolean"}]});
    let cases = [
        (
            args(json!({"kind":"XDTOPackage","name":"Probe","namespace":NAMESPACE})),
            true,
        ),
        (
            args(json!({"kind":"XDTOPackage","name":"Probe","namespace":"urn:unica:empty"})),
            true,
        ),
        (args(json!({"kind":"XDTOPackage","name":"Probe"})), false),
        (
            args(json!({"kind":"XDTOPackage","name":"Probe","namespace":null})),
            false,
        ),
        (
            args(json!({"kind":"XDTOPackage","name":"Probe","namespace":"relative/path"})),
            false,
        ),
        (
            args(json!({"kind":"XDTOPackage","name":"Probe","namespace":"urn: bad"})),
            false,
        ),
        (
            args(json!({"kind":"XDTOPackage","name":"Probe","namespace":"urn:x%ZZ"})),
            false,
        ),
        (
            args(
                json!({"kind":"XDTOPackage","name":"Probe","namespace":NAMESPACE,"type":metadata_type}),
            ),
            false,
        ),
        (
            args(json!({"kind":"XDTOPackage","name":"Probe","namespace":NAMESPACE,"types":[]})),
            false,
        ),
        (
            args(json!({"kind":"FilterCriterion","name":"Probe","type":metadata_type})),
            true,
        ),
        (
            args(
                json!({"kind":"FilterCriterion","name":"Probe","type":metadata_type,"namespace":NAMESPACE}),
            ),
            false,
        ),
        (args(json!({"kind":"CommonForm","name":"Probe"})), true),
        (
            args(json!({"kind":"CommonForm","name":"Probe","namespace":NAMESPACE})),
            false,
        ),
    ];
    assert_parity(
        &can["argsSchema"],
        &fixture.binding(false),
        at,
        "object.create",
        &cases,
    );
    let raw_contract = unica_metadata::contracts::metadata_object_create();
    for (request, expected) in cases {
        assert_eq!(
            raw_contract.normalize(&request, "args").is_ok(),
            expected,
            "{request}"
        );
    }
}

#[test]
fn xdto_package_creation_publishes_native_files_and_allows_first_type() {
    for format in ["2.20", "2.21", "2.22"] {
        let fixture = Fixture::blank();
        for relative in ["Configuration.xml", "Languages/Русский.xml"] {
            let path = fixture.source.join(relative);
            let xml = std::fs::read_to_string(&path).unwrap();
            std::fs::write(
                path,
                xml.replace("version=\"2.20\"", &format!("version=\"{format}\"")),
            )
            .unwrap();
        }
        let runtime = runtime();
        let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
        publish(&runtime, &fixture, create("main:Configuration"));
        let created = unica_workspace::test_support::tree_snapshot(&fixture.source);
        assert_eq!(
            changed_paths(&before, &created),
            (
                BTreeSet::from([PathBuf::from(DESCRIPTOR), PathBuf::from(BODY)]),
                BTreeSet::from([PathBuf::from("Configuration.xml")]),
                BTreeSet::new()
            ),
            "{format}"
        );
        let text = |path: &str| std::str::from_utf8(&created[Path::new(path)]).unwrap();
        assert!(text("Configuration.xml").contains("<XDTOPackage>EmptyPackage</XDTOPackage>"));
        let descriptor =
            roxmltree::Document::parse(text(DESCRIPTOR).trim_start_matches('\u{feff}')).unwrap();
        assert_eq!(descriptor.root_element().attribute("version"), Some(format));
        assert_eq!(
            descriptor
                .descendants()
                .find(|n| n.has_tag_name("Namespace"))
                .and_then(|n| n.text()),
            Some(NAMESPACE)
        );
        assert_eq!(
            text(BODY),
            concat!(
                "\u{feff}<package xmlns=\"http://v8.1c.ru/8.1/xdto\" ",
                "xmlns:xs=\"http://www.w3.org/2001/XMLSchema\" ",
                "xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" ",
                "targetNamespace=\"https://example.invalid/xdto?mode=empty&amp;v=1\">\r\n",
                "</package>\r\n",
            )
        );
        let body = roxmltree::Document::parse(text(BODY).trim_start_matches('\u{feff}')).unwrap();
        assert_eq!(
            body.root_element().attribute("targetNamespace"),
            Some(NAMESPACE)
        );
        assert!(!body.root_element().children().any(|node| node.is_element()));
        assert_eq!(view(&runtime, &fixture, PACKAGE_AT)["at"], PACKAGE_AT);

        contract(&runtime, &fixture, PACKAGE_AT, "valueType.add");
        publish(
            &runtime,
            &fixture,
            json!({"at":PACKAGE_AT,"ops":[{"op":"valueType.add","args":{
                "values":{"name":"FirstValue","base":"xs:string"}
            }}]}),
        );
        let with_type = unica_workspace::test_support::tree_snapshot(&fixture.source);
        assert_eq!(
            changed_paths(&created, &with_type),
            (
                BTreeSet::new(),
                BTreeSet::from([PathBuf::from(BODY)]),
                BTreeSet::new()
            )
        );
        let module = std::str::from_utf8(&with_type[Path::new(BODY)]).unwrap();
        assert!(module.starts_with('\u{feff}'));
        assert!(module.contains("name=\"FirstValue\""));
        assert!(module.contains("base=\"xs:string\""));
        assert!(module.ends_with("</package>\r\n"));
    }
}

#[test]
fn xdto_package_creation_refuses_an_extension_and_resource_collisions() {
    let runtime = runtime();
    let extension = Fixture::with_extension();
    let ext_source = extension.workspace.join("src-ext");
    let ext_before = unica_workspace::test_support::tree_snapshot(&ext_source);
    let can = contract(&runtime, &extension, "ext:Configuration", "object.create");
    let kinds = can["argsSchema"]["properties"]["values"]["properties"]["kind"]["enum"]
        .as_array()
        .unwrap();
    assert!(!kinds.contains(&json!("XDTOPackage")));
    let refused = call(
        &runtime,
        &extension,
        ToolIdentity::Apply,
        create("ext:Configuration"),
    );
    assert!(!refused.ok, "{refused:?}");
    assert_eq!(refused.at.as_deref(), Some("ops[0].args.values.kind"));
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&ext_source),
        ext_before
    );

    let fixture = Fixture::blank();
    let original = unica_workspace::test_support::tree_snapshot(&fixture.source);
    let request = create("main:Configuration");
    let preview = call(&runtime, &fixture, ToolIdentity::Apply, request.clone());
    assert!(preview.ok, "{preview:?}");
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&fixture.source),
        original
    );
    let package = fixture.source.join(BODY);
    std::fs::create_dir_all(package.parent().unwrap()).unwrap();
    std::fs::write(package, b"existing package").unwrap();
    let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
    for attempt in [
        json!({"executionToken":preview.data.unwrap()["executionToken"]}),
        request,
    ] {
        let refused = call(&runtime, &fixture, ToolIdentity::Apply, attempt);
        assert!(!refused.ok, "{refused:?}");
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.source),
            before
        );
    }
}
