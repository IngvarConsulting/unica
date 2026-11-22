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
        line: PlatformLine,
        owner: &str,
        kind: &str,
        origin: &str,
        addresses: BTreeSet<String>,
        test_case: &str,
    ) {
        let owner_address = QualifiedAddress::parse(owner).unwrap();
        let key = address_key(
            (line, SourceSetKind::Configuration),
            &owner_address,
            kind,
            None,
        );
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
        let line = self.authority.platform_profile().line();
        for address in &addresses {
            self.evidence.record(
                line,
                owner,
                kind,
                None,
                origin,
                AddressEvidence::Read(address),
            );
        }
        self.required
            .read(line, owner, kind, origin, addresses, SHARD);
    }
}

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
        native_form_requirements(&mut required, &source, line);
        native_module_requirements(&mut required, &source, line);
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

fn native_form_requirements(required: &mut RequiredProjections, source: &Path, line: PlatformLine) {
    let owner = "main:CommonForm.MainA";
    let path = source.join("CommonForms/MainA/Ext/Form.xml");
    let raw = read_fixture_descriptor(&path);
    let xml = parse_fixture_descriptor(&raw, &path);
    let form = xml.root_element();
    assert!(form.has_tag_name((NATIVE_FORM_NS, "Form")));
    let origin = fixture_origin(&path);
    let bars = native_form_children(form, "AutoCommandBar");
    let items = native_form_children(form, "ChildItems")
        .flat_map(|group| group.children().filter(roxmltree::Node::is_element));
    let attributes = native_form_children(form, "Attributes")
        .flat_map(|group| native_form_children(group, "Attribute"));
    for (kind, mut members) in [
        (
            "Item",
            named_addresses(
                owner,
                "Item",
                bars.chain(items).filter_map(|node| node.attribute("name")),
            ),
        ),
        (
            "Attribute",
            named_addresses(
                owner,
                "Attribute",
                attributes.map(|node| node.attribute("name").unwrap()),
            ),
        ),
    ] {
        assert!(!members.is_empty(), "{owner}.{kind}");
        members.insert(format!("{owner}.{kind}"));
        required.read(line, owner, kind, &origin, members, FORMS);
    }
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
    source: &Path,
    line: PlatformLine,
) {
    let owner = "main:Catalog.Валюты.Module.Manager";
    let path = source.join("Catalogs/Валюты/Ext/ManagerModule.bsl");
    let raw = read_fixture_descriptor(&path);
    let origin = fixture_origin(&path);
    let method_names = raw.lines().filter_map(|line| {
        let signature = line.trim_start().strip_prefix("Функция ")?;
        signature.split_once('(').map(|(name, _)| name)
    });
    let region_names = raw
        .lines()
        .filter_map(|line| line.trim().strip_prefix("#Область "));
    required.read(
        line,
        owner,
        "Body",
        &origin,
        BTreeSet::from([format!("{owner}.Body")]),
        MODULES,
    );
    for (kind, mut members) in [
        ("Method", named_addresses(owner, "Method", method_names)),
        ("Region", named_addresses(owner, "Region", region_names)),
    ] {
        assert!(!members.is_empty(), "{owner}.{kind}");
        if kind == "Method" {
            for method in &members {
                required.read(
                    line,
                    method,
                    "Body",
                    &origin,
                    BTreeSet::from([format!("{method}.Body")]),
                    MODULES,
                );
            }
        }
        members.insert(format!("{owner}.{kind}"));
        required.read(line, owner, kind, &origin, members, MODULES);
    }
}
