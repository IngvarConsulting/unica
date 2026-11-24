use super::*;

const V8_NS: &str = "http://v8.1c.ru/8.1/data/core";

pub(super) fn native_additional_relations(
    native: &mut NativeProjections<'_, '_>,
    source: &Path,
    registrations: &[(String, String)],
    document_relation_owners: &BTreeSet<String>,
    version: &str,
    line: PlatformLine,
) {
    input_by_string(native, source, registrations, document_relation_owners);
    if version == "2.20" {
        data_lock_fields(native, line);
    }

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

fn input_by_string(
    native: &mut NativeProjections<'_, '_>,
    source: &Path,
    registrations: &[(String, String)],
    document_relation_owners: &BTreeSet<String>,
) {
    let document_attributes =
        check_explicit_document_attributes(native.authority, source, registrations);
    for (kind, name) in registrations {
        let (_, declaration) = unica_address::kinds::declared(kind).unwrap();
        let Some(layout) = declaration.layout else {
            continue;
        };
        let path = source.join(layout.directory).join(format!("{name}.xml"));
        let xml = read_fixture_descriptor(&path);
        let document = parse_fixture_descriptor(&xml, &path);
        let properties = fixture_properties(&document, kind, name, &path);
        let Some(container) = properties
            .children()
            .find(|node| node.has_tag_name((MD_CLASSES_NS, "InputByString")))
        else {
            continue;
        };
        let expected = relation_entries(container, "Field");
        if expected.is_empty() {
            continue;
        }

        let owner = format!("{kind}.{name}");
        let branch = relation(native.authority, &owner);
        let rows = relation_rows(&branch, "inputByString");
        assert_eq!(rows.len(), expected.len(), "{owner}");
        for (row, field) in rows.into_iter().zip(expected) {
            let qualified = format!("main:{field}");
            let address = QualifiedAddress::parse(&qualified).unwrap();
            let field_kind = address.segments().last().unwrap().kind().as_str();
            assert_eq!(address.to_string(), qualified);
            assert_target(native.authority, row, field, field_kind);
            if field == "Document.Orders.StandardAttribute.Number" {
                assert!(!properties
                    .children()
                    .any(|node| node.has_tag_name((MD_CLASSES_NS, "StandardAttributes"))));
                let at = "main:Document.Orders";
                let collection = format!("{at}.StandardAttribute");
                let kind = "StandardAttribute";
                let members =
                    named_addresses(at, kind, document_attributes.iter().map(String::as_str));
                assert!(members.contains(&qualified), "{qualified}");
                for member in &members {
                    let view = read(native.authority, member);
                    assert_eq!(view["kind"], kind);
                    assert!(
                        view["props"]
                            .as_object()
                            .is_none_or(serde_json::Map::is_empty),
                        "{member}: {view}"
                    );
                }
                check_native_projection_refusal(
                    native.authority,
                    &format!("{collection}.Unregistered"),
                    "was not found",
                );
                native.collection(at, kind, &fixture_origin(&path), members);
            }
        }
        // RegisterRecords and InputByString in the same descriptor prove one Relation address.
        if !document_relation_owners.contains(&format!("main:{owner}")) {
            record(native, &owner, &path);
        }
    }
}

fn check_explicit_document_attributes(
    authority: &LogicalViewReadAuthority<'_>,
    source: &Path,
    registrations: &[(String, String)],
) -> Vec<String> {
    let name = "Приход";
    assert_registered(registrations, &format!("Document.{name}"));
    let path = source.join(format!("Documents/{name}.xml"));
    let raw = read_fixture_descriptor(&path);
    let xml = parse_fixture_descriptor(&raw, &path);
    let properties = fixture_properties(&xml, "Document", name, &path);
    let standard = child(properties, MD_CLASSES_NS, "StandardAttributes");
    let collection = format!("main:Document.{name}.StandardAttribute");
    let expected = native_named_declaration_members(standard, &collection, "StandardAttribute");
    assert!(expected.contains(&format!("{collection}.Number")));
    assert!(expected.contains(&format!("{collection}.Date")));
    check_native_branch(
        authority,
        &read(authority, &format!("main:Document.{name}")),
        &collection,
        &expected,
    );
    let prefix = format!("{collection}.");
    expected
        .into_iter()
        .map(|address| {
            assert_eq!(read(authority, &address)["kind"], "StandardAttribute");
            address.strip_prefix(&prefix).unwrap().to_owned()
        })
        .collect()
}

fn data_lock_fields(native: &mut NativeProjections<'_, '_>, line: PlatformLine) {
    let source = fixture_path("acceptance/workspace-object-data-lock/src");
    let owner = "Catalog.Codes";
    assert_registered(&root_registrations(&source, "2.20"), owner);
    let path = source.join("Catalogs/Codes.xml");
    let xml = read_fixture_descriptor(&path);
    let document = parse_fixture_descriptor(&xml, &path);
    let properties = fixture_properties(&document, "Catalog", "Codes", &path);
    let expected = relation_inputs(properties, "DataLockFields", "Field");
    assert_eq!(expected, ["Catalog.Codes.Attribute.Key"], "{path:?}");
    let cancellation = CancellationToken::new();
    let authority = native_authority(
        &cancellation,
        &source,
        PlatformProfile::for_line(line).unwrap(),
    );
    let branch = relation(&authority, owner);
    let rows = relation_rows(&branch, "dataLockField");
    assert_eq!(rows.len(), expected.len());
    for (row, field) in rows.into_iter().zip(expected) {
        assert_target(&authority, row, field, "Attribute");
    }
    record(
        &mut NativeProjections {
            authority: &authority,
            evidence: &mut *native.evidence,
            required: &mut *native.required,
        },
        owner,
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
    relation_entries(child(properties, MD_CLASSES_NS, container), tag)
}

fn relation_entries<'a, 'input>(container: roxmltree::Node<'a, 'input>, tag: &str) -> Vec<&'a str> {
    container
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
