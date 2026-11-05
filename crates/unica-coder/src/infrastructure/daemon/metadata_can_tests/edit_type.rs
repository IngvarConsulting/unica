use super::*;
const OWNERS: [(&str, &str); 2] = [
    ("main:Catalog.Items", "Catalogs/Items.xml"),
    (
        "main:InformationRegister.History",
        "InformationRegisters/History.xml",
    ),
];
fn edit(at: &str, value: Value) -> Value {
    json!({"at":at,"ops":[{"op":"props.set","args":{"values":{"EditType":value}}}]})
}
fn without_mode(bytes: &[u8]) -> Vec<u8> {
    let source = std::str::from_utf8(bytes).unwrap();
    let body = source.trim_start_matches('\u{feff}');
    let prefix = source.len() - body.len();
    let doc = roxmltree::Document::parse(body).unwrap();
    let node = doc
        .descendants()
        .find(|n| n.has_tag_name(("http://v8.1c.ru/8.3/MDClasses", "EditType")))
        .unwrap();
    let range = node.range();
    [
        &bytes[..range.start + prefix],
        b"<EditType/>",
        &bytes[range.end + prefix..],
    ]
    .concat()
}
#[test]
fn metadata_edit_type_changes_both_owners_all_native_modes_and_keeps_form_members_and_module_bytes()
{
    let fixture = Fixture::new("workspace-metadata-edit-type");
    let runtime = runtime();
    let original = unica_workspace::test_support::tree_snapshot(&fixture.source);
    for (at, relative) in OWNERS {
        let can = contract(&runtime, &fixture, at, "props.set");
        assert_eq!(
            can["argsSchema"]["properties"]["values"]["properties"]["EditType"],
            json!({"type":"string","enum":["InDialog","InList","BothWays"]})
        );
        for (index, value) in ["InList", "BothWays", "InDialog", "InDialog"]
            .into_iter()
            .enumerate()
        {
            let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
            let result = publish(&runtime, &fixture, edit(at, json!(value)));
            assert_eq!(view(&runtime, &fixture, at)["props"]["EditType"], value);
            let mut after = unica_workspace::test_support::tree_snapshot(&fixture.source);
            let changed = after.remove(&PathBuf::from(relative)).unwrap();
            let mut expected = before.clone();
            let old = expected.remove(&PathBuf::from(relative)).unwrap();
            assert_eq!(after, expected, "only the selected descriptor changes");
            assert_eq!(
                without_mode(&changed),
                without_mode(&old),
                "identity, form assignments, members and all other descriptor properties survive"
            );
            if index == 3 {
                assert_eq!(result.data.as_ref().unwrap()["effects"], 0);
                assert_eq!(
                    unica_workspace::test_support::tree_snapshot(&fixture.source),
                    before
                );
            }
        }
    }
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&fixture.source),
        original,
        "returning both modes restores the exact source image"
    );
}
#[test]
fn metadata_edit_type_bad_values_owners_and_existing_shapes_refuse_without_writes() {
    for (at, relative) in OWNERS {
        for value in [
            json!(false),
            json!(null),
            json!(9),
            json!(""),
            json!("Both"),
            json!("inlist"),
            json!("Unknown"),
        ] {
            let fixture = Fixture::new("workspace-metadata-edit-type");
            let runtime = runtime();
            let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
            let refused = call(&runtime, &fixture, ToolIdentity::Apply, edit(at, value));
            assert!(!refused.ok, "{refused:?}");
            assert_eq!(
                unica_workspace::test_support::tree_snapshot(&fixture.source),
                before
            );
        }
        for field in [
            "",
            "<EditType/>",
            "<EditType>Unknown</EditType>",
            "<EditType>InDialog</EditType><EditType>InList</EditType>",
            "<EditType xmlns=\"urn:foreign\">InDialog</EditType>",
            "<EditType xsi:nil=\"true\"/>",
            "<EditType><Nested/></EditType>",
        ] {
            let fixture = Fixture::new("workspace-metadata-edit-type");
            let runtime = runtime();
            let path = fixture.source.join(relative);
            let old = std::fs::read_to_string(&path).unwrap();
            assert!(old.contains("<EditType>InDialog</EditType>"));
            std::fs::write(path, old.replace("<EditType>InDialog</EditType>", field)).unwrap();
            let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
            let refused = call(
                &runtime,
                &fixture,
                ToolIdentity::Apply,
                edit(at, json!("InList")),
            );
            assert!(!refused.ok, "{at}/{field}: {refused:?}");
            assert_eq!(
                unica_workspace::test_support::tree_snapshot(&fixture.source),
                before
            );
        }
    }
    for at in [
        "main:Configuration",
        "main:Language.Русский",
        "main:Catalog.Items.Attribute.Detail",
        "absent:Catalog.Items",
    ] {
        let fixture = Fixture::new("workspace-metadata-edit-type");
        let runtime = runtime();
        let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
        let refused = call(
            &runtime,
            &fixture,
            ToolIdentity::Apply,
            edit(at, json!("InList")),
        );
        assert!(!refused.ok, "{at}: {refused:?}");
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.source),
            before
        );
    }
}
#[test]
fn metadata_edit_type_saved_plan_guards_both_descriptors_and_owner_registration() {
    for (at, relative) in OWNERS {
        for root in [false, true] {
            let fixture = Fixture::new("workspace-metadata-edit-type");
            let runtime = runtime();
            let p = call(
                &runtime,
                &fixture,
                ToolIdentity::Apply,
                edit(at, json!("InList")),
            );
            assert!(p.ok, "{p:?}");
            let token = p.data.unwrap()["executionToken"].clone();
            let path = fixture
                .source
                .join(if root { "Configuration.xml" } else { relative });
            let mut bytes = std::fs::read(&path).unwrap();
            bytes.extend_from_slice(b"\n");
            std::fs::write(path, bytes).unwrap();
            let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
            let stale = call(
                &runtime,
                &fixture,
                ToolIdentity::Apply,
                json!({"executionToken":token}),
            );
            assert!(!stale.ok, "{stale:?}");
            assert_eq!(stale.diagnostics[0]["code"], "stale_revision", "{stale:?}");
            assert_eq!(
                unica_workspace::test_support::tree_snapshot(&fixture.source),
                before
            );
        }
    }
}
#[test]
fn metadata_edit_type_noop_preserves_entities_comments_prefix_and_extensions_do_not_offer_it() {
    for (at, relative) in OWNERS {
        let fixture = Fixture::new("workspace-metadata-edit-type");
        let runtime = runtime();
        let path = fixture.source.join(relative);
        let old = std::fs::read_to_string(&path).unwrap();
        std::fs::write(path,old.replace("<EditType>InDialog</EditType>","<m:EditType xmlns:m=\"http://v8.1c.ru/8.3/MDClasses\">In&#68;ialog<!--keep--></m:EditType>")).unwrap();
        let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
        let r = publish(&runtime, &fixture, edit(at, json!("InDialog")));
        assert_eq!(r.data.unwrap()["effects"], 0);
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.source),
            before
        );
    }
    let fixture = Fixture::with_extension();
    let runtime = runtime();
    publish(
        &runtime,
        &fixture,
        json!({"at":"ext:Configuration","ops":[{"op":"object.create","args":{"values":{"kind":"Catalog","name":"Boundary"}}}]}),
    );
    let can = contract(&runtime, &fixture, "ext:Catalog.Boundary", "props.set");
    assert!(can["argsSchema"]["properties"]["values"]["properties"]
        .get("EditType")
        .is_none());
    publish(
        &runtime,
        &fixture,
        json!({"at":"ext:Catalog.Boundary","ops":[{"op":"props.set","args":{"values":{"Comment":"independent"}}}]}),
    );
    let before = unica_workspace::test_support::tree_snapshot(&fixture.workspace.join("src-ext"));
    let refused = call(
        &runtime,
        &fixture,
        ToolIdentity::Apply,
        edit("ext:Catalog.Boundary", json!("InList")),
    );
    assert!(!refused.ok, "{refused:?}");
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&fixture.workspace.join("src-ext")),
        before
    );
}
