use super::*;

const OWNER: &str = "main:Catalog.Codes";
const FILE: &str = "Catalogs/Codes/Ext/Predefined.xml";
const ROOT_ID: &str = "aaaaaaaa-0000-4000-8000-000000000001";
const BRANCH_ID: &str = "bbbbbbbb-0000-4000-8000-000000000002";
const LEAF_ID: &str = "cccccccc-0000-4000-8000-000000000003";
const ADDED_ID: &str = "ffffffff-0000-4000-8000-000000000006";

fn item(id: &str, name: &str, description: &str, children: &str) -> String {
    let folder = !children.is_empty();
    let children = if folder {
        format!("<ChildItems>{children}</ChildItems>")
    } else {
        String::new()
    };
    format!(
        r#"<Item id="{id}"><Name>{name}</Name><Code/><Description>{description}</Description><IsFolder>{folder}</IsFolder>{children}</Item>"#
    )
}

fn nested_fixture() -> (Fixture, V5CanonicalInvocationRuntime, String, String) {
    let fixture = Fixture::blank();
    let context =
        crate::infrastructure::workspace::discover_workspace(Some(fixture.workspace.clone()))
            .unwrap();
    assert!(
        context.cache_root.starts_with(&fixture.workspace),
        "workspace snapshot must include the actual cache root: {}",
        context.cache_root.display()
    );
    let configuration = fixture.source.join("Configuration.xml");
    let xml = std::fs::read_to_string(&configuration).unwrap();
    std::fs::write(
        configuration,
        xml.replace("<Name>Store</Name>", "<Name>Store</Name><Comment/>"),
    )
    .unwrap();
    let runtime = runtime();
    publish(
        &runtime,
        &fixture,
        json!({"at":"main:Configuration","ops":[{"op":"object.create","args":{"values":{"kind":"Catalog","name":"Codes"}}}]}),
    );
    let leaf = item(LEAF_ID, "Leaf", "leaf", "");
    let branch = item(BRANCH_ID, "Branch", "branch", &leaf);
    let peer = item("dddddddd-0000-4000-8000-000000000004", "Peer", "peer", "");
    let root = item(ROOT_ID, "Root", "root", &format!("{branch}{peer}"));
    let sibling = item(
        "eeeeeeee-0000-4000-8000-000000000005",
        "Sibling",
        "sibling",
        "",
    );
    let xml = format!(
        r#"<PredefinedData xmlns="http://v8.1c.ru/8.3/xcf/predef" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:type="CatalogPredefinedItems" version="2.20">{root}{sibling}</PredefinedData>"#
    );
    let path = fixture.source.join(FILE);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, &xml).unwrap();
    (fixture, runtime, xml, branch)
}

fn warm_owner(runtime: &V5CanonicalInvocationRuntime, fixture: &Fixture) {
    let read = call(runtime, fixture, ToolIdentity::View, json!({"at":OWNER}));
    if !read.ok {
        assert_eq!(
            read.diagnostics[0]["detailCode"], "source_unreadable",
            "{read:?}"
        );
    }
}

fn publish_preserving_preview(
    runtime: &V5CanonicalInvocationRuntime,
    fixture: &Fixture,
    args: Value,
) {
    warm_owner(runtime, fixture);
    let before = unica_workspace::test_support::tree_snapshot(&fixture.workspace);
    let preview = call(runtime, fixture, ToolIdentity::Apply, args.clone());
    assert!(preview.ok, "{preview:?}");
    let repeated = call(runtime, fixture, ToolIdentity::Apply, args);
    assert!(repeated.ok, "{repeated:?}");
    assert_eq!(
        preview.data.as_ref().unwrap()["planHash"],
        repeated.data.as_ref().unwrap()["planHash"]
    );
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&fixture.workspace),
        before,
        "preview preserves source and workspace cache"
    );
    let token = preview.data.unwrap()["executionToken"].clone();
    let result = call(
        runtime,
        fixture,
        ToolIdentity::Apply,
        json!({"executionToken":token}),
    );
    assert!(result.ok, "{result:?}");
}

fn assert_only_predefined_changed(fixture: &Fixture, before: &SourceImage, expected: &[u8]) {
    let mut expected_tree = before.clone();
    expected_tree.insert(PathBuf::from(FILE), expected.to_vec());
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&fixture.source),
        expected_tree
    );
}

#[test]
fn canonical_predefined_apply_adds_at_root_and_updates_and_removes_nested_identity() {
    let (fixture, runtime, original, branch) = nested_fixture();
    let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
    publish_preserving_preview(
        &runtime,
        &fixture,
        json!({"at":OWNER,"ops":[{"op":"predefinedItem.add","args":{"items":[{"id":ADDED_ID,"name":"Added","description":"added"}]}}]}),
    );
    let added = std::fs::read_to_string(fixture.source.join(FILE)).unwrap();
    let doc = roxmltree::Document::parse(&added).unwrap();
    let roots = doc
        .root_element()
        .children()
        .filter(|node| node.is_element())
        .collect::<Vec<_>>();
    assert_eq!(
        roots
            .iter()
            .map(|node| node.attribute("id").unwrap())
            .collect::<Vec<_>>(),
        vec![ROOT_ID, "eeeeeeee-0000-4000-8000-000000000005", ADDED_ID]
    );
    // Existing subtrees remain byte-identical; the added UUID belongs directly
    // to PredefinedData rather than the last nested ChildItems container.
    let old = roxmltree::Document::parse(&original).unwrap();
    for (old, new) in old
        .root_element()
        .children()
        .filter(|node| node.is_element())
        .zip(&roots)
    {
        assert_eq!(&original[old.range()], &added[new.range()]);
    }
    assert_eq!(
        roots[2]
            .children()
            .find(|node| node.has_tag_name(("http://v8.1c.ru/8.3/xcf/predef", "Description")))
            .unwrap()
            .text(),
        Some("added")
    );
    assert_only_predefined_changed(&fixture, &before, added.as_bytes());
    let detail = view(
        &runtime,
        &fixture,
        "main:Catalog.Codes.PredefinedItem.Added",
    );
    assert_eq!(detail["props"]["id"], ADDED_ID);
    assert!(detail["props"].get("parentId").is_none_or(Value::is_null));

    let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
    publish_preserving_preview(
        &runtime,
        &fixture,
        json!({"at":"main:Catalog.Codes.PredefinedItem.Branch","ops":[{"op":"predefinedItem.set","args":{"values":{"description":"changed"}}}]}),
    );
    let updated = added.replace(
        "<Description>branch</Description>",
        "<Description>changed</Description>",
    );
    assert_only_predefined_changed(&fixture, &before, updated.as_bytes());
    let detail = view(
        &runtime,
        &fixture,
        "main:Catalog.Codes.PredefinedItem.Branch",
    );
    assert_eq!(detail["props"]["id"], BRANCH_ID);
    assert_eq!(detail["props"]["parentId"], ROOT_ID);
    assert_eq!(detail["props"]["description"], "changed");
    assert_eq!(
        view(&runtime, &fixture, "main:Catalog.Codes.PredefinedItem.Leaf")["props"]["parentId"],
        BRANCH_ID
    );

    let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
    publish_preserving_preview(
        &runtime,
        &fixture,
        json!({"at":OWNER,"ops":[{"op":"predefinedItem.remove","args":{"at":"main:Catalog.Codes.PredefinedItem.Branch"}}]}),
    );
    let removed = updated.replace(
        &branch.replace(
            "<Description>branch</Description>",
            "<Description>changed</Description>",
        ),
        "",
    );
    assert_only_predefined_changed(&fixture, &before, removed.as_bytes());
    let items = view(&runtime, &fixture, "main:Catalog.Codes.PredefinedItem");
    assert_eq!(
        items["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["props"]["id"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec![
            ROOT_ID,
            "dddddddd-0000-4000-8000-000000000004",
            "eeeeeeee-0000-4000-8000-000000000005",
            ADDED_ID
        ]
    );
    for name in ["Branch", "Leaf"] {
        let missing = call(
            &runtime,
            &fixture,
            ToolIdentity::View,
            json!({"at":format!("{OWNER}.PredefinedItem.{name}")}),
        );
        assert!(!missing.ok, "{missing:?}");
        assert_eq!(missing.diagnostics[0]["code"], "not_found");
    }
}

#[test]
fn canonical_predefined_apply_refuses_duplicate_source_uuids_without_mutation() {
    for duplicate in [ROOT_ID.to_string(), ROOT_ID.to_ascii_uppercase()] {
        let (fixture, runtime, xml, _) = nested_fixture();
        std::fs::write(
            fixture.source.join(FILE),
            xml.replace(BRANCH_ID, &duplicate),
        )
        .unwrap();
        warm_owner(&runtime, &fixture);
        let guard_args = json!({"at":"main:Configuration","ops":[{"op":"props.set","args":{"values":{"Comment":"guard publication"}}}]});
        let guard = call(&runtime, &fixture, ToolIdentity::Apply, guard_args.clone());
        assert!(guard.ok, "{guard:?}");
        let before = unica_workspace::test_support::tree_snapshot(&fixture.workspace);
        for (at, op, args) in [
            (
                OWNER,
                "predefinedItem.add",
                json!({"items":[{"id":ADDED_ID,"name":"Added"}]}),
            ),
            (
                "main:Catalog.Codes.PredefinedItem.Leaf",
                "predefinedItem.set",
                json!({"values":{"description":"must not write"}}),
            ),
            (
                "main:Catalog.Codes.PredefinedItem.Branch",
                "predefinedItem.remove",
                json!({}),
            ),
        ] {
            let refused = call(
                &runtime,
                &fixture,
                ToolIdentity::Apply,
                json!({"at":at,"ops":[{"op":op,"args":args}]}),
            );
            assert!(!refused.ok, "{op}: {refused:?}");
            assert!(refused.changed.is_empty());
            assert!(refused
                .data
                .as_ref()
                .is_none_or(|data| data.get("executionToken").is_none()));
            assert!(
                refused
                    .diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic["message"]
                        .as_str()
                        .is_some_and(|message| message.contains("UUID is duplicated"))),
                "{refused:?}"
            );
            warm_owner(&runtime, &fixture);
            assert_eq!(
                unica_workspace::test_support::tree_snapshot(&fixture.workspace),
                before,
                "{op} refusal preserves source and workspace cache"
            );
        }
        let repeated = call(&runtime, &fixture, ToolIdentity::Apply, guard_args);
        assert!(repeated.ok, "{repeated:?}");
        assert_eq!(
            guard.data.as_ref().unwrap()["planHash"],
            repeated.data.as_ref().unwrap()["planHash"]
        );
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.workspace),
            before
        );
        let executed = call(
            &runtime,
            &fixture,
            ToolIdentity::Apply,
            json!({"executionToken":guard.data.unwrap()["executionToken"]}),
        );
        assert!(
            executed.ok,
            "refusals must not invalidate an already prepared source fence: {executed:?}"
        );
    }
}
