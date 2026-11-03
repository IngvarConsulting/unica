use super::*;
fn field_property_range(xml: &str, tag: &str, name: &str, key: &str) -> std::ops::Range<usize> {
    let doc = roxmltree::Document::parse(xml).unwrap();
    let node = doc
        .descendants()
        .find(|n| {
            n.is_element()
                && n.tag_name().name() == tag
                && n.children()
                    .find(|c| c.is_element() && c.tag_name().name() == "Properties")
                    .and_then(|p| {
                        p.children()
                            .find(|c| c.is_element() && c.tag_name().name() == "Name")
                    })
                    .and_then(|n| n.text())
                    == Some(name)
        })
        .unwrap();
    node.children()
        .find(|n| n.is_element() && n.tag_name().name() == "Properties")
        .unwrap()
        .children()
        .find(|n| n.is_element() && n.tag_name().name() == key)
        .unwrap()
        .range()
}
fn verify_member_history(at: &str, file: &str, tag: &str, name: &str, op: &str) {
    let fixture = Fixture::new("workspace-member-search-history");
    let runtime = runtime();
    let owner = PathBuf::from(file);
    let path = fixture.source.join(&owner);
    let original = unica_workspace::test_support::tree_snapshot(&fixture.source);
    let initial = String::from_utf8(original[&owner].clone()).unwrap();
    let can = contract(&runtime, &fixture, at, op);
    for (key, xml_key) in [
        ("fullTextSearch", "FullTextSearch"),
        ("dataHistory", "DataHistory"),
    ] {
        assert_eq!(
            can["argsSchema"]["properties"]["values"]["properties"][key]["enum"],
            json!(["Use", "DontUse"])
        );
        let range = field_property_range(&initial, tag, name, xml_key);
        let old = &initial[range.clone()];
        assert_eq!(old, format!("<{xml_key}>Use</{xml_key}>"));
        for value in ["DontUse", "Use", "Use"] {
            publish(
                &runtime,
                &fixture,
                json!({"at":at,"ops":[{"op":op,"args":{"values":{key:value}}}]}),
            );
            let mut changed = initial.clone();
            changed.replace_range(range.clone(), &format!("<{xml_key}>{value}</{xml_key}>"));
            let mut expected = original.clone();
            expected.insert(owner.clone(), changed.into_bytes());
            assert_eq!(
                unica_workspace::test_support::tree_snapshot(&fixture.source),
                expected
            );
            assert_eq!(view(&runtime, &fixture, at)["props"][xml_key], value);
            let collection = view(&runtime, &fixture, at.rsplit_once('.').unwrap().0);
            let row = collection["items"]
                .as_array()
                .unwrap()
                .iter()
                .find(|row| row["at"] == at)
                .unwrap();
            assert_eq!(row["props"][xml_key], value);
        }
        for value in [json!(true), json!(1), json!("Unknown"), Value::Null] {
            let r = call(
                &runtime,
                &fixture,
                ToolIdentity::Apply,
                json!({"at":at,"ops":[{"op":op,"args":{"values":{key:value}}}]}),
            );
            assert!(!r.ok, "{r:?}");
            assert_eq!(
                unica_workspace::test_support::tree_snapshot(&fixture.source),
                original
            );
        }
        for damaged in [
            String::new(),
            format!("<{xml_key}/>"),
            format!("<{xml_key}>Unknown</{xml_key}>"),
            format!("<{xml_key} xmlns=\"foreign\">Use</{xml_key}>"),
            format!("{old}{old}"),
            format!("<{xml_key}><value>Use</value></{xml_key}>"),
        ] {
            let mut broken = initial.clone();
            broken.replace_range(range.clone(), &damaged);
            std::fs::write(&path, broken).unwrap();
            let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
            let r = call(
                &runtime,
                &fixture,
                ToolIdentity::Apply,
                json!({"at":at,"ops":[{"op":op,"args":{"values":{key:"DontUse"}}}]}),
            );
            assert!(!r.ok, "{xml_key} {damaged}: {r:?}");
            assert_eq!(
                unica_workspace::test_support::tree_snapshot(&fixture.source),
                before
            );
        }
        std::fs::write(&path, &initial).unwrap();
    }
}
#[test]
fn member_search_history_catalog_attribute() {
    verify_member_history(
        "main:Catalog.Codes.Attribute.Key",
        "Catalogs/Codes.xml",
        "Attribute",
        "Key",
        "attribute.set",
    );
}
#[test]
fn member_search_history_tabular_attribute() {
    verify_member_history(
        "main:Catalog.Codes.TabularSection.Rows.Attribute.Text",
        "Catalogs/Codes.xml",
        "Attribute",
        "Text",
        "attribute.set",
    );
}
#[test]
fn member_search_history_register_dimension() {
    verify_member_history(
        "main:InformationRegister.Цены.Dimension.Товар",
        "InformationRegisters/Цены.xml",
        "Dimension",
        "Товар",
        "dimension.set",
    );
}
#[test]
fn member_search_history_register_resource() {
    verify_member_history(
        "main:InformationRegister.Цены.Resource.Цена",
        "InformationRegisters/Цены.xml",
        "Resource",
        "Цена",
        "resource.set",
    );
}
