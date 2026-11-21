//! Собираем таблицу из ветвей канонического view на существующих фикстурах.
use super::*;
use crate::infrastructure::daemon::v13_call_graph::call_graph_support;
use unica_source_contract::format_identity::export_format_line;

const EVIDENCE_TEST_CASE: &str =
    "medium_published_document_and_bsl_branches_have_canonical_addresses";
const MD_CLASSES_NS: &str = "http://v8.1c.ru/8.3/MDClasses";

enum Evidence<'a> {
    Read(&'a str),
    ModuleRoundTrip { at: &'a str, descriptor: &'a str },
}

struct EvidenceRows {
    directory: Option<PathBuf>,
    lines: Vec<String>,
}

impl EvidenceRows {
    fn new() -> Self {
        Self {
            directory: std::env::var_os("UNICA_484_ADDRESS_JSONL_DIR").map(PathBuf::from),
            lines: Vec::new(),
        }
    }

    fn record(&mut self, fixture: &Fixture, origin: (&str, &str), at: &str) {
        self.record_evidence(fixture, origin, Evidence::Read(at));
    }

    fn record_evidence(&mut self, fixture: &Fixture, origin: (&str, &str), evidence: Evidence<'_>) {
        if self.directory.is_none() {
            return;
        }
        let (at, level, descriptor) = match evidence {
            Evidence::Read(at) => (at, "read", None),
            Evidence::ModuleRoundTrip { at, descriptor } => (at, "roundtrip", Some(descriptor)),
        };
        let address = parsed(at);
        let (branch, owner_segments) = address.segments().split_last().unwrap();
        let role = branch.name();
        if descriptor.is_some() {
            assert_eq!(branch.kind(), NodeKind::Module, "{at}");
            assert!(role.is_some(), "{at}: expected a named module role");
        } else {
            assert!(role.is_none(), "{at}: expected a branch address");
        }
        let owner = if owner_segments.is_empty() {
            parsed(&format!("{}:Configuration", address.source_set()))
        } else {
            address.prefix(owner_segments.len())
        };
        let (_, source_kind, root) = source_sets(&fixture.workspace)
            .into_iter()
            .find(|(name, _, _)| name == owner.source_set())
            .expect("fixture source set for evidence");
        let descriptor = match source_kind {
            SourceSetKind::Configuration | SourceSetKind::Extension => {
                assert!(
                    descriptor.is_none(),
                    "{at}: expected a configuration descriptor"
                );
                "Configuration.xml"
            }
            SourceSetKind::ExternalProcessor | SourceSetKind::ExternalReport => {
                descriptor.expect("external witness requires its owner descriptor")
            }
        };
        let xml = fixture_xml(fixture, at, descriptor);
        let descriptor = roxmltree::Document::parse(xml.trim_start_matches('\u{feff}')).unwrap();
        let format = descriptor.root_element().attribute("version").unwrap();
        let line = export_format_line(format).expect("fixture export format");
        let source = root.strip_prefix(&fixture.workspace).unwrap();
        let origin_path = source.join(origin.1).to_string_lossy().replace('\\', "/");
        let origin = format!("tests/fixtures/acceptance/{}/{origin_path}", origin.0);
        let owner_shape: Vec<_> = owner
            .segments()
            .iter()
            .map(|segment| {
                json!({
                    "kind": segment.kind().as_str(),
                    "named": segment.name().is_some(),
                    "role": (segment.kind() == NodeKind::Interface)
                        .then_some(segment.name())
                        .flatten(),
                })
            })
            .collect();
        self.lines.push(
            json!({
                "schema_version": 1,
                "record_type": "evidence",
                "key": {
                    "line": line.to_string(),
                    "source_kind": source_kind,
                    "owner_shape": owner_shape,
                    "branch": branch.kind().as_str(),
                    "role": role,
                },
                "witness": {
                    "owner": owner.to_string(),
                    "at": at,
                    "level": level,
                    "origin": origin,
                    "test_case": EVIDENCE_TEST_CASE,
                    "reason": null,
                },
            })
            .to_string(),
        );
    }

    fn finish(mut self) {
        if let Some(directory) = self.directory {
            use std::io::Write;
            self.lines.sort_unstable();
            let output = directory.join(format!("{EVIDENCE_TEST_CASE}.jsonl"));
            let mut file = std::fs::File::create_new(&output)
                .unwrap_or_else(|error| panic!("address evidence {}: {error}", output.display()));
            for line in self.lines {
                writeln!(file, "{line}").unwrap_or_else(|error| {
                    panic!("address evidence {}: {error}", output.display())
                });
            }
        }
    }
}

fn read_view(
    runtime: &V5CanonicalInvocationRuntime,
    workspace: &Path,
    at: &str,
) -> (Value, Vec<Value>) {
    let call = |args| invoke_on(runtime, workspace, ToolIdentity::View, args);
    let mut result = call(json!({"at":at,"limit":50}));
    assert!(result.ok, "{at}: {result:?}");
    let first = result.data.take().expect("view has a projection");
    assert_eq!(first["at"].as_str(), Some(at));
    let mut items = collection_rows(&first).to_vec();
    let mut cursors = BTreeSet::new();
    while let Some(cursor) = result.cursor.take() {
        assert!(cursors.insert(cursor.clone()), "{at}: repeated cursor");
        result = call(json!({"at":at,"cursor":cursor,"limit":50}));
        assert!(result.ok, "{at}: {result:?}");
        let page = result.data.as_ref().unwrap();
        assert_eq!(page["at"].as_str(), Some(at));
        items.extend_from_slice(collection_rows(page));
    }
    (first, items)
}

fn collection_rows(view: &Value) -> &[Value] {
    view["items"].as_array().map_or(&[], Vec::as_slice)
}

fn parsed(at: &str) -> QualifiedAddress {
    let parsed = QualifiedAddress::parse(at).unwrap_or_else(|error| panic!("{at}: {error}"));
    assert_eq!(
        parsed.to_string(),
        at,
        "published address must be canonical"
    );
    parsed
}

#[derive(Debug, PartialEq, Eq)]
struct Branch {
    owner_kind: String,
    kind: NodeKind,
    graph: bool,
}

type Edges = BTreeMap<(String, String), Branch>;
type Row = (String, &'static str, String, &'static str);

fn record_branches(
    owner: &QualifiedAddress,
    view: &Value,
    contains: &impl Fn(&str) -> bool,
    queue: &mut VecDeque<String>,
    edges: &mut Edges,
) {
    let at = owner.to_string();
    let mut declared = BTreeSet::new();
    for entry in view["branches"].as_array().into_iter().flatten() {
        let kind = view["kind"].as_str().expect("a branch has an owning kind");
        let branch = entry["at"]
            .as_str()
            .expect("a published branch has an address");
        assert!(
            contains(branch),
            "{at}: branch leaves the witness subtree: {branch}"
        );
        assert!(declared.insert(branch), "{at} published {branch} twice");
        let parsed_branch = parsed(branch);
        let terminal = parsed_branch.segments().last().unwrap().kind();
        let graph = matches!(terminal, NodeKind::Caller | NodeKind::Callee);
        if graph {
            // The daemon dispatches these branches before the source reader.
            // Their existing route proof does not start an analyzer.
            assert_eq!(kind, NodeKind::Method.as_str());
            assert!(entry.get("count").is_none());
            assert_eq!(branch, format!("{at}.{}", terminal.as_str()));
            call_graph_support(owner).unwrap_or_else(|error| panic!("{branch}: {error}"));
        } else {
            queue.push_back(branch.to_string());
        }
        let edge = Branch {
            owner_kind: kind.to_string(),
            kind: terminal,
            graph,
        };
        let key = (at.clone(), branch.to_string());
        if let Some(previous) = edges.get(&key) {
            assert_eq!(previous, &edge, "{at}: inconsistent branch {branch}");
        } else {
            edges.insert(key, edge);
        }
    }
}

/// Both node projections and collection rows can carry branches. Every arrow
/// in the published source tree must have a row proved here.
fn check_branches(
    runtime: &V5CanonicalInvocationRuntime,
    workspace: &Path,
    seed: &str,
) -> Vec<Row> {
    let contains = |at: &str| {
        at == seed
            || at
                .strip_prefix(seed)
                .is_some_and(|tail| tail.starts_with('.'))
    };
    let profile = PlatformProfile::v8_3_27();
    let mut queue = VecDeque::from([seed.to_string()]);
    let mut opened = BTreeSet::new();
    let mut edges = Edges::new();
    while let Some(at) = queue.pop_front() {
        if !opened.insert(at.clone()) {
            continue;
        }
        let owner = parsed(&at);
        assert!(classify_route(&owner, profile).is_some(), "{at}");
        let (view, items) = read_view(runtime, workspace, &at);
        record_branches(&owner, &view, &contains, &mut queue, &mut edges);
        for item in items {
            let Some(item_at) = item["at"].as_str() else {
                assert!(
                    item.get("branches").is_none(),
                    "{at}: a data row owns branches"
                );
                continue;
            };
            // References into other trees need their own witnesses. Methods
            // listed inside a region or interface belong to this same module.
            if contains(item_at) {
                let item_address = parsed(item_at);
                record_branches(&item_address, &item, &contains, &mut queue, &mut edges);
                queue.push_back(item_at.to_string());
            }
        }
    }

    assert!(!edges.is_empty(), "{seed} did not publish any branches");
    edges
        .into_iter()
        .map(|((owner, at), branch)| {
            let evidence = if branch.graph {
                "canonical call-graph route; analyzer not queried"
            } else {
                assert!(
                    opened.contains(&at),
                    "{owner}: missing verified row for {at}"
                );
                "canonical view"
            };
            (branch.owner_kind, branch.kind.as_str(), at, evidence)
        })
        .collect()
}

enum FixtureBranch {
    Present(usize),
    AbsentInFixture,
}

impl FixtureBranch {
    fn from_count(count: usize) -> Self {
        if count == 0 {
            Self::AbsentInFixture
        } else {
            Self::Present(count)
        }
    }
}

fn check_fixture_branches<const N: usize>(view: &Value, expected: [(&str, FixtureBranch); N]) {
    let mut actual = BTreeMap::new();
    for branch in view["branches"].as_array().into_iter().flatten() {
        let at = branch["at"].as_str().unwrap();
        let count = branch["count"].as_u64().unwrap();
        assert!(
            actual.insert(at, count).is_none(),
            "duplicate published branch {at}"
        );
    }
    let expected: BTreeMap<_, _> = expected
        .into_iter()
        .filter_map(|(at, state)| match state {
            FixtureBranch::Present(count) => Some((at, count as u64)),
            FixtureBranch::AbsentInFixture => None,
        })
        .collect();
    assert_eq!(actual, expected, "published branches for {}", view["at"]);
}

fn check_fixture_members<'a>(
    runtime: &V5CanonicalInvocationRuntime,
    fixture: &Fixture,
    branch: &str,
    names: impl IntoIterator<Item = &'a str>,
) -> BTreeMap<String, Value> {
    let address = parsed(branch);
    let kind = address.segments().last().unwrap().kind().as_str();
    let mut expected = BTreeMap::new();
    for name in names {
        let at = address.member(name).unwrap().to_string();
        assert!(
            expected.insert(at, name).is_none(),
            "duplicate fixture member {name} of {branch}"
        );
    }
    let (_, items) = read_view(runtime, &fixture.workspace, branch);
    let actual: BTreeSet<_> = items
        .iter()
        .map(|item| item["at"].as_str().unwrap())
        .collect();
    assert_eq!(
        actual.len(),
        items.len(),
        "duplicate published member of {branch}"
    );
    assert_eq!(
        actual,
        expected.keys().map(String::as_str).collect::<BTreeSet<_>>(),
        "published members of {branch}"
    );
    expected
        .into_iter()
        .map(|(at, name)| {
            let (view, _) = read_view(runtime, &fixture.workspace, &at);
            assert_eq!(view["kind"], kind, "{at}");
            (name.to_owned(), view)
        })
        .collect()
}

fn fixture_xml(fixture: &Fixture, owner: &str, relative: &str) -> String {
    let path = source_root(&fixture.workspace, parsed(owner).source_set()).join(relative);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("fixture {}: {error}", path.display()))
}

fn child_name<'a>(parent: roxmltree::Node<'a, '_>, namespace: &str) -> &'a str {
    parent
        .children()
        .find(|node| node.has_tag_name((namespace, "name")))
        .and_then(|node| node.text())
        .expect("fixture node has no name")
}

fn check_role_xml(
    runtime: &V5CanonicalInvocationRuntime,
    fixture: &Fixture,
    evidence: &mut EvidenceRows,
) {
    const NS: &str = "http://v8.1c.ru/8.2/roles";
    const ORIGIN: (&str, &str) = (
        "workspace-role-comment-subsystem-help",
        "Roles/Reader/Ext/Rights.xml",
    );
    let owner = "main:Role.Reader";
    let branch = "main:Role.Reader.Right";
    let raw = fixture_xml(fixture, owner, ORIGIN.1);
    let document = roxmltree::Document::parse(raw.trim_start_matches('\u{feff}')).unwrap();
    let root = document.root_element();
    assert!(root.has_tag_name((NS, "Rights")));
    let mut restrictions = BTreeMap::new();
    for object in root
        .children()
        .filter(|node| node.has_tag_name((NS, "object")))
    {
        let (kind, name) = child_name(object, NS).split_once('.').unwrap();
        let rights: Vec<_> = object
            .children()
            .filter(|node| {
                node.has_tag_name((NS, "right"))
                    && node
                        .children()
                        .any(|child| child.has_tag_name((NS, "restrictionByCondition")))
            })
            .map(|node| child_name(node, NS))
            .collect();
        assert!(restrictions
            .insert(format!("{kind}_{name}"), rights)
            .is_none());
    }
    assert!(restrictions.values().any(Vec::is_empty));
    assert!(restrictions.values().any(|rights| !rights.is_empty()));
    let (view, _) = read_view(runtime, &fixture.workspace, owner);
    check_fixture_branches(
        &view,
        [(branch, FixtureBranch::Present(restrictions.len()))],
    );
    let objects = check_fixture_members(
        runtime,
        fixture,
        branch,
        restrictions.keys().map(String::as_str),
    );
    evidence.record(fixture, ORIGIN, branch);
    for (name, rights) in restrictions {
        let view = &objects[&name];
        let at = format!("{}.RLS", view["at"].as_str().unwrap());
        check_fixture_branches(view, [(&at, FixtureBranch::from_count(rights.len()))]);
        if !rights.is_empty() {
            check_fixture_members(runtime, fixture, &at, rights);
            evidence.record(fixture, ORIGIN, &at);
        }
    }
}

fn check_xdto_xml(
    runtime: &V5CanonicalInvocationRuntime,
    fixture: &Fixture,
    evidence: &mut EvidenceRows,
) {
    const NS: &str = "http://v8.1c.ru/8.1/xdto";
    const ORIGIN: (&str, &str) = (
        "workspace-xdto-anonymous",
        "XDTOPackages/ПримерФормата/Ext/Package.bin",
    );
    let owner = "main:XDTOPackage.ПримерФормата";
    let raw = fixture_xml(fixture, owner, ORIGIN.1);
    let document = roxmltree::Document::parse(raw.trim_start_matches('\u{feff}')).unwrap();
    let root = document.root_element();
    assert!(root.has_tag_name((NS, "package")));
    let namespace = root.attribute("targetNamespace").unwrap();
    let types: Vec<_> = root
        .children()
        .filter(|node| {
            node.has_tag_name((NS, "objectType")) || node.has_tag_name((NS, "valueType"))
        })
        .collect();
    assert!(!types.is_empty());
    assert!(
        !root
            .children()
            .any(|node| node.has_tag_name((NS, "property"))),
        "fixture must witness absent global Property"
    );
    let namespace_at = format!("{owner}.Namespace");
    let types_at = format!("{owner}.Type");
    let (view, _) = read_view(runtime, &fixture.workspace, owner);
    check_fixture_branches(
        &view,
        [
            (&namespace_at, FixtureBranch::Present(1)),
            (&types_at, FixtureBranch::Present(types.len())),
            (&format!("{owner}.Property"), FixtureBranch::AbsentInFixture),
        ],
    );
    let (view, _) = read_view(runtime, &fixture.workspace, &namespace_at);
    assert_eq!(view["kind"], "Namespace");
    assert_eq!(view["props"]["namespace"], namespace);
    evidence.record(fixture, ORIGIN, &namespace_at);
    let nodes = check_fixture_members(
        runtime,
        fixture,
        &types_at,
        types.iter().map(|node| node.attribute("name").unwrap()),
    );
    evidence.record(fixture, ORIGIN, &types_at);
    let mut counts = BTreeSet::new();
    for ty in types {
        let view = &nodes[ty.attribute("name").unwrap()];
        let at = format!("{}.Property", view["at"].as_str().unwrap());
        let properties: Vec<_> = ty
            .children()
            .filter(|node| node.has_tag_name((NS, "property")))
            .map(|node| node.attribute("name").unwrap())
            .collect();
        counts.insert(properties.len());
        check_fixture_branches(view, [(&at, FixtureBranch::from_count(properties.len()))]);
        if !properties.is_empty() {
            check_fixture_members(runtime, fixture, &at, properties);
            evidence.record(fixture, ORIGIN, &at);
        }
    }
    assert!(counts.contains(&0) && counts.iter().any(|count| *count > 0));
}

fn check_mxl_xml(
    runtime: &V5CanonicalInvocationRuntime,
    fixture: &Fixture,
    evidence: &mut EvidenceRows,
) {
    const NS: &str = "http://v8.1c.ru/8.2/data/spreadsheet";
    const XSI: &str = "http://www.w3.org/2001/XMLSchema-instance";
    const ORIGIN: (&str, &str) = (
        "workspace-mxl",
        "Reports/F06Report/Templates/F06Template/Ext/Template.xml",
    );
    let owner = "cf:Report.F06Report.Template.F06Template";
    let branch = "cf:Report.F06Report.Template.F06Template.Area";
    let raw = fixture_xml(fixture, owner, ORIGIN.1);
    let document = roxmltree::Document::parse(raw.trim_start_matches('\u{feff}')).unwrap();
    let root = document.root_element();
    assert!(root.has_tag_name((NS, "document")));
    let names: Vec<_> = root
        .children()
        .filter(|node| {
            node.has_tag_name((NS, "namedItem"))
                && node.attribute((XSI, "type")) == Some("NamedItemCells")
        })
        .map(|node| child_name(node, NS))
        .collect();
    assert!(!names.is_empty());
    let (view, _) = read_view(runtime, &fixture.workspace, owner);
    check_fixture_branches(&view, [(branch, FixtureBranch::Present(names.len()))]);
    check_fixture_members(runtime, fixture, branch, names);
    evidence.record(fixture, ORIGIN, branch);
}

fn check_resource(
    runtime: &V5CanonicalInvocationRuntime,
    fixture: &Fixture,
    at: &str,
    relative: &str,
) {
    let owner = parsed(at);
    let source = source_root(&fixture.workspace, owner.source_set());
    let path = source.join(relative);
    assert!(path.is_file(), "native witness is absent: {:?}", path);
    let by_address = invoke_on(
        runtime,
        &fixture.workspace,
        ToolIdentity::Resolve,
        json!({"at":at}),
    );
    assert!(by_address.ok, "{at}: {by_address:?}");
    let data = by_address.data.unwrap();
    assert_eq!(data["at"], at);
    assert_eq!(data["path"], relative);

    let by_path = invoke_on(
        runtime,
        &fixture.workspace,
        ToolIdentity::Resolve,
        json!({"path":path.to_string_lossy()}),
    );
    assert!(by_path.ok, "{relative}: {by_path:?}");
    let data = by_path.data.unwrap();
    assert_eq!(data["at"], at);
    assert_eq!(data["path"], relative);
}

fn metadata_child<'a, 'input: 'a>(
    parent: roxmltree::Node<'a, 'input>,
    tag: &str,
) -> roxmltree::Node<'a, 'input> {
    parent
        .children()
        .find(|node| node.has_tag_name((MD_CLASSES_NS, tag)))
        .unwrap_or_else(|| panic!("fixture has no {tag} under {}", parent.tag_name().name()))
}

fn check_metadata_identity<'a, 'input: 'a>(
    root: roxmltree::Node<'a, 'input>,
    kind: &str,
    name: &str,
) -> roxmltree::Node<'a, 'input> {
    assert!(root.has_tag_name((MD_CLASSES_NS, "MetaDataObject")));
    let node = metadata_child(root, kind);
    assert_eq!(
        metadata_child(metadata_child(node, "Properties"), "Name").text(),
        Some(name)
    );
    node
}

fn check_external_modules(
    runtime: &V5CanonicalInvocationRuntime,
    fixture: &Fixture,
    evidence: &mut EvidenceRows,
) {
    for (alias, kind, name, stem, source_kind) in [
        (
            "epf",
            "ExternalDataProcessor",
            "SyntheticProcessor",
            "source",
            SourceSetKind::ExternalProcessor,
        ),
        (
            "erf",
            "ExternalReport",
            "SyntheticReport",
            "report-source",
            SourceSetKind::ExternalReport,
        ),
    ] {
        assert!(fixture
            .sets
            .iter()
            .any(|(set, kind, _)| set == alias && *kind == source_kind));
        let owner = format!("{alias}:{kind}.{name}");
        let descriptor = format!("{stem}.xml");
        let xml = fixture_xml(fixture, &owner, &descriptor);
        let document = roxmltree::Document::parse(xml.trim_start_matches('\u{feff}')).unwrap();
        let root = document.root_element();
        let object = check_metadata_identity(root, kind, name);
        let main_forms = metadata_child(object, "ChildObjects")
            .children()
            .filter(|node| {
                node.has_tag_name((MD_CLASSES_NS, "Form")) && node.text() == Some("Main")
            })
            .count();
        assert_eq!(main_forms, 1, "{descriptor}: Form.Main registration");

        let form_xml = fixture_xml(fixture, &owner, &format!("{stem}/Forms/Main.xml"));
        let form_document =
            roxmltree::Document::parse(form_xml.trim_start_matches('\u{feff}')).unwrap();
        let form_root = form_document.root_element();
        check_metadata_identity(form_root, "Form", "Main");
        assert_eq!(form_root.attribute("version"), root.attribute("version"));

        let form = format!("{owner}.Form.Main");
        for (parent, role, file) in [
            (owner.as_str(), "Object", "Ext/ObjectModule.bsl"),
            (form.as_str(), "Form", "Forms/Main/Ext/Form/Module.bsl"),
        ] {
            let branch = format!("{parent}.Module");
            let (view, _) = read_view(runtime, &fixture.workspace, parent);
            let published: Vec<_> = published_branch_counts(&view, &branch).collect();
            assert_eq!(published, vec![1], "{parent}: {branch}");
            check_fixture_members(runtime, fixture, &branch, [role]);
            let at = format!("{branch}.{role}");
            let file = format!("{stem}/{file}");
            check_resource(runtime, fixture, &at, &file);
            evidence.record_evidence(
                fixture,
                ("workspace-external-alias", &file),
                Evidence::ModuleRoundTrip {
                    at: &at,
                    descriptor: &descriptor,
                },
            );
        }
    }
}

fn published_branch_counts<'a>(view: &'a Value, at: &'a str) -> impl Iterator<Item = u64> + 'a {
    view["branches"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(move |branch| branch["at"] == at)
        .map(|branch| branch["count"].as_u64().expect("published branch count"))
}

fn check_extension_common_module(
    runtime: &V5CanonicalInvocationRuntime,
    fixture: &Fixture,
    evidence: &mut EvidenceRows,
) {
    const KIND: &str = "CommonModule";
    const COLL: &str = "ext:CommonModule";
    const NAME: &str = "ПР_Пример";
    assert!(fixture
        .sets
        .iter()
        .any(|(set, kind, _)| set == "ext" && *kind == SourceSetKind::Extension));
    let config_xml = fixture_xml(fixture, COLL, "Configuration.xml");
    let config = roxmltree::Document::parse(config_xml.trim_start_matches('\u{feff}')).unwrap();
    let root = config.root_element();
    let extension = check_metadata_identity(root, "Configuration", "CodeExtension");
    let registered: Vec<_> = metadata_child(extension, "ChildObjects")
        .children()
        .filter(|node| node.has_tag_name((MD_CLASSES_NS, KIND)))
        .map(|node| node.text().expect("registered CommonModule name"))
        .collect();
    assert_eq!(registered, [NAME]);

    let descriptor = fixture_xml(fixture, COLL, &format!("CommonModules/{NAME}.xml"));
    let descriptor = roxmltree::Document::parse(descriptor.trim_start_matches('\u{feff}')).unwrap();
    let descriptor_root = descriptor.root_element();
    check_metadata_identity(descriptor_root, KIND, NAME);
    assert_eq!(
        descriptor_root.attribute("version"),
        root.attribute("version")
    );
    let module = fixture_xml(
        fixture,
        COLL,
        &format!("CommonModules/{NAME}/Ext/Module.bsl"),
    );
    let source = module.trim_start_matches('\u{feff}');
    assert!(!source.is_empty());

    let (view, _) = read_view(runtime, &fixture.workspace, "ext:Configuration");
    let counts: Vec<_> = published_branch_counts(&view, COLL).collect();
    assert_eq!(
        counts,
        [registered.len() as u64],
        "ext:Configuration: {COLL}"
    );
    check_fixture_members(runtime, fixture, COLL, registered);
    let body = format!("{COLL}.{NAME}.Body");
    let response = invoke_on(
        runtime,
        &fixture.workspace,
        ToolIdentity::View,
        json!({"at":body}),
    );
    assert!(response.ok, "{body}: {response:?}");
    assert_eq!(response.data.unwrap()["at"], body);
    assert_eq!(response.source_text.as_deref(), Some(source), "{body}");
    evidence.record(fixture, ("workspace-code", "Configuration.xml"), COLL);
}

/// rule: CTR.SOURCE.QUALIFIED-LOGICAL-ADDRESS
/// rule: INV.SOURCE.LOGICAL-BRANCH-COUNT
/// rule: INV.SOURCE.ROLE-RIGHT-ADDRESSES
#[test]
fn medium_published_document_and_bsl_branches_have_canonical_addresses() {
    let mut table = BTreeSet::new();
    let mut evidence = EvidenceRows::new();
    for (name, addresses) in [
        (
            "workspace-form-item-bindings",
            &["main:Catalog.Команды.Form.ФормаСписка"][..],
        ),
        (
            "form-conditional-appearance",
            &["main:CommonForm.Appearance"][..],
        ),
        (
            "workspace-dcs",
            &["cf:Report.F05Report.Template.F05Schema"][..],
        ),
        (
            "workspace-structured-property-names",
            &["cf:Report.F05Report.Template.F05Schema"][..],
        ),
        (
            "workspace-mxl",
            &["cf:Report.F06Report.Template.F06Template"][..],
        ),
        (
            "workspace-xdto-anonymous",
            &["main:XDTOPackage.ПримерФормата"][..],
        ),
        (
            "workspace-command-groups",
            &["main:Subsystem.Подсистема1"][..],
        ),
        (
            "workspace-configuration-support",
            &["main:Module.ManagedApplication"][..],
        ),
        (
            "workspace-role-comment-subsystem-help",
            &["main:Role.Reader"][..],
        ),
        (
            "workspace-module-variables",
            &["main:CommonModule.Probe"][..],
        ),
        (
            "workspace-notification-binding",
            &["main:Catalog.Команды.Form.ФормаСписка.Module.Form"][..],
        ),
        (
            "workspace-common-module-interface",
            &["main:CommonModule.ОбщегоНазначения"][..],
        ),
        (
            "workspace-external-alias",
            &[
                "epf:ExternalDataProcessor.SyntheticProcessor.Module",
                "epf:ExternalDataProcessor.SyntheticProcessor.Form.Main.Module",
                "erf:ExternalReport.SyntheticReport.Module",
                "erf:ExternalReport.SyntheticReport.Form.Main.Module",
            ][..],
        ),
        ("workspace-code", &[][..]),
    ] {
        let fixture = fixture(name);
        let runtime = V5CanonicalInvocationRuntime::new(
            Arc::new(CanonicalV13ReadService::default()),
            Arc::new(TokioClock),
        );
        for at in addresses {
            table.extend(check_branches(&runtime, &fixture.workspace, at));
        }
        match name {
            "workspace-role-comment-subsystem-help" => {
                check_role_xml(&runtime, &fixture, &mut evidence)
            }
            "workspace-xdto-anonymous" => check_xdto_xml(&runtime, &fixture, &mut evidence),
            "workspace-mxl" => check_mxl_xml(&runtime, &fixture, &mut evidence),
            "workspace-external-alias" => check_external_modules(&runtime, &fixture, &mut evidence),
            "workspace-code" => check_extension_common_module(&runtime, &fixture, &mut evidence),
            _ => {}
        }
        if name == "workspace-command-groups" {
            let interface = "main:Subsystem.Подсистема1.Interface";
            let group = format!("{interface}.Group");
            for at in [interface, &format!("{interface}.Command"), &group] {
                assert!(table.iter().any(|(_, _, row, _)| row == at), "{at}");
            }
            let (_, groups) = read_view(&runtime, &fixture.workspace, &group);
            assert!(groups.iter().any(|row| {
                row["at"].as_str()
                    == Some(r"main:Subsystem.Подсистема1.Interface.Group.CommandGroup\.ГруппаБ")
            }));
        }
        if name == "workspace-configuration-support" {
            assert!(table
                .iter()
                .any(|(_, _, row, _)| row == "main:Module.ManagedApplication.Body"));
            check_resource(
                &runtime,
                &fixture,
                "main:Module.ManagedApplication",
                "Ext/ManagedApplicationModule.bsl",
            );
        }
    }
    println!("| Kind | Branch | Canonical address | Evidence |");
    println!("| --- | --- | --- | --- |");
    for (kind, branch, at, evidence) in table {
        println!("| {kind} | {branch} | {at} | {evidence} |");
    }
    evidence.finish();
}
