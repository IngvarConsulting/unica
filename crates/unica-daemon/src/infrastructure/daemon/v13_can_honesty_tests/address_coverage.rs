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
