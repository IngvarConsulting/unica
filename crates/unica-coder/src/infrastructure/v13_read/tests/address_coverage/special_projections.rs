use super::*;

const SHARD: &str = "native_special_projection_addresses_match_source";
const FORMS: &str = "native_form_and_template_addresses_follow_text_registrations";
const MODULES: &str = "native_registered_modules_match_profile_and_physical_paths";
const READABLE_NS: &str = "http://v8.1c.ru/8.3/xcf/readable";
const PREDEFINED_NS: &str = "http://v8.1c.ru/8.3/xcf/predef";

struct RequiredProjections {
    output: Option<PathBuf>,
    rows: BTreeMap<String, Value>,
}

impl RequiredProjections {
    fn new() -> Self {
        Self {
            output: std::env::var_os(EVIDENCE_DIRECTORY).map(|directory| {
                PathBuf::from(directory).join("required_native_special_projection_keys.jsonl")
            }),
            rows: BTreeMap::new(),
        }
    }

    fn read(
        &mut self,
        source: (PlatformLine, SourceSetKind),
        owner: &str,
        kind: &str,
        origin: &str,
        addresses: BTreeSet<String>,
        test_case: &str,
    ) {
        let owner_address = QualifiedAddress::parse(owner).unwrap();
        let key = address_key(source, &owner_address, kind, None);
        let observation = json!({
            "owner": owner, "origin": origin, "addresses": addresses,
            "level": "read", "test_case": test_case,
        });
        let row = self.rows.entry(key.to_string()).or_insert_with(|| {
            json!({
                "schema_version": 1, "record_type": "required", "key": key,
                "requirement": {"kind": "exact", "observations": []},
            })
        });
        let observations = row["requirement"]["observations"].as_array_mut().unwrap();
        assert!(
            !observations.contains(&observation),
            "duplicate {observation}"
        );
        observations.push(observation);
    }

    fn finish(self) {
        if let Some(path) = self.output {
            use std::io::Write;
            let mut output = fs::File::create_new(&path).unwrap();
            for row in self.rows.into_values() {
                writeln!(output, "{row}").unwrap();
            }
        }
    }
}

fn child<'a, 'input>(
    owner: roxmltree::Node<'a, 'input>,
    namespace: &str,
    name: &str,
) -> roxmltree::Node<'a, 'input> {
    owner
        .children()
        .find(|node| node.has_tag_name((namespace, name)))
        .unwrap_or_else(|| panic!("{owner:?}: missing {name}"))
}

fn named_addresses<'a>(
    owner: &str,
    kind: &str,
    names: impl IntoIterator<Item = &'a str>,
) -> BTreeSet<String> {
    let branch = QualifiedAddress::parse(&format!("{owner}.{kind}")).unwrap();
    let mut addresses = BTreeSet::new();
    for name in names {
        let address = branch.member(name).unwrap().to_string();
        assert!(addresses.insert(address), "{branch}: duplicate {name}");
    }
    addresses
}

struct NativeProjections<'a, 'b> {
    authority: &'a LogicalViewReadAuthority<'b>,
    evidence: &'a mut EvidenceRows,
    required: &'a mut RequiredProjections,
}

impl NativeProjections<'_, '_> {
    fn collection(&mut self, owner: &str, kind: &str, origin: &str, mut members: BTreeSet<String>) {
        let branch = format!("{owner}.{kind}");
        check_native_branch(
            self.authority,
            &read(self.authority, owner),
            &branch,
            &members,
        );
        for address in &members {
            assert_eq!(read(self.authority, address)["kind"], kind, "{address}");
        }
        assert!(members.insert(branch));
        self.record(owner, kind, origin, members);
    }

    fn record(&mut self, owner: &str, kind: &str, origin: &str, addresses: BTreeSet<String>) {
        let source = (
            self.authority.platform_profile().line(),
            self.authority.read.source_set_kind(),
        );
        for address in &addresses {
            self.evidence.record_for_source(
                source,
                owner,
                kind,
                None,
                origin,
                AddressEvidence::Read(address),
            );
        }
        self.required
            .read(source, owner, kind, origin, addresses, SHARD);
    }
}

/// rule: CTR.SOURCE.QUALIFIED-LOGICAL-ADDRESS
/// rule: INV.SOURCE.DECLARATION-BRANCHES-FOLLOW-THE-NAME
/// rule: INV.SOURCE.PREDEFINED-ITEMS-ANSWER-BY-ADDRESS
/// rule: INV.SOURCE.MODULE-SUMMARY
/// rule: INV.SOURCE.MODULE-REGION-INTERFACE
#[test]
fn native_special_projection_addresses_match_source() {
    let mut evidence = EvidenceRows::new(SHARD, SourceSetKind::Configuration);
    let mut required = RequiredProjections::new();
    for version in ["2.20", "2.21", "2.22"] {
        let source = fixture_path(&format!("formats/platform_xml/{version}/all_kinds/src"));
        let line = export_format_line(version).unwrap();
        let registrations = root_registrations(&source, version);
        for (kind, name) in [
            ("Catalog", "Валюты"),
            ("ChartOfAccounts", "Хозрасчетный"),
            ("ChartOfCalculationTypes", "ВидыРасчета"),
            ("CommonForm", "MainA"),
        ] {
            assert!(registrations
                .iter()
                .any(|(tag, value)| tag == kind && value == name));
        }
        let cancellation = CancellationToken::new();
        let authority = native_authority(
            &cancellation,
            &source,
            PlatformProfile::for_line(line).unwrap(),
        );
        let mut native = NativeProjections {
            authority: &authority,
            evidence: &mut evidence,
            required: &mut required,
        };
        declarations(&mut native, &source);
        predefined(&mut native, &source, version);
        native_module_slices(&mut native, &source, line);
        let common_form = source.join("CommonForms/MainA/Ext/Form.xml");
        direct_form_command_and_parameter(&mut native, "main:CommonForm.MainA", &common_form);
        native_named_form_parameter(&mut evidence, &mut required, version, line);
        external_form_command_and_parameter(&mut evidence, &mut required, version, line);
        native_form_requirements(&mut required, &source, line, &registrations);
        native_module_requirements(&mut required, &source, line, &registrations);
    }
    nonempty_characteristics(&mut evidence, &mut required);
    evidence.finish();
    required.finish();
}

fn nonempty_characteristics(evidence: &mut EvidenceRows, required: &mut RequiredProjections) {
    let name = "АктОбУничтоженииПерсональныхДанных";
    let source = fixture_path("acceptance/workspace/src");
    assert!(root_registrations(&source, "2.20")
        .iter()
        .any(|(kind, item)| kind == "Document" && item == name));
    let path = source.join(format!("Documents/{name}.xml"));
    let raw = read_fixture_descriptor(&path);
    let xml = parse_fixture_descriptor(&raw, &path);
    let props = child(
        fixture_artifact(&xml, "Document", &path),
        MD_CLASSES_NS,
        "Properties",
    );
    let characteristics = child(props, MD_CLASSES_NS, "Characteristics");
    let expected: Vec<_> = characteristics
        .children()
        .filter(|node| node.has_tag_name((READABLE_NS, "Characteristic")))
        .map(|node| {
            let from = |tag| child(node, READABLE_NS, tag).attribute("from").unwrap();
            (from("CharacteristicTypes"), from("CharacteristicValues"))
        })
        .collect();
    assert!(!expected.is_empty());
    let cancellation = CancellationToken::new();
    let authority = native_authority(&cancellation, &source, PlatformProfile::v8_3_27());
    let owner = format!("main:Document.{name}");
    let branch = format!("{owner}.Characteristic");
    check_native_branch_count(&read(&authority, &owner), &branch, expected.len());
    let view = read(&authority, &branch);
    let rows = view["items"].as_array().unwrap();
    assert_eq!(rows.len(), expected.len());
    for (row, (types, values)) in rows.iter().zip(expected) {
        assert!(row.get("at").is_none(), "{branch}: {row}");
        assert_eq!(row["types"]["source"], types);
        assert_eq!(row["values"]["source"], values);
    }
    check_native_projection_refusal(
        &authority,
        &format!("{branch}.Any"),
        "a characteristic carries no applied name",
    );
    NativeProjections {
        authority: &authority,
        evidence,
        required,
    }
    .record(
        &owner,
        "Characteristic",
        &fixture_origin(&path),
        BTreeSet::from([branch]),
    );
}

fn declarations(native: &mut NativeProjections<'_, '_>, source: &Path) {
    let catalog = "main:Catalog.Валюты";
    let path = source.join("Catalogs/Валюты.xml");
    let raw = read_fixture_descriptor(&path);
    let xml = parse_fixture_descriptor(&raw, &path);
    let props = child(
        fixture_artifact(&xml, "Catalog", &path),
        MD_CLASSES_NS,
        "Properties",
    );
    let standards = child(props, MD_CLASSES_NS, "StandardAttributes");
    let origin = fixture_origin(&path);
    let branch = format!("{catalog}.StandardAttribute");
    let members = native_named_declaration_members(standards, &branch, "StandardAttribute");
    native.collection(catalog, "StandardAttribute", &origin, members);

    // The native declaration contains no characteristic rows. The collection
    // itself is readable, and even a nonempty row has no applied name.
    let characteristics = child(props, MD_CLASSES_NS, "Characteristics");
    assert!(!characteristics.children().any(|node| node.is_element()));
    let branch = format!("{catalog}.Characteristic");
    let collection = read(native.authority, &branch);
    assert_eq!(collection["kind"], "Characteristic");
    assert!(collection["items"].as_array().unwrap().is_empty());
    check_native_projection_refusal(
        native.authority,
        &format!("{branch}.Any"),
        "a characteristic carries no applied name",
    );
    native.record(catalog, "Characteristic", &origin, BTreeSet::from([branch]));

    let owner = "main:ChartOfAccounts.Хозрасчетный";
    let path = source.join("ChartsOfAccounts/Хозрасчетный.xml");
    let raw = read_fixture_descriptor(&path);
    let xml = parse_fixture_descriptor(&raw, &path);
    let props = child(
        fixture_artifact(&xml, "ChartOfAccounts", &path),
        MD_CLASSES_NS,
        "Properties",
    );
    let sections = child(props, MD_CLASSES_NS, "StandardTabularSections");
    let origin = fixture_origin(&path);
    let branch = format!("{owner}.StandardTabularSection");
    let members = native_named_declaration_members(sections, &branch, "StandardTabularSection");
    native.collection(owner, "StandardTabularSection", &origin, members);
    for section in sections.children().filter(roxmltree::Node::is_element) {
        let name = section.attribute("name").unwrap();
        let owner = QualifiedAddress::parse(&branch)
            .unwrap()
            .member(name)
            .unwrap()
            .to_string();
        let attributes = child(section, READABLE_NS, "StandardAttributes");
        let members = native_named_declaration_members(
            attributes,
            &format!("{owner}.StandardAttribute"),
            "StandardAttribute",
        );
        native.collection(&owner, "StandardAttribute", &origin, members);
    }
}

fn predefined(native: &mut NativeProjections<'_, '_>, source: &Path, version: &str) {
    let owner = "main:ChartOfCalculationTypes.ВидыРасчета";
    let path = source.join("ChartsOfCalculationTypes/ВидыРасчета/Ext/Predefined.xml");
    let raw = read_fixture_descriptor(&path);
    let xml = parse_fixture_descriptor(&raw, &path);
    let root = xml.root_element();
    assert!(root.has_tag_name((PREDEFINED_NS, "PredefinedData")));
    assert_eq!(root.attribute("version"), Some(version));
    let mut ids = BTreeMap::new();
    for item in root
        .descendants()
        .filter(|node| node.has_tag_name((PREDEFINED_NS, "Item")))
    {
        let name = child(item, PREDEFINED_NS, "Name").text().unwrap();
        let id = item.attribute("id").unwrap();
        assert!(!uuid::Uuid::parse_str(id).unwrap().is_nil());
        assert!(ids.insert(name, id).is_none(), "duplicate native {name}");
    }
    assert!(!ids.is_empty(), "{path:?}");
    let members = named_addresses(owner, "PredefinedItem", ids.keys().copied());
    native.collection(owner, "PredefinedItem", &fixture_origin(&path), members);
    for (name, id) in ids {
        let node = read(native.authority, &format!("{owner}.PredefinedItem.{name}"));
        assert_eq!(node["props"]["id"], id);
    }
}

fn direct_form_command_and_parameter(
    native: &mut NativeProjections<'_, '_>,
    owner: &str,
    path: &Path,
) {
    let raw = read_fixture_descriptor(path);
    let xml = parse_fixture_descriptor(&raw, path);
    let form = xml.root_element();
    assert!(form.has_tag_name((NATIVE_FORM_NS, "Form")));
    let origin = fixture_origin(path);
    for (container, kind) in [("Commands", "Command"), ("Parameters", "Parameter")] {
        let names = native_form_children(form, container)
            .flat_map(|node| native_form_children(node, kind))
            .map(|node| node.attribute("name").unwrap());
        let members = named_addresses(owner, kind, names);
        if members.is_empty() {
            let branch = format!("{owner}.{kind}");
            let view = read(native.authority, &branch);
            assert_eq!(view["kind"], kind);
            assert!(addresses(&view, "items").is_empty(), "{branch}");
            native.record(owner, kind, &origin, BTreeSet::from([branch]));
        } else {
            native.collection(owner, kind, &origin, members);
        }
    }
}

fn native_named_form_parameter(
    evidence: &mut EvidenceRows,
    required: &mut RequiredProjections,
    version: &str,
    line: PlatformLine,
) {
    const CORE: &str = "http://v8.1c.ru/8.1/data/core";
    let source = fixture_path(&format!(
        "acceptance/workspace-native-form-parameter/src-{version}"
    ));
    assert!(root_registrations(&source, version).contains(&("CommonForm".into(), "MainA".into())));
    let path = source.join("CommonForms/MainA/Ext/Form.xml");
    let raw = read_fixture_descriptor(&path);
    let xml = parse_fixture_descriptor(&raw, &path);
    let form = xml.root_element();
    assert!(form.has_tag_name((NATIVE_FORM_NS, "Form")));
    assert_eq!(form.attribute("version"), Some(version));
    let parameters: Vec<_> = native_form_children(form, "Parameters")
        .flat_map(|node| native_form_children(node, "Parameter"))
        .collect();
    let owner = "main:CommonForm.MainA";
    let names: Vec<_> = parameters
        .iter()
        .map(|parameter| parameter.attribute("name").unwrap())
        .collect();
    assert!(names.contains(&"AddressScope"));
    let cancellation = CancellationToken::new();
    let authority = native_authority(
        &cancellation,
        &source,
        PlatformProfile::for_line(line).unwrap(),
    );
    NativeProjections {
        authority: &authority,
        evidence,
        required,
    }
    .collection(
        owner,
        "Parameter",
        &fixture_origin(&path),
        named_addresses(owner, "Parameter", names),
    );
    for parameter in parameters {
        let name = parameter.attribute("name").unwrap();
        let type_element = child(parameter, NATIVE_FORM_NS, "Type");
        assert_eq!(child(type_element, CORE, "Type").text(), Some("xs:string"));
        let key = child(parameter, NATIVE_FORM_NS, "KeyParameter")
            .text()
            .unwrap()
            .parse::<bool>()
            .unwrap();
        let view = read(&authority, &format!("{owner}.Parameter.{name}"));
        assert_eq!(view["props"]["type"], "string", "{name}");
        assert_eq!(view["props"]["isKey"], key, "{name}");
    }
    check_native_projection_refusal(
        &authority,
        &format!("{owner}.Parameter.UnregisteredParameter"),
        "was not found",
    );
}

fn external_form_command_and_parameter(
    evidence: &mut EvidenceRows,
    required: &mut RequiredProjections,
    version: &str,
    line: PlatformLine,
) {
    for (alias, source_kind, kind, name) in [
        (
            "epf",
            SourceSetKind::ExternalProcessor,
            "ExternalDataProcessor",
            "SyntheticProcessor",
        ),
        (
            "erf",
            SourceSetKind::ExternalReport,
            "ExternalReport",
            "SyntheticReport",
        ),
    ] {
        let source = fixture_path(&format!(
            "formats/platform_xml/{version}/external_artifacts/{alias}"
        ));
        let descriptor = source.join(format!("{name}.xml"));
        let raw = read_fixture_descriptor(&descriptor);
        let xml = parse_fixture_descriptor(&raw, &descriptor);
        let artifact = fixture_artifact(&xml, kind, &descriptor);
        assert_eq!(fixture_properties_name(artifact), Some(name));
        assert_eq!(xml.root_element().attribute("version"), Some(version));
        assert!(native_child_objects(artifact).any(|node| {
            node.has_tag_name((MD_CLASSES_NS, "Form")) && node.text() == Some("Main")
        }));

        let path = source.join(format!("{name}/Forms/Main/Ext/Form.xml"));
        let cancellation = CancellationToken::new();
        let authority = LogicalViewReadAuthority::new(
            &cancellation,
            alias,
            "native-external-form-special-projection",
            source_kind,
            Arc::new(
                unica_workspace::read::open_source_directory(&source.canonicalize().unwrap())
                    .unwrap(),
            ),
            PlatformProfile::for_line(line).unwrap(),
        );
        direct_form_command_and_parameter(
            &mut NativeProjections {
                authority: &authority,
                evidence,
                required,
            },
            &format!("{alias}:{kind}.{name}.Form.Main"),
            &path,
        );
    }
}

fn native_form_requirements(
    required: &mut RequiredProjections,
    source: &Path,
    line: PlatformLine,
    registrations: &[(String, String)],
) {
    for (kind, name) in registrations {
        let Some(layout) = unica_address::kinds::declared(kind).and_then(|(_, kind)| kind.layout)
        else {
            continue;
        };
        let descriptor = source.join(layout.directory).join(format!("{name}.xml"));
        let owner = format!("main:{kind}.{name}");
        if kind == "CommonForm" {
            let path = descriptor.with_extension("").join("Ext/Form.xml");
            registered_form_requirements(required, line, &owner, &path);
            continue;
        }

        let raw = read_fixture_descriptor(&descriptor);
        let xml = parse_fixture_descriptor(&raw, &descriptor);
        let artifact = fixture_artifact(&xml, kind, &descriptor);
        for child_form in
            native_child_objects(artifact).filter(|node| node.has_tag_name((MD_CLASSES_NS, "Form")))
        {
            let name = child_form.text().unwrap().trim();
            assert!(!name.is_empty(), "{descriptor:?}");
            let path = descriptor
                .with_extension("")
                .join("Forms")
                .join(name)
                .join("Ext/Form.xml");
            registered_form_requirements(required, line, &format!("{owner}.Form.{name}"), &path);
        }
    }
}

fn registered_form_requirements(
    required: &mut RequiredProjections,
    line: PlatformLine,
    owner: &str,
    path: &Path,
) {
    let raw = read_fixture_descriptor(path);
    let xml = parse_fixture_descriptor(&raw, path);
    let form = xml.root_element();
    assert!(form.has_tag_name((NATIVE_FORM_NS, "Form")));
    let origin = fixture_origin(path);
    form_item_requirements(required, line, owner, form, &origin);
    let module = path.parent().unwrap().join("Form/Module.bsl");
    if module.is_file() {
        let module_at = format!("{owner}.Module.Form");
        let route = unica_address::module_routes::module_route(
            &QualifiedAddress::parse(&module_at).unwrap(),
        )
        .expect("address of a native form module");
        assert_eq!(route.role(), unica_address::module_routes::ModuleRole::Form);
        required.read(
            (line, SourceSetKind::Configuration),
            &module_at,
            "Body",
            &fixture_origin(&module),
            BTreeSet::from([format!("{module_at}.Body")]),
            FORMS,
        );
    }
    let attributes = native_form_children(form, "Attributes").next();
    if let Some(attributes) = attributes {
        let names = native_form_children(attributes, "Attribute")
            .map(|node| node.attribute("name").unwrap());
        let mut members = named_addresses(owner, "Attribute", names);
        members.insert(format!("{owner}.Attribute"));
        required.read(
            (line, SourceSetKind::Configuration),
            owner,
            "Attribute",
            &origin,
            members,
            FORMS,
        );
    }
}

fn form_item_requirements(
    required: &mut RequiredProjections,
    line: PlatformLine,
    owner: &str,
    container: roxmltree::Node<'_, '_>,
    origin: &str,
) {
    let bars = native_form_children(container, "AutoCommandBar")
        .filter(|node| node.attribute("name").is_some_and(|name| !name.is_empty()));
    let items = native_form_children(container, "ChildItems")
        .flat_map(|group| group.children().filter(roxmltree::Node::is_element));
    let items: Vec<_> = bars.chain(items).collect();
    if items.is_empty() {
        return;
    }
    let branch = QualifiedAddress::parse(&format!("{owner}.Item")).unwrap();
    let mut addresses = BTreeSet::from([branch.to_string()]);
    for item in items {
        let child = branch
            .member(item.attribute("name").unwrap())
            .unwrap()
            .to_string();
        assert!(addresses.insert(child.clone()), "duplicate {child}");
        form_item_requirements(required, line, &child, item, origin);
    }
    required.read(
        (line, SourceSetKind::Configuration),
        owner,
        "Item",
        origin,
        addresses,
        FORMS,
    );
}

fn native_module_slices(native: &mut NativeProjections<'_, '_>, source: &Path, line: PlatformLine) {
    let owner = "main:Catalog.Валюты.Module.Manager";
    let origin = fixture_origin(&source.join("Catalogs/Валюты/Ext/ManagerModule.bsl"));
    // The three named slices are part of INV.SOURCE.MODULE-REGION-INTERFACE.
    let interfaces =
        ["Public", "Library", "Override"].map(|slice| format!("{owner}.Interface.{slice}"));
    native.collection(owner, "Interface", &origin, BTreeSet::from(interfaces));

    let catalog = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
        "../unica-metadata/src/platform-event-catalog-{line}.json"
    ));
    let catalog: Value = serde_json::from_slice(&fs::read(catalog).unwrap()).unwrap();
    assert_eq!(catalog["profile"], line.to_string());
    let manager = catalog["moduleCatalogs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|catalog| catalog["ownerKind"] == "Catalog" && catalog["moduleRole"] == "Manager")
        .expect("platform event declaration for the Catalog Manager");
    let names = manager["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|event| event["eventId"].as_str().unwrap());
    let members = named_addresses(owner, "Event", names);
    assert!(!members.is_empty());
    native.collection(owner, "Event", &origin, members);
}

fn native_module_requirements(
    required: &mut RequiredProjections,
    root: &Path,
    line: PlatformLine,
    registrations: &[(String, String)],
) {
    // These module interiors are witnessed by the native Method and Region
    // checks. Their addresses come from the registrations and source BSL.
    for (kind, name, role, file) in [
        ("Catalog", "Валюты", "Manager", "ManagerModule.bsl"),
        ("Catalog", "Валюты", "Object", "ObjectModule.bsl"),
        ("HTTPService", "ПроверкаСвязи", "HTTPService", "Module.bsl"),
        ("WebService", "ОбменДанными", "WebService", "Module.bsl"),
    ] {
        assert!(registrations
            .iter()
            .any(|(tag, item)| tag == kind && item == name));
        let (_, declaration) = unica_address::kinds::declared(kind).unwrap();
        let directory = declaration.layout.unwrap().directory;
        let owner = format!("main:{kind}.{name}.Module.{role}");
        let route =
            unica_address::module_routes::module_route(&QualifiedAddress::parse(&owner).unwrap())
                .expect("address of a native owner module");
        assert_eq!(route.owner_kind().as_str(), kind);
        assert_eq!(route.role().as_str(), role);
        let path = root.join(directory).join(name).join("Ext").join(file);
        let bsl = read_fixture_descriptor(&path);
        let origin = fixture_origin(&path);
        let methods = bsl.lines().filter_map(|line| {
            let text = line.trim_start();
            let signature = text
                .strip_prefix("Функция ")
                .or_else(|| text.strip_prefix("Процедура "))?;
            signature.split_once('(').map(|(name, _)| name)
        });
        let regions = bsl
            .lines()
            .filter_map(|line| line.trim().strip_prefix("#Область "));
        let source = (line, SourceSetKind::Configuration);
        required.read(
            source,
            &owner,
            "Body",
            &origin,
            BTreeSet::from([format!("{owner}.Body")]),
            MODULES,
        );
        let methods = named_addresses(&owner, "Method", methods);
        assert!(!methods.is_empty());
        for method in &methods {
            required.read(
                source,
                method,
                "Body",
                &origin,
                BTreeSet::from([format!("{method}.Body")]),
                MODULES,
            );
        }
        for (kind, mut members) in [
            ("Method", methods),
            ("Region", named_addresses(&owner, "Region", regions)),
        ] {
            members.insert(format!("{owner}.{kind}"));
            required.read(source, &owner, kind, &origin, members, MODULES);
        }
    }
}
