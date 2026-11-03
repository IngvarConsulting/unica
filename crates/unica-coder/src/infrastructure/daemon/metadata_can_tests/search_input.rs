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
