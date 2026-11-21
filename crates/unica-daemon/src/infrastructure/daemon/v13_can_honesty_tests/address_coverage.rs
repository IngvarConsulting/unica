//! Собираем таблицу из ветвей канонического view на существующих фикстурах.
use super::*;
use crate::infrastructure::daemon::v13_call_graph::call_graph_support;

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

fn check_role_xml(runtime: &V5CanonicalInvocationRuntime, fixture: &Fixture) {
    const NS: &str = "http://v8.1c.ru/8.2/roles";
    let owner = "main:Role.Reader";
    let branch = "main:Role.Reader.Right";
    let raw = fixture_xml(fixture, owner, "Roles/Reader/Ext/Rights.xml");
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
    for (name, rights) in restrictions {
        let view = &objects[&name];
        let at = format!("{}.RLS", view["at"].as_str().unwrap());
        check_fixture_branches(view, [(&at, FixtureBranch::from_count(rights.len()))]);
        if !rights.is_empty() {
            check_fixture_members(runtime, fixture, &at, rights);
        }
    }
}

fn check_xdto_xml(runtime: &V5CanonicalInvocationRuntime, fixture: &Fixture) {
    const NS: &str = "http://v8.1c.ru/8.1/xdto";
    let owner = "main:XDTOPackage.ПримерФормата";
    let raw = fixture_xml(fixture, owner, "XDTOPackages/ПримерФормата/Ext/Package.bin");
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
    assert_eq!(view["props"]["namespace"], namespace);
    let nodes = check_fixture_members(
        runtime,
        fixture,
        &types_at,
        types.iter().map(|node| node.attribute("name").unwrap()),
    );
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
        }
    }
    assert!(counts.contains(&0) && counts.iter().any(|count| *count > 0));
}

fn check_mxl_xml(runtime: &V5CanonicalInvocationRuntime, fixture: &Fixture) {
    const NS: &str = "http://v8.1c.ru/8.2/data/spreadsheet";
    const XSI: &str = "http://www.w3.org/2001/XMLSchema-instance";
    let owner = "cf:Report.F06Report.Template.F06Template";
    let branch = "cf:Report.F06Report.Template.F06Template.Area";
    let raw = fixture_xml(
        fixture,
        owner,
        "Reports/F06Report/Templates/F06Template/Ext/Template.xml",
    );
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
    assert_eq!(by_address.data.unwrap()["path"].as_str(), Some(relative));

    let by_path = invoke_on(
        runtime,
        &fixture.workspace,
        ToolIdentity::Resolve,
        json!({"path":path.to_string_lossy()}),
    );
    assert!(by_path.ok, "{relative}: {by_path:?}");
    assert_eq!(by_path.data.unwrap()["at"].as_str(), Some(at));
}

/// rule: CTR.SOURCE.QUALIFIED-LOGICAL-ADDRESS
/// rule: INV.SOURCE.LOGICAL-BRANCH-COUNT
/// rule: INV.SOURCE.ROLE-RIGHT-ADDRESSES
#[test]
fn medium_published_document_and_bsl_branches_have_canonical_addresses() {
    let mut table = BTreeSet::new();
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
            "workspace-role-comment-subsystem-help" => check_role_xml(&runtime, &fixture),
            "workspace-xdto-anonymous" => check_xdto_xml(&runtime, &fixture),
            "workspace-mxl" => check_mxl_xml(&runtime, &fixture),
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
        if name == "workspace-external-alias" {
            for (owner, stem) in [
                ("epf:ExternalDataProcessor.SyntheticProcessor", "source"),
                ("erf:ExternalReport.SyntheticReport", "report-source"),
            ] {
                for (suffix, file) in [
                    (".Module.Object", "Ext/ObjectModule.bsl"),
                    (".Form.Main.Module.Form", "Forms/Main/Ext/Form/Module.bsl"),
                ] {
                    check_resource(
                        &runtime,
                        &fixture,
                        &format!("{owner}{suffix}"),
                        &format!("{stem}/{file}"),
                    );
                }
            }
        }
    }
    println!("| Kind | Branch | Canonical address | Evidence |");
    println!("| --- | --- | --- | --- |");
    for (kind, branch, at, evidence) in table {
        println!("| {kind} | {branch} | {at} | {evidence} |");
    }
}
