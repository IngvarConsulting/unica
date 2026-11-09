use super::*;
fn verify_search_input(key: &str, values: &[&str], initial_value: &str) {
    for (kind, directory, name) in [
        ("Catalog", "Catalogs", "Codes"),
        ("Document", "Documents", "Приход"),
    ] {
        let fixture = Fixture::new("workspace-search-input-settings");
        let runtime = runtime();
        let at = format!("main:{kind}.{name}");
        let owner = PathBuf::from(format!("{directory}/{name}.xml"));
        let path = fixture.source.join(&owner);
        let original = unica_workspace::test_support::tree_snapshot(&fixture.source);
        let initial = String::from_utf8(original[&owner].clone()).unwrap();
        let old = format!("\n\t\t\t<{key}>{initial_value}</{key}>");
        assert_eq!(initial.matches(&old).count(), 1);
        let can = contract(&runtime, &fixture, &at, "props.set");
        assert_eq!(
            can["argsSchema"]["properties"]["values"]["properties"][key]["enum"],
            json!(values)
        );
        for value in values.iter().copied().chain([initial_value, initial_value]) {
            publish(
                &runtime,
                &fixture,
                json!({"at":at,"ops":[{"op":"props.set","args":{"values":{key:value}}}]}),
            );
            let mut expected = original.clone();
            expected.insert(
                owner.clone(),
                initial
                    .replace(&old, &format!("\n\t\t\t<{key}>{value}</{key}>"))
                    .into_bytes(),
            );
            assert_eq!(
                unica_workspace::test_support::tree_snapshot(&fixture.source),
                expected
            );
            assert_eq!(view(&runtime, &fixture, &at)["props"][key], value);
        }
        for value in [
            Value::Null,
            json!(true),
            json!(1),
            json!("Unknown"),
            json!("Anywhere"),
            json!("SlowlyObtainedSet"),
        ] {
            let r = call(
                &runtime,
                &fixture,
                ToolIdentity::Apply,
                json!({"at":at,"ops":[{"op":"props.set","args":{"values":{key:value}}}]}),
            );
            assert!(!r.ok, "{r:?}");
            assert_eq!(
                unica_workspace::test_support::tree_snapshot(&fixture.source),
                original
            );
        }
        if key == "ChoiceHistoryOnInput" {
            let refused = call(
                &runtime,
                &fixture,
                ToolIdentity::Apply,
                json!({"at":at,"ops":[{"op":"props.set","args":{"values":{key:"Use"}}}]}),
            );
            assert!(!refused.ok, "{refused:?}");
            assert_eq!(
                unica_workspace::test_support::tree_snapshot(&fixture.source),
                original
            );
        }
        for damaged in [
            String::new(),
            format!("<{key}/>"),
            format!("<{key}>Unknown</{key}>"),
            format!("<{key} xmlns=\"foreign\">{initial_value}</{key}>"),
            format!("{old}{old}"),
            format!("<{key}><value>{initial_value}</value></{key}>"),
        ] {
            std::fs::write(&path, initial.replace(&old, &damaged)).unwrap();
            let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
            let r = call(
                &runtime,
                &fixture,
                ToolIdentity::Apply,
                json!({"at":at,"ops":[{"op":"props.set","args":{"values":{key:values[0]}}}]}),
            );
            assert!(!r.ok, "{key} {damaged}: {r:?}");
            assert_eq!(
                unica_workspace::test_support::tree_snapshot(&fixture.source),
                before
            );
        }
        std::fs::write(&path, &initial).unwrap();
    }
}
#[test]
fn search_input_full_text_preserves_neighbors_and_refuses_damage() {
    verify_search_input("FullTextSearch", &["Use", "DontUse"], "Use");
}
#[test]
fn search_input_search_string_preserves_neighbors_and_refuses_damage() {
    verify_search_input(
        "SearchStringModeOnInputByString",
        &["Begin", "AnyPart"],
        "Begin",
    );
}
#[test]
fn search_input_full_text_input_preserves_neighbors_and_refuses_damage() {
    verify_search_input(
        "FullTextSearchOnInputByString",
        &["Use", "DontUse"],
        "DontUse",
    );
}
#[test]
fn search_input_choice_data_preserves_neighbors_and_refuses_damage() {
    verify_search_input(
        "ChoiceDataGetModeOnInputByString",
        &["Directly", "Background"],
        "Directly",
    );
}
#[test]
fn search_input_create_input_preserves_neighbors_and_refuses_damage() {
    verify_search_input("CreateOnInput", &["Auto", "Use", "DontUse"], "DontUse");
}
#[test]
fn search_input_choice_history_preserves_neighbors_and_refuses_damage() {
    verify_search_input("ChoiceHistoryOnInput", &["Auto", "DontUse"], "Auto");
}

#[test]
fn search_input_preserves_prefixed_property_namespaces() {
    for (at, file) in [
        ("main:Catalog.Codes", "Catalogs/Codes.xml"),
        ("main:Document.Приход", "Documents/Приход.xml"),
    ] {
        let fixture = Fixture::new("workspace-search-input-settings");
        let path = fixture.source.join(file);
        let initial = std::fs::read_to_string(&path).unwrap();
        let doc = roxmltree::Document::parse(&initial).unwrap();
        let mut inserts = Vec::new();
        for node in doc.descendants().filter(|node| {
            node.is_element()
                && node.tag_name().namespace() == Some("http://v8.1c.ru/8.3/MDClasses")
        }) {
            let range = node.range();
            let raw = &initial[range.clone()];
            inserts.push(range.start + 1);
            if let Some(close) = raw.rfind("</") {
                inserts.push(range.start + close + 2);
            }
        }
        inserts.sort_unstable_by(|a, b| b.cmp(a));
        let mut prefixed = initial.clone();
        for offset in inserts {
            prefixed.insert_str(offset, "md:");
        }
        prefixed = prefixed.replacen(
            "xmlns=\"http://v8.1c.ru/8.3/MDClasses\"",
            "xmlns:md=\"http://v8.1c.ru/8.3/MDClasses\"",
            1,
        );
        for key in [
            "FullTextSearch",
            "SearchStringModeOnInputByString",
            "FullTextSearchOnInputByString",
            "ChoiceDataGetModeOnInputByString",
            "CreateOnInput",
            "ChoiceHistoryOnInput",
        ] {
            prefixed = prefixed.replace(
                &format!("\n\t\t\t<md:{key}>"),
                &format!("\n\t\t\t<md:{key} xmlns:note=\"urn:a>b\">"),
            );
        }
        std::fs::write(&path, &prefixed).unwrap();
        let runtime = runtime();
        let mut expected = unica_workspace::test_support::tree_snapshot(&fixture.source);
        let values = json!({"FullTextSearch":"DontUse","SearchStringModeOnInputByString":"AnyPart","FullTextSearchOnInputByString":"Use","ChoiceDataGetModeOnInputByString":"Background","CreateOnInput":"Use","ChoiceHistoryOnInput":"DontUse"});
        for (key, old, new) in [
            ("FullTextSearch", "Use", "DontUse"),
            ("SearchStringModeOnInputByString", "Begin", "AnyPart"),
            ("FullTextSearchOnInputByString", "DontUse", "Use"),
            ("ChoiceDataGetModeOnInputByString", "Directly", "Background"),
            ("CreateOnInput", "DontUse", "Use"),
            ("ChoiceHistoryOnInput", "Auto", "DontUse"),
        ] {
            let old = format!("\n\t\t\t<md:{key} xmlns:note=\"urn:a>b\">{old}</md:{key}>");
            assert_eq!(prefixed.matches(&old).count(), 1);
            prefixed = prefixed.replace(
                &old,
                &format!("\n\t\t\t<md:{key} xmlns:note=\"urn:a>b\">{new}</md:{key}>"),
            );
        }
        expected.insert(PathBuf::from(file), prefixed.into_bytes());
        let changes = unica_metadata::MetaPropertyChanges::convert(
            if at.contains("Catalog") {
                unica_metadata::MetadataKind::Catalog
            } else {
                unica_metadata::MetadataKind::Document
            },
            values
                .as_object()
                .unwrap()
                .iter()
                .map(|(key, value)| {
                    unica_metadata::MetaPropertyInput::new(
                        key,
                        unica_metadata::MetaPropertyValue::String(value.as_str().unwrap().into()),
                    )
                })
                .collect(),
        )
        .unwrap();
        let mut edited = std::fs::read_to_string(&path).unwrap();
        for _ in 0..2 {
            unica_metadata_designer::edit::apply_typed_properties(&mut edited, &changes, "")
                .unwrap();
            assert_eq!(edited.as_bytes(), expected[&PathBuf::from(file)]);
        }
        for _ in 0..2 {
            publish(
                &runtime,
                &fixture,
                json!({"at":at,"ops":[{"op":"props.set","args":{"values":values}}]}),
            );
            assert_eq!(
                unica_workspace::test_support::tree_snapshot(&fixture.source),
                expected
            );
            let props = view(&runtime, &fixture, at);
            for (key, value) in values.as_object().unwrap() {
                assert_eq!(&props["props"][key], value);
            }
        }
    }
}
