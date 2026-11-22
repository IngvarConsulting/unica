use super::*;
use std::io::Write;
use unica_source_contract::read::SOURCE_TREE_TRAVERSAL_MAX_DEPTH;

const KIND: &str = "Subsystem";
const DIRECTORY: &str = "Subsystems";
const TEST_CASE: &str = "native_recursive_subsystem_registrations_have_addresses";
const REQUIRED_SHARD: &str = "native_recursive_subsystem_requirements.jsonl";

struct RegisteredBranch {
    owner: QualifiedAddress,
    collection: QualifiedAddress,
    descriptor: PathBuf,
    members: Vec<QualifiedAddress>,
}

impl RegisteredBranch {
    fn new(
        owner: QualifiedAddress,
        collection: QualifiedAddress,
        descriptor: PathBuf,
        names: Vec<String>,
    ) -> Self {
        let members: Vec<_> = names
            .iter()
            .map(|name| collection.member(name).unwrap())
            .collect();
        assert_eq!(
            members
                .iter()
                .map(ToString::to_string)
                .collect::<BTreeSet<_>>()
                .len(),
            members.len(),
            "{collection}: duplicate native registration",
        );
        Self {
            owner,
            collection,
            descriptor,
            members,
        }
    }

    fn expected(&self) -> Vec<String> {
        self.members.iter().map(ToString::to_string).collect()
    }

    fn record_requirement(&self, line: PlatformLine, rows: &mut BTreeMap<String, Value>) {
        let key = address_key(
            (line, SourceSetKind::Configuration),
            &self.owner,
            KIND,
            None,
        );
        let mut addresses = vec![self.collection.to_string()];
        addresses.extend(self.expected());
        let row = rows.entry(key.to_string()).or_insert_with(|| {
            serde_json::json!({
                "schema_version": 1,
                "record_type": "required",
                "key": key,
                "requirement": {"kind": "read", "observations": []},
            })
        });
        row["requirement"]["observations"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({
                "owner": self.owner.to_string(),
                "origin": fixture_origin(&self.descriptor),
                "addresses": addresses,
            }));
    }

    fn verify(&self, authority: &LogicalViewReadAuthority<'_>, evidence: &mut EvidenceRows) {
        let owner = self.owner.to_string();
        let collection = self.collection.to_string();
        let expected = self.expected();
        let observed = if expected.is_empty() {
            read(authority, &collection)
        } else {
            check_native_branch(
                authority,
                &read(authority, &owner),
                &collection,
                &expected.iter().cloned().collect(),
            )
        };
        assert_eq!(observed["kind"], KIND, "{collection}");
        let actual: Vec<_> = observed["items"]
            .as_array()
            .expect("a subsystem collection returns items")
            .iter()
            .map(|item| item["at"].as_str().unwrap())
            .collect();
        assert!(
            actual
                .iter()
                .copied()
                .eq(expected.iter().map(String::as_str)),
            "{collection}: expected native registration order {expected:?}, got {actual:?}",
        );

        let line = authority.platform_profile().line();
        let origin = fixture_origin(&self.descriptor);
        evidence.record(
            line,
            &owner,
            KIND,
            None,
            &origin,
            AddressEvidence::Read(&collection),
        );
        for at in actual {
            assert_eq!(read(authority, at)["kind"], KIND, "{at}");
            evidence.record(line, &owner, KIND, None, &origin, AddressEvidence::Read(at));
        }
    }
}

fn registered_tree(source: &Path, version: &str) -> Vec<RegisteredBranch> {
    let roots = root_registrations(source, version)
        .into_iter()
        .filter_map(|(kind, name)| (kind == KIND).then_some(name))
        .collect();
    let root = RegisteredBranch::new(
        QualifiedAddress::parse("main:Configuration").unwrap(),
        QualifiedAddress::parse("main:Subsystem").unwrap(),
        source.join("Configuration.xml"),
        roots,
    );
    assert!(
        !root.members.is_empty(),
        "{version}: a native subsystem fixture"
    );
    let mut branches = Vec::new();
    for member in &root.members {
        let name = member.segments().last().unwrap().name().unwrap();
        let descriptor = source.join(DIRECTORY).join(format!("{name}.xml"));
        visit_registered_subsystem(source, version, member, descriptor, &mut branches);
    }
    branches.push(root);
    branches
}

fn visit_registered_subsystem(
    source: &Path,
    version: &str,
    owner: &QualifiedAddress,
    descriptor: PathBuf,
    branches: &mut Vec<RegisteredBranch>,
) {
    let relative = descriptor.strip_prefix(source).unwrap();
    assert!(
        relative.components().count() - 1 <= SOURCE_TREE_TRAVERSAL_MAX_DEPTH,
        "{owner}: native descriptor exceeds the traversal limit",
    );
    let xml = read_fixture_descriptor(&descriptor);
    let document = parse_fixture_descriptor(&xml, &descriptor);
    assert_eq!(document.root_element().attribute("version"), Some(version));
    let artifact = fixture_artifact(&document, KIND, &descriptor);
    let name = owner.segments().last().unwrap().name().unwrap();
    assert_eq!(fixture_properties_name(artifact), Some(name), "{owner}");
    assert!(uuid::Uuid::parse_str(artifact.attribute("uuid").unwrap()).is_ok());
    assert_eq!(
        artifact
            .children()
            .filter(|node| node.has_tag_name((MD_CLASSES_NS, "ChildObjects")))
            .count(),
        1,
        "{owner}: declared native ChildObjects container",
    );

    let names = native_child_objects(artifact)
        .map(|node| {
            assert!(node.has_tag_name((MD_CLASSES_NS, KIND)), "{owner}");
            node.text().expect("registered subsystem name").to_owned()
        })
        .collect();
    let collection = QualifiedAddress::parse(&format!("{owner}.{KIND}")).unwrap();
    let nested_dir = descriptor.with_extension("").join(DIRECTORY);
    let branch = RegisteredBranch::new(owner.clone(), collection, descriptor, names);
    for member in &branch.members {
        let name = member.segments().last().unwrap().name().unwrap();
        let nested = nested_dir.join(format!("{name}.xml"));
        visit_registered_subsystem(source, version, member, nested, branches);
    }
    branches.push(branch);
}

/// rule: CTR.SOURCE.QUALIFIED-LOGICAL-ADDRESS
/// rule: INV.SOURCE.SUBSYSTEM-REGISTRATION
#[test]
fn native_recursive_subsystem_registrations_have_addresses() {
    let layout = METADATA_LAYOUTS
        .iter()
        .find(|layout| layout.tag == KIND)
        .unwrap();
    assert!(DECLARED_ROOT_KINDS.contains(&KIND));
    assert_eq!(layout.directory, DIRECTORY);
    assert!(layout.requires_child_objects);
    assert!(NodeKind::Subsystem.takes_name());

    let mut requirements = BTreeMap::new();
    let mut evidence = EvidenceRows::new(TEST_CASE, SourceSetKind::Configuration);
    let formats =
        ["2.20", "2.21", "2.22"].map(|version| (export_format_line(version).unwrap(), version));
    assert_eq!(
        formats
            .iter()
            .map(|(line, _)| *line)
            .collect::<BTreeSet<_>>(),
        DESCRIBED_PLATFORM_LINES
            .iter()
            .map(|(line, _)| *line)
            .collect::<BTreeSet<_>>(),
    );
    for (line, version) in formats {
        assert!(metadata_kind_exists(Some(line), KIND));
        let source = fixture_path(&format!("formats/platform_xml/{version}/all_kinds/src"));
        let branches = registered_tree(&source, version);
        for branch in &branches {
            branch.record_requirement(line, &mut requirements);
        }

        let profile = PlatformProfile::for_line(line).unwrap();
        let cancellation = CancellationToken::new();
        let authority = native_authority(&cancellation, &source, profile);
        for branch in &branches {
            branch.verify(&authority, &mut evidence);
        }
    }
    if let Some(directory) = std::env::var_os(EVIDENCE_DIRECTORY) {
        let path = PathBuf::from(directory).join(REQUIRED_SHARD);
        let mut file = fs::File::create_new(&path).unwrap();
        for row in requirements.into_values() {
            writeln!(&mut file, "{row}").unwrap();
        }
    }
    evidence.finish();
}
