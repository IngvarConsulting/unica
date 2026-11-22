//! Собираем таблицу из ветвей канонического view на существующих фикстурах.
use super::*;
use crate::infrastructure::daemon::v13_call_graph::call_graph_support;
use unica_source_contract::format_identity::export_format_line;

const EVIDENCE_TEST_CASE: &str =
    "medium_published_document_and_bsl_branches_have_canonical_addresses";
const MD_CLASSES_NS: &str = "http://v8.1c.ru/8.3/MDClasses";
const MXL_NS: &str = "http://v8.1c.ru/8.2/data/spreadsheet";
const COMMAND_INTERFACE_NS: &str = "http://v8.1c.ru/8.3/xcf/extrnprops";

enum Evidence<'a> {
    Read {
        at: &'a str,
        external_owner_descriptor: Option<&'a str>,
    },
    ModuleRoundTrip {
        at: &'a str,
        external_owner_descriptor: Option<&'a str>,
    },
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
        self.record_evidence(
            fixture,
            origin,
            Evidence::Read {
                at,
                external_owner_descriptor: None,
            },
        );
    }

    fn record_evidence(&mut self, fixture: &Fixture, origin: (&str, &str), evidence: Evidence<'_>) {
        if self.directory.is_none() {
            return;
        }
        let (at, level, descriptor) = match evidence {
            Evidence::Read {
                at,
                external_owner_descriptor,
            } => (at, "read", external_owner_descriptor),
            Evidence::ModuleRoundTrip {
                at,
                external_owner_descriptor,
            } => (at, "roundtrip", external_owner_descriptor),
        };
        let address = parsed(at);
        let (branch, owner_segments) = address.segments().split_last().unwrap();
        let (owner, branch_kind, role) = if branch.kind() == NodeKind::CommonModule
            && branch.name().is_some()
            && level == "read"
        {
            assert!(
                owner_segments.is_empty(),
                "{at}: expected a root CommonModule"
            );
            (address.clone(), NodeKind::Module, Some("Common"))
        } else {
            let role = branch.name();
            if level == "roundtrip" {
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
            (owner, branch.kind(), role)
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
                    "role": matches!(segment.kind(), NodeKind::Interface | NodeKind::Module)
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
                    "branch": branch_kind.as_str(),
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
    let kind = address.segments().last().unwrap().kind();
    check_fixture_members_with_kind(runtime, fixture, &address, names, kind)
}

fn check_fixture_members_with_kind<'a>(
    runtime: &V5CanonicalInvocationRuntime,
    fixture: &Fixture,
    address: &QualifiedAddress,
    names: impl IntoIterator<Item = &'a str>,
    kind: NodeKind,
) -> BTreeMap<String, Value> {
    let branch = address.to_string();
    let mut expected = BTreeMap::new();
    for name in names {
        let at = address.member(name).unwrap().to_string();
        assert!(
            expected.insert(at, name).is_none(),
            "duplicate fixture member {name} of {branch}"
        );
    }
    let (_, items) = read_view(runtime, &fixture.workspace, &branch);
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
            assert_eq!(view["kind"], kind.as_str(), "{at}");
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
    assert!(root.has_tag_name((MXL_NS, "document")));
    let names: Vec<_> = root
        .children()
        .filter(|node| {
            node.has_tag_name((MXL_NS, "namedItem"))
                && node.attribute((XSI, "type")) == Some("NamedItemCells")
        })
        .map(|node| child_name(node, MXL_NS))
        .collect();
    assert!(!names.is_empty());
    let (view, _) = read_view(runtime, &fixture.workspace, owner);
    check_fixture_branches(&view, [(branch, FixtureBranch::Present(names.len()))]);
    check_fixture_members(runtime, fixture, branch, names);
    evidence.record(fixture, ORIGIN, branch);
}

fn check_native_mxl_rows(
    runtime: &V5CanonicalInvocationRuntime,
    fixture: &Fixture,
    evidence: &mut EvidenceRows,
) {
    const OWNER: &str = "cf:Report.F06Report.Template.PlatformExport";
    const AREA_BRANCH: &str = "cf:Report.F06Report.Template.PlatformExport.Area";
    const ORIGIN: (&str, &str) = (
        "workspace-mxl",
        "Reports/F06Report/Templates/PlatformExport/Ext/Template.xml",
    );
    let raw = fixture_xml(fixture, OWNER, ORIGIN.1);
    let document = roxmltree::Document::parse(raw.trim_start_matches('\u{feff}')).unwrap();
    let root = document.root_element();
    assert!(root.has_tag_name((MXL_NS, "document")));
    let mut areas = BTreeMap::new();
    for node in root
        .children()
        .filter(|node| node.has_tag_name((MXL_NS, "namedItem")))
    {
        assert_eq!(
            node.attribute(("http://www.w3.org/2001/XMLSchema-instance", "type")),
            Some("NamedItemCells")
        );
        let name = child_name(node, MXL_NS);
        let area = mxl_child(node, "area").unwrap();
        assert!(areas.insert(name, area).is_none(), "duplicate area {name}");
    }
    assert_eq!(
        areas.keys().copied().collect::<BTreeSet<_>>(),
        BTreeSet::from(["Колонки", "Подвал", "Строка", "Угол", "Шапка"])
    );
    let (view, _) = read_view(runtime, &fixture.workspace, OWNER);
    check_fixture_branches(&view, [(AREA_BRANCH, FixtureBranch::Present(areas.len()))]);
    let area_views = check_fixture_members(runtime, fixture, AREA_BRANCH, areas.keys().copied());
    evidence.record(fixture, ORIGIN, AREA_BRANCH);

    // Only row areas supply body and parameter evidence. Columns and rectangles
    // are checked above for address identity without assuming clipping semantics.
    for (name, body_count, parameter_count) in [("Шапка", 5, 1), ("Строка", 2, 3), ("Подвал", 2, 1)]
    {
        let area = areas[name];
        let value = |tag| mxl_child(area, tag).and_then(|node| node.text()).unwrap();
        assert_eq!(value("type"), "Rows");
        let begin = value("beginRow").parse::<usize>().unwrap();
        let end = value("endRow").parse::<usize>().unwrap();
        assert!(begin <= end, "{name}: invalid row range");
        let owner = parsed(AREA_BRANCH).member(name).unwrap();
        let body_branch = format!("{owner}.Body");
        let parameter_branch = format!("{owner}.Parameter");
        let parameter_address = parsed(&parameter_branch);
        let (body, parameters) = native_mxl_row_items(root, begin..=end, &parameter_address);
        assert_eq!(body.len(), body_count, "{name}: raw body count");
        assert_eq!(
            parameters.len(),
            parameter_count,
            "{name}: raw parameter count"
        );
        check_fixture_branches(
            &area_views[name],
            [
                (&body_branch, FixtureBranch::Present(body_count)),
                (&parameter_branch, FixtureBranch::Present(parameter_count)),
            ],
        );
        let (body_view, body_items) = read_view(runtime, &fixture.workspace, &body_branch);
        assert_eq!(body_view["kind"], "Body", "{body_branch}");
        assert_eq!(body_items, body, "{body_branch}");
        let (parameter_view, parameter_items) =
            read_view(runtime, &fixture.workspace, &parameter_branch);
        assert_eq!(parameter_view["kind"], "Parameter", "{parameter_branch}");
        assert_eq!(parameter_items, parameters, "{parameter_branch}");
        for item in &parameters {
            if let Some(at) = item["at"].as_str() {
                let (view, _) = read_view(runtime, &fixture.workspace, at);
                assert_eq!(view["kind"], "Parameter", "{at}");
                assert_eq!(view["title"], item["title"], "{at}");
            }
        }
        if name == "Строка" {
            let anonymous = parameters.last().unwrap();
            assert_eq!(anonymous["title"], "Сумма [tpl]");
            assert_eq!(anonymous["addressable"], false);
            assert!(anonymous.get("at").is_none());
        }
        evidence.record(fixture, ORIGIN, &body_branch);
        evidence.record(fixture, ORIGIN, &parameter_branch);
    }
}

fn mxl_child<'a, 'input>(
    parent: roxmltree::Node<'a, 'input>,
    tag: &str,
) -> Option<roxmltree::Node<'a, 'input>> {
    parent
        .children()
        .find(|node| node.has_tag_name((MXL_NS, tag)))
}

fn native_mxl_row_items(
    root: roxmltree::Node<'_, '_>,
    range: std::ops::RangeInclusive<usize>,
    parameter_address: &QualifiedAddress,
) -> (Vec<Value>, Vec<Value>) {
    const CORE_NS: &str = "http://v8.1c.ru/8.1/data/core";
    let mut rows = BTreeMap::new();
    for node in root
        .children()
        .filter(|node| node.has_tag_name((MXL_NS, "rowsItem")))
    {
        let index = mxl_child(node, "index")
            .and_then(|node| node.text())
            .unwrap()
            .parse::<usize>()
            .unwrap();
        let row = mxl_child(node, "row").unwrap();
        assert!(rows.insert(index, row).is_none(), "duplicate row {index}");
    }
    let mut body = Vec::new();
    let mut parameters = Vec::new();
    for (_, row) in rows.range(range) {
        for group in row
            .children()
            .filter(|node| node.has_tag_name((MXL_NS, "c")))
        {
            let cell = mxl_child(group, "c").unwrap();
            if let Some(name) = mxl_child(cell, "parameter").and_then(|node| node.text()) {
                parameters.push(json!({
                    "at": parameter_address.member(name).unwrap().to_string(),
                    "kind": "Parameter",
                    "title": name,
                }));
            }
            let Some(text) = mxl_child(cell, "tl")
                .and_then(|label| {
                    label
                        .children()
                        .find(|node| node.has_tag_name((CORE_NS, "item")))
                })
                .and_then(|item| {
                    item.children()
                        .find(|node| node.has_tag_name((CORE_NS, "content")))
                })
                .and_then(|node| node.text())
                .filter(|text| !text.is_empty())
            else {
                continue;
            };
            let mut template = false;
            for (name, _) in text
                .split('[')
                .skip(1)
                .filter_map(|tail| tail.split_once(']'))
            {
                assert!(
                    !name.is_empty() && !name.chars().all(|ch| ch.is_ascii_digit()),
                    "{text}"
                );
                template = true;
                parameters.push(json!({
                    "kind": "Parameter",
                    "title": format!("{name} [tpl]"),
                    "addressable": false,
                }));
            }
            body.push(json!({"index": body.len() + 1, "template": template, "text": text}));
        }
    }
    (body, parameters)
}

fn interface_value<'a>(parent: roxmltree::Node<'a, '_>, tag: &str) -> &'a str {
    parent
        .children()
        .find(|node| node.has_tag_name((COMMAND_INTERFACE_NS, tag)))
        .and_then(|node| node.text())
        .unwrap_or_else(|| panic!("missing interface {tag}"))
}

fn check_native_subsystem_interface(
    runtime: &V5CanonicalInvocationRuntime,
    fixture: &Fixture,
    fixture_name: &str,
    format: &str,
    evidence: &mut EvidenceRows,
) {
    const NS: &str = COMMAND_INTERFACE_NS;
    const READABLE_NS: &str = "http://v8.1c.ru/8.3/xcf/readable";
    const NAME: &str = "Подсистема1";
    const OWNER: &str = "main:Subsystem.Подсистема1";
    const RELATIVE: &str = "Subsystems/Подсистема1/Ext/CommandInterface.xml";
    let configuration = fixture_xml(fixture, OWNER, "Configuration.xml");
    let configuration =
        roxmltree::Document::parse(configuration.trim_start_matches('\u{feff}')).unwrap();
    let root = configuration.root_element();
    let registered = metadata_child(metadata_child(root, "Configuration"), "ChildObjects");
    assert_eq!(
        registered
            .children()
            .filter(|node| node.has_tag_name((MD_CLASSES_NS, "Subsystem")))
            .filter(|node| node.text() == Some(NAME))
            .count(),
        1,
    );
    let raw = fixture_xml(fixture, OWNER, "Subsystems/Подсистема1.xml");
    let descriptor = roxmltree::Document::parse(raw.trim_start_matches('\u{feff}')).unwrap();
    let subsystem = check_metadata_identity(descriptor.root_element(), "Subsystem", NAME);
    assert!(subsystem
        .attribute("uuid")
        .is_some_and(unica_xml_format::names::is_valid_uuid));
    let raw_content = metadata_child(metadata_child(subsystem, "Properties"), "Content");
    let content_count = raw_content
        .children()
        .filter(|node| node.has_tag_name((READABLE_NS, "Item")))
        .count();
    assert_eq!(content_count, 3);
    assert!(metadata_child(subsystem, "ChildObjects")
        .first_element_child()
        .is_none());

    let raw = fixture_xml(fixture, OWNER, RELATIVE);
    let interface = roxmltree::Document::parse(raw.trim_start_matches('\u{feff}')).unwrap();
    let root = interface.root_element();
    assert!(root.has_tag_name((NS, "CommandInterface")));
    assert_eq!(root.attribute("version"), Some(format));
    assert_eq!(
        descriptor.root_element().attribute("version"),
        root.attribute("version")
    );
    assert_eq!(
        configuration.root_element().attribute("version"),
        root.attribute("version")
    );
    let sections: Vec<_> = root.children().filter(|node| node.is_element()).collect();
    let [placement, order, groups] = sections.as_slice() else {
        panic!("native interface should have placement, command order, and group order");
    };
    assert!(placement.has_tag_name((NS, "CommandsPlacement")));
    assert!(order.has_tag_name((NS, "CommandsOrder")));
    assert!(groups.has_tag_name((NS, "GroupsOrder")));
    let group_names: Vec<_> = groups
        .children()
        .filter(|node| node.is_element())
        .map(|node| {
            assert!(node.has_tag_name((NS, "Group")));
            node.text().unwrap()
        })
        .collect();
    assert_eq!(group_names.len(), 4);
    let commands: Vec<_> = placement
        .children()
        .filter(|node| node.is_element())
        .collect();
    let [command] = commands.as_slice() else {
        panic!("native interface should have exactly one command placement");
    };
    assert!(command.has_tag_name((NS, "Command")));
    let reference = command.attribute("name").unwrap();
    let target = format!("main:{reference}");
    let placement_value = interface_value(*command, "Placement");
    let group_name = interface_value(*command, "CommandGroup");
    assert!(group_names.contains(&group_name));
    let mut orders = BTreeMap::<_, Vec<_>>::new();
    for command in order.children().filter(|node| node.is_element()) {
        assert!(command.has_tag_name((NS, "Command")));
        let name = interface_value(command, "CommandGroup");
        assert!(group_names.contains(&name));
        orders
            .entry(name)
            .or_default()
            .push(command.attribute("name").unwrap());
    }
    assert_eq!(
        orders.get(group_name).map(Vec::as_slice),
        Some(["Catalog.Справочник1.StandardCommand.Create", reference].as_slice())
    );

    let interface_at = format!("{OWNER}.Interface");
    let relation_at = format!("{OWNER}.Relation");
    let command_at = format!("{interface_at}.Command");
    let group_at = format!("{interface_at}.Group");
    let (subsystem, _) = read_view(runtime, &fixture.workspace, OWNER);
    check_fixture_branches(
        &subsystem,
        [
            (&relation_at, FixtureBranch::Present(content_count)),
            (&interface_at, FixtureBranch::Present(1)),
        ],
    );
    let (interface, _) = read_view(runtime, &fixture.workspace, &interface_at);
    assert_eq!(interface["kind"], "Interface");
    check_fixture_branches(
        &interface,
        [
            (&command_at, FixtureBranch::Present(commands.len())),
            (&group_at, FixtureBranch::Present(group_names.len())),
        ],
    );
    let (_, command_items) = read_view(runtime, &fixture.workspace, &command_at);
    let [item] = command_items.as_slice() else {
        panic!("{command_at}: expected one canonical target");
    };
    assert_eq!(item["at"], target);
    assert_eq!(item["props"]["group"], group_name);
    assert_eq!(item["props"]["placement"], placement_value);
    let (target_view, _) = read_view(runtime, &fixture.workspace, &target);
    assert_eq!(target_view["kind"], "CommonCommand");

    let (_, groups) = read_view(runtime, &fixture.workspace, &group_at);
    assert_eq!(groups.len(), group_names.len());
    let groups_at = parsed(&group_at);
    for (index, (item, name)) in groups.iter().zip(group_names).enumerate() {
        let at = groups_at.member(name).unwrap().to_string();
        let commands = orders.get(name).map_or(&[][..], Vec::as_slice);
        assert_eq!(
            *item,
            json!({
                "at": at, "kind": "Group", "title": name,
                "order": index + 1, "commands": commands.len(),
            })
        );
        let (view, items) = read_view(runtime, &fixture.workspace, &at);
        assert_eq!(view["kind"], "Group", "{at}");
        let expected: Vec<_> = commands
            .iter()
            .enumerate()
            .map(|(index, command)| json!({"order": index + 1, "command": command}))
            .collect();
        assert_eq!(items, expected, "{at}");
    }
    let origin = (fixture_name, RELATIVE);
    evidence.record(fixture, origin, &interface_at);
    if format == "2.20" {
        for at in [&command_at, &group_at] {
            evidence.record(fixture, origin, at);
        }
    } else {
        let response = invoke_on(
            runtime,
            &fixture.workspace,
            ToolIdentity::Resolve,
            json!({"at": interface_at}),
        );
        assert!(response.ok, "{interface_at}: {response:?}");
        let found = response.data.unwrap();
        assert_eq!(found["at"], interface_at);
        assert_eq!(found["kind"], "Interface");
        assert_eq!(found["path"], RELATIVE);
    }
}

fn check_module_variables_fixture(
    runtime: &V5CanonicalInvocationRuntime,
    fixture: &Fixture,
    evidence: &mut EvidenceRows,
) {
    const OWNER: &str = "main:CommonModule.Probe";
    const BRANCH: &str = "main:CommonModule.Probe.Variable";
    const ORIGIN: (&str, &str) = (
        "workspace-module-variables",
        "CommonModules/Probe/Ext/Module.bsl",
    );
    let raw = fixture_xml(fixture, OWNER, ORIGIN.1);
    let lines: Vec<_> = raw.trim_start_matches('\u{feff}').lines().collect();
    assert_eq!(lines[1], "Перем Первый Экспорт, Второй;");
    assert_eq!(&lines[3..5], ["Var Third,", "    Fourth Export;"]);
    assert_eq!(lines[7], "Перем ВнеОбласти;");
    assert_eq!(lines[9].trim(), "Перем Локальная;");
    let outer = format!("{OWNER}.Region.ОписаниеПеременных");
    let inner = format!("{outer}.Region.Вложенная");
    let expected = [
        ("Первый", true, 2, Some(outer.as_str())),
        ("Второй", false, 2, Some(outer.as_str())),
        ("Third", false, 4, Some(inner.as_str())),
        ("Fourth", true, 4, Some(inner.as_str())),
        ("ВнеОбласти", false, 8, None),
    ];
    let (owner, _) = read_view(runtime, &fixture.workspace, OWNER);
    assert_eq!(
        published_branch_counts(&owner, BRANCH).collect::<Vec<_>>(),
        [expected.len() as u64],
    );
    let variables = check_fixture_members(
        runtime,
        fixture,
        BRANCH,
        expected.iter().map(|(name, _, _, _)| *name),
    );
    for (name, exported, line, region) in expected {
        let mut props = json!({"name": name, "export": exported, "line": line});
        if let Some(region) = region {
            props["region"] = json!(region);
        }
        assert_eq!(variables[name]["props"], props, "{name}");
    }
    evidence.record(fixture, ORIGIN, BRANCH);
}

fn check_notification_binding_fixture(
    runtime: &V5CanonicalInvocationRuntime,
    fixture: &Fixture,
    evidence: &mut EvidenceRows,
) {
    const MODULE: &str = "main:Catalog.Команды.Form.ФормаСписка.Module.Form";
    const ORIGIN: (&str, &str) = (
        "workspace-notification-binding",
        "Catalogs/Команды/Forms/ФормаСписка/Ext/Form/Module.bsl",
    );
    let raw = fixture_xml(fixture, MODULE, ORIGIN.1);
    let lines: Vec<_> = raw.trim_start_matches('\u{feff}').lines().collect();
    let step = format!("{MODULE}.Method.Step");
    let start = format!("{MODULE}.Method.Start");
    let binding = format!("{step}.Binding");
    assert_eq!(lines[1], "Procedure Start()");
    assert_eq!(lines[16], "Procedure Step(Parameters = Undefined) Export");
    assert!(lines[5].contains("NotifyDescription(GetName(), ThisObject)"));
    assert!(lines[6].contains(r#"NotifyDescription("Step", GetReceiver())"#));
    let expected_ranges: Vec<_> = [
        (3, r#"New NotifyDescription("Step", ThisObject)"#),
        (4, r#"Новый ОписаниеОповещения("Step", ЭтотОбъект)"#),
    ]
    .into_iter()
    .flat_map(|(line, expression)| {
        let text = lines[line];
        text.match_indices(expression)
            .map(move |(offset, matched)| {
                let start = text[..offset].encode_utf16().count();
                let end = start + matched.encode_utf16().count();
                json!({
                    "start": {"line": line, "character": start},
                    "end": {"line": line, "character": end},
                })
            })
    })
    .collect();
    assert_eq!(expected_ranges.len(), 3);
    let (method, _) = read_view(runtime, &fixture.workspace, &step);
    let published: Vec<_> = method["branches"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row["at"] == binding)
        .cloned()
        .collect();
    assert_eq!(published, [json!({"at": binding})]);
    let (view, rows) = read_view(runtime, &fixture.workspace, &binding);
    assert_eq!(view["kind"], "Binding");
    assert_eq!(rows.len(), expected_ranges.len());
    for (row, range) in rows.iter().zip(expected_ranges) {
        // at points to the containing method, not to a notification node.
        assert_eq!(row["at"], start);
        assert_eq!(row["kind"], "Method");
        assert_eq!(row["binding"], "notifyDescription");
        assert_eq!(row["range"], range);
    }
    assert!(view["limits"].as_array().unwrap().iter().any(|limit| {
        limit
            .as_str()
            .is_some_and(|text| text.contains("dynamic name or receiver"))
    }));
    let (origin, _) = read_view(runtime, &fixture.workspace, &start);
    assert_eq!(origin["kind"], "Method");
    evidence.record(fixture, ORIGIN, &binding);
}

fn check_resource(
    runtime: &V5CanonicalInvocationRuntime,
    fixture: &Fixture,
    relative: &str,
    (at, at_from_path): (&str, &str),
) {
    let owner = parsed(at);
    let source = source_root(&fixture.workspace, owner.source_set());
    let path = source.join(relative);
    assert!(path.is_file(), "fixture witness is absent: {:?}", path);
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
    assert_eq!(data["at"], at_from_path);
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
            check_resource(runtime, fixture, &file, (&at, &at));
            evidence.record_evidence(
                fixture,
                ("workspace-external-alias", &file),
                Evidence::ModuleRoundTrip {
                    at: &at,
                    external_owner_descriptor: Some(&descriptor),
                },
            );
            check_module_body(
                runtime,
                fixture,
                evidence,
                ("workspace-external-alias", &file),
                &at,
                Some(&descriptor),
            );
        }
    }
}

fn check_configuration_module_roles(
    runtime: &V5CanonicalInvocationRuntime,
    fixture: &Fixture,
    fixture_name: &str,
    evidence: &mut EvidenceRows,
) {
    let owners: &[(&str, &str, &str, &str, &[&str])] = match fixture_name {
        "workspace-resolve-resources" => &[(
            "Document",
            "Invoice",
            "Documents",
            "Manager",
            &["Object", "Manager"],
        )],
        "workspace-data-history-settings" => &[
            (
                "Constant",
                "Setting",
                "Constants",
                "ValueManager",
                &["Manager", "ValueManager"],
            ),
            (
                "InformationRegister",
                "Цены",
                "InformationRegisters",
                "RecordSet",
                &["Manager", "RecordSet"],
            ),
        ],
        "workspace-common-command-facts" => &[(
            "CommonCommand",
            "ВсеФайлы",
            "CommonCommands",
            "Command",
            &["Command"],
        )],
        _ => unreachable!("{fixture_name} has no configuration module witness"),
    };
    assert!(fixture
        .sets
        .iter()
        .any(|(set, kind, _)| { set == "main" && *kind == SourceSetKind::Configuration }));
    let configuration = fixture_xml(fixture, "main:Configuration", "Configuration.xml");
    let configuration =
        roxmltree::Document::parse(configuration.trim_start_matches('\u{feff}')).unwrap();
    let root = configuration.root_element();
    let format = root.attribute("version");
    assert_eq!(format, Some("2.20"));
    let registrations = metadata_child(metadata_child(root, "Configuration"), "ChildObjects");
    for &(kind, name, directory, role, roles) in owners {
        assert_eq!(
            registrations.children()
                .filter(|node| node.has_tag_name((MD_CLASSES_NS, kind)) && node.text() == Some(name))
                .count(),
            1,
            "{fixture_name}: {kind}.{name} registration",
        );
        let owner = format!("main:{kind}.{name}");
        let stem = format!("{directory}/{name}");
        let raw = fixture_xml(fixture, &owner, &format!("{stem}.xml"));
        let document = roxmltree::Document::parse(raw.trim_start_matches('\u{feff}')).unwrap();
        let root = document.root_element();
        let object = check_metadata_identity(root, kind, name);
        assert_eq!(root.attribute("version"), format);
        assert!(object
            .attribute("uuid")
            .is_some_and(unica_xml_format::names::is_valid_uuid));
        check_configuration_module_role(
            runtime,
            fixture,
            evidence,
            &owner,
            role,
            roles,
            (fixture_name, &format!("{stem}/Ext/{role}Module.bsl")),
        );
        if kind == "Document" {
            let forms = metadata_child(object, "ChildObjects")
                .children()
                .filter(|node| {
                    node.has_tag_name((MD_CLASSES_NS, "Form")) && node.text() == Some("Main")
                })
                .count();
            assert_eq!(forms, 1, "{owner}: Form.Main registration");
            let raw = fixture_xml(fixture, &owner, &format!("{stem}/Forms/Main.xml"));
            let document = roxmltree::Document::parse(raw.trim_start_matches('\u{feff}')).unwrap();
            let root = document.root_element();
            let form = check_metadata_identity(root, "Form", "Main");
            assert_eq!(root.attribute("version"), format);
            assert!(form
                .attribute("uuid")
                .is_some_and(unica_xml_format::names::is_valid_uuid));
            check_configuration_module_role(
                runtime,
                fixture,
                evidence,
                &format!("{owner}.Form.Main"),
                "Form",
                &["Form"],
                (
                    fixture_name,
                    &format!("{stem}/Forms/Main/Ext/Form/Module.bsl"),
                ),
            );
        }
    }
}

fn check_configuration_module_role(
    runtime: &V5CanonicalInvocationRuntime,
    fixture: &Fixture,
    evidence: &mut EvidenceRows,
    parent: &str,
    role: &str,
    roles: &[&str],
    origin: (&str, &str),
) {
    let branch = format!("{parent}.Module");
    let (view, _) = read_view(runtime, &fixture.workspace, parent);
    assert_eq!(
        published_branch_counts(&view, &branch).collect::<Vec<_>>(),
        [roles.len() as u64],
        "{parent}: {branch}",
    );
    let modules = check_fixture_members(runtime, fixture, &branch, roles.iter().copied());
    assert_eq!(modules[role]["props"]["role"], role);
    let at = format!("{branch}.{role}");
    check_resource(runtime, fixture, origin.1, (&at, &at));
    evidence.record_evidence(
        fixture,
        origin,
        Evidence::ModuleRoundTrip {
            at: &at,
            external_owner_descriptor: None,
        },
    );
    if role != "Form" {
        // Configuration Form bodies are covered by the native nested fixtures.
        check_module_body(runtime, fixture, evidence, origin, &at, None);
    }
}

fn check_module_body(
    runtime: &V5CanonicalInvocationRuntime,
    fixture: &Fixture,
    evidence: &mut EvidenceRows,
    origin: (&str, &str),
    module: &str,
    external_owner_descriptor: Option<&str>,
) {
    let at = format!("{module}.Body");
    let source = fixture_xml(fixture, &at, origin.1);
    let response = invoke_on(
        runtime,
        &fixture.workspace,
        ToolIdentity::View,
        json!({"at":at}),
    );
    assert!(response.ok, "{at}: {response:?}");
    let data = response.data.unwrap();
    assert_eq!(data["at"], at);
    assert_eq!(data["kind"], "Body");
    assert_eq!(
        response.source_text.as_deref(),
        Some(source.trim_start_matches('\u{feff}')),
        "{at}"
    );
    evidence.record_evidence(
        fixture,
        origin,
        Evidence::Read {
            at: &at,
            external_owner_descriptor,
        },
    );
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

    let descriptor_path = format!("CommonModules/{NAME}.xml");
    let descriptor = fixture_xml(fixture, COLL, &descriptor_path);
    let descriptor = roxmltree::Document::parse(descriptor.trim_start_matches('\u{feff}')).unwrap();
    let descriptor_root = descriptor.root_element();
    check_metadata_identity(descriptor_root, KIND, NAME);
    assert_eq!(
        descriptor_root.attribute("version"),
        root.attribute("version")
    );
    let module_path = format!("CommonModules/{NAME}/Ext/Module.bsl");
    let module = fixture_xml(fixture, COLL, &module_path);
    let source = module.trim_start_matches('\u{feff}');
    assert!(!source.is_empty());

    let (view, _) = read_view(runtime, &fixture.workspace, "ext:Configuration");
    let counts: Vec<_> = published_branch_counts(&view, COLL).collect();
    assert_eq!(
        counts,
        [registered.len() as u64],
        "ext:Configuration: {COLL}"
    );
    // The registration is CommonModule. Direct view opens its BSL module.
    check_fixture_members_with_kind(
        runtime,
        fixture,
        &parsed(COLL),
        registered,
        NodeKind::Module,
    );
    let at = format!("{COLL}.{NAME}");
    let body = format!("{at}.Body");
    check_resource(runtime, fixture, &descriptor_path, (&at, &at));
    check_resource(runtime, fixture, &module_path, (&body, &at));
    let response = invoke_on(
        runtime,
        &fixture.workspace,
        ToolIdentity::View,
        json!({"at":body}),
    );
    assert!(response.ok, "{body}: {response:?}");
    let data = response.data.unwrap();
    assert_eq!(data["at"], body);
    assert_eq!(data["kind"], "Body");
    assert_eq!(response.source_text.as_deref(), Some(source), "{body}");
    evidence.record(fixture, ("workspace-code", "Configuration.xml"), COLL);
    evidence.record(fixture, ("workspace-code", &module_path), &at);
    evidence.record(fixture, ("workspace-code", &module_path), &body);
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
        ("workspace-resolve-resources", &[][..]),
        ("workspace-data-history-settings", &[][..]),
        ("workspace-common-command-facts", &[][..]),
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
            "workspace-mxl" => {
                check_mxl_xml(&runtime, &fixture, &mut evidence);
                check_native_mxl_rows(&runtime, &fixture, &mut evidence);
            }
            "workspace-command-groups" => {
                check_native_subsystem_interface(&runtime, &fixture, name, "2.20", &mut evidence)
            }
            "workspace-module-variables" => {
                check_module_variables_fixture(&runtime, &fixture, &mut evidence)
            }
            "workspace-notification-binding" => {
                check_notification_binding_fixture(&runtime, &fixture, &mut evidence)
            }
            "workspace-external-alias" => check_external_modules(&runtime, &fixture, &mut evidence),
            "workspace-code" => check_extension_common_module(&runtime, &fixture, &mut evidence),
            "workspace-resolve-resources"
            | "workspace-data-history-settings"
            | "workspace-common-command-facts" => {
                check_configuration_module_roles(&runtime, &fixture, name, &mut evidence)
            }
            _ => {}
        }
        if name == "workspace-configuration-support" {
            assert!(table
                .iter()
                .any(|(_, _, row, _)| row == "main:Module.ManagedApplication.Body"));
            check_resource(
                &runtime,
                &fixture,
                "Ext/ManagedApplicationModule.bsl",
                (
                    "main:Module.ManagedApplication",
                    "main:Module.ManagedApplication",
                ),
            );
        }
    }
    for (name, format) in [
        ("workspace-native-subsystem-interface-2.21", "2.21"),
        ("workspace-native-subsystem-interface-2.22", "2.22"),
    ] {
        let fixture = fixture_with_profile(
            name,
            unica_workspace::source_profile::platform_xml_source_profile,
        );
        let runtime = V5CanonicalInvocationRuntime::new(
            Arc::new(CanonicalV13ReadService::default()),
            Arc::new(TokioClock),
        );
        check_native_subsystem_interface(&runtime, &fixture, name, format, &mut evidence);
    }
    println!("| Kind | Branch | Canonical address | Evidence |");
    println!("| --- | --- | --- | --- |");
    for (kind, branch, at, evidence) in table {
        println!("| {kind} | {branch} | {at} | {evidence} |");
    }
    evidence.finish();
}
