use super::*;

const V8_NS: &str = "http://v8.1c.ru/8.1/data/core";

pub(super) fn native_additional_relations(
    native: &mut NativeProjections<'_, '_>,
    source: &Path,
    registrations: &[(String, String)],
    version: &str,
    line: PlatformLine,
) {
    let catalog = "Catalog.Валюты";
    let path = source.join("Catalogs/Валюты.xml");
    assert_registered(registrations, catalog);
    let xml = read_fixture_descriptor(&path);
    let document = parse_fixture_descriptor(&xml, &path);
    let properties = fixture_properties(&document, "Catalog", "Валюты", &path);
    let expected = relation_inputs(properties, "InputByString", "Field");
    assert_eq!(expected.len(), 2, "{path:?}");
    let branch = relation(native.authority, catalog);
    let rows = relation_rows(&branch, "inputByString");
    assert_eq!(rows.len(), expected.len());
    for (row, field) in rows.into_iter().zip(expected) {
        assert!(field.starts_with("Catalog.Валюты.StandardAttribute."));
        assert_target(native.authority, row, field, "StandardAttribute");
    }
    record(native, catalog, &path);

    event_subscription(native, source, registrations);
    subsystem_backrefs(native, source, registrations);

    let source = fixture_path(&format!("formats/platform_xml/{version}/recalculation/src"));
    let registrations = root_registrations(&source, version);
    let calculation = "ChartOfCalculationTypes.ВидыРасчета";
    assert_registered(&registrations, calculation);
    let path = source.join("ChartsOfCalculationTypes/ВидыРасчета.xml");
    let xml = read_fixture_descriptor(&path);
    let document = parse_fixture_descriptor(&xml, &path);
    let properties = fixture_properties(&document, "ChartOfCalculationTypes", "ВидыРасчета", &path);
    let expected = relation_inputs(properties, "BaseCalculationTypes", "Item");
    assert_eq!(expected, [calculation], "{path:?}");
    let cancellation = CancellationToken::new();
    let authority = native_authority(
        &cancellation,
        &source,
        PlatformProfile::for_line(line).unwrap(),
    );
    let branch = relation(&authority, calculation);
    let rows = relation_rows(&branch, "baseCalculationType");
    assert_eq!(rows.len(), expected.len());
    for (row, target) in rows.into_iter().zip(expected) {
        assert_target(&authority, row, target, "ChartOfCalculationTypes");
    }
    record(
        &mut NativeProjections {
            authority: &authority,
            evidence: &mut *native.evidence,
            required: &mut *native.required,
        },
        calculation,
        &path,
    );
}

fn event_subscription(
    native: &mut NativeProjections<'_, '_>,
    source: &Path,
    registrations: &[(String, String)],
) {
    let subscription = "EventSubscription.CorpusEventSubscription";
    assert_registered(registrations, subscription);
    let path = source.join("EventSubscriptions/CorpusEventSubscription.xml");
    let xml = read_fixture_descriptor(&path);
    let document = parse_fixture_descriptor(&xml, &path);
    let properties = fixture_properties(
        &document,
        "EventSubscription",
        "CorpusEventSubscription",
        &path,
    );
    let source_type = child(child(properties, MD_CLASSES_NS, "Source"), V8_NS, "Type");
    let catalog_name = source_type
        .text()
        .unwrap()
        .strip_prefix("cfg:CatalogObject.")
        .unwrap();
    let catalog = format!("Catalog.{catalog_name}");
    assert_registered(registrations, &catalog);
    let handler = child(properties, MD_CLASSES_NS, "Handler").text().unwrap();
    let (module, method) = handler.rsplit_once('.').unwrap();
    assert_registered(registrations, module);

    let branch = relation(native.authority, subscription);
    let sources = relation_rows(&branch, "source");
    assert_eq!(sources.len(), 1);
    assert_eq!(
        sources[0]["target"],
        json!({"kind":"object", "metadataPath":catalog})
    );
    assert_target(native.authority, sources[0], &catalog, "Catalog");
    let handlers = relation_rows(&branch, "handler");
    assert_eq!(handlers.len(), 1);
    assert_target(
        native.authority,
        handlers[0],
        &format!("{module}.Method.{method}"),
        "Method",
    );
    check_native_projection_refusal(
        native.authority,
        &format!("main:{subscription}.Relation.Invalid"),
        "the relation branch lists targets; read the target at its own address",
    );
    record(native, subscription, &path);
}

fn subsystem_backrefs(
    native: &mut NativeProjections<'_, '_>,
    source: &Path,
    registrations: &[(String, String)],
) {
    let root = "СтандартныеПодсистемы";
    let nested = "БазоваяФункциональность";
    let owner = "CommonModule.EventHandlers";
    assert_registered(registrations, &format!("Subsystem.{root}"));
    assert_registered(registrations, owner);

    let root_path = source.join(format!("Subsystems/{root}.xml"));
    let root_xml = read_fixture_descriptor(&root_path);
    let root_document = parse_fixture_descriptor(&root_xml, &root_path);
    let root_artifact = fixture_artifact(&root_document, "Subsystem", &root_path);
    assert_eq!(fixture_properties_name(root_artifact), Some(root));
    let registered = child(
        child(root_artifact, MD_CLASSES_NS, "ChildObjects"),
        MD_CLASSES_NS,
        "Subsystem",
    );
    assert_eq!(registered.text(), Some(nested));
    let nested_path = source.join(format!("Subsystems/{root}/Subsystems/{nested}.xml"));
    let nested_xml = read_fixture_descriptor(&nested_path);
    let nested_document = parse_fixture_descriptor(&nested_xml, &nested_path);
    let nested_properties = fixture_properties(&nested_document, "Subsystem", nested, &nested_path);
    for properties in [
        child(root_artifact, MD_CLASSES_NS, "Properties"),
        nested_properties,
    ] {
        assert_eq!(relation_inputs(properties, "Content", "Item"), [owner]);
    }

    let branch = relation(native.authority, owner);
    let rows = relation_rows(&branch, "subsystem");
    let expected = [
        format!("Subsystem.{root}"),
        format!("Subsystem.{root}.Subsystem.{nested}"),
    ];
    assert_eq!(rows.len(), expected.len());
    for target in expected {
        let row = rows
            .iter()
            .find(|row| row["at"] == format!("main:{target}"))
            .unwrap();
        assert_eq!(row["direct"], true);
        assert_target(native.authority, row, &target, "Subsystem");
    }
    for path in [&root_path, &nested_path] {
        record(native, owner, path);
    }
}

fn fixture_properties<'a, 'input: 'a>(
    document: &'a roxmltree::Document<'input>,
    kind: &str,
    name: &str,
    path: &Path,
) -> roxmltree::Node<'a, 'input> {
    let artifact = fixture_artifact(document, kind, path);
    assert_eq!(fixture_properties_name(artifact), Some(name), "{path:?}");
    child(artifact, MD_CLASSES_NS, "Properties")
}

fn assert_registered(registrations: &[(String, String)], address: &str) {
    let (kind, name) = address.split_once('.').unwrap();
    assert!(
        registrations
            .iter()
            .any(|(tag, registration)| tag == kind && registration == name),
        "{address}"
    );
}

fn relation_inputs<'a, 'input>(
    properties: roxmltree::Node<'a, 'input>,
    container: &str,
    tag: &str,
) -> Vec<&'a str> {
    child(properties, MD_CLASSES_NS, container)
        .children()
        .filter(roxmltree::Node::is_element)
        .map(|item| {
            assert!(item.has_tag_name((READABLE_NS, tag)));
            item.text().unwrap()
        })
        .collect()
}

fn relation(authority: &LogicalViewReadAuthority<'_>, owner: &str) -> Value {
    let branch = format!("main:{owner}.Relation");
    let result = read(authority, &branch);
    assert_eq!(result["kind"], "Relation", "{branch}");
    assert_eq!(result["at"], branch);
    result
}

fn relation_rows<'a>(branch: &'a Value, name: &str) -> Vec<&'a Value> {
    branch["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["relation"] == name)
        .collect()
}

fn assert_target(authority: &LogicalViewReadAuthority<'_>, row: &Value, target: &str, kind: &str) {
    let target = format!("main:{target}");
    assert_eq!(row["at"], target);
    assert_eq!(row["kind"], kind);
    assert_eq!(read(authority, &target)["kind"], kind, "{target}");
}

fn record(native: &mut NativeProjections<'_, '_>, owner: &str, path: &Path) {
    native.record(
        &format!("main:{owner}"),
        "Relation",
        &fixture_origin(path),
        BTreeSet::from([format!("main:{owner}.Relation")]),
    );
}
