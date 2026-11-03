use super::*;

const FLAGS: [&str; 2] = [
    "UpdateDataHistoryImmediatelyAfterWrite",
    "ExecuteAfterWriteDataHistoryVersionProcessing",
];
const STATES: &[(&str, bool, bool)] = &[
    ("Use", false, false),
    ("Use", true, false),
    ("Use", true, true),
    ("Use", false, true),
    ("DontUse", false, true),
    ("DontUse", true, true),
    ("DontUse", true, false),
    ("DontUse", false, false),
    ("DontUse", false, false),
];

fn verify_history(kind: &str, directory: &str, name: &str) {
    let fixture = Fixture::new("workspace-data-history-settings");
    let runtime = runtime();
    let at = format!("main:{kind}.{name}");
    let owner = PathBuf::from(format!("{directory}/{name}.xml"));
    let path = fixture.source.join(&owner);
    let original = unica_workspace::test_support::tree_snapshot(&fixture.source);
    let initial = String::from_utf8(original[&owner].clone()).unwrap();
    assert_eq!(
        initial
            .matches("<DataHistory>DontUse</DataHistory>")
            .count(),
        1
    );
    let can = contract(&runtime, &fixture, &at, "props.set");
    assert_eq!(
        can["argsSchema"]["properties"]["values"]["properties"]["DataHistory"]["enum"],
        json!(["Use", "DontUse"])
    );
    for key in FLAGS {
        assert_eq!(
            can["argsSchema"]["properties"]["values"]["properties"][key]["type"],
            "boolean"
        );
    }
    let mut previous = ("DontUse", false, false);
    for &(history, immediate, processing) in STATES {
        let mut values = serde_json::Map::new();
        if previous.0 != history {
            values.insert("DataHistory".into(), json!(history));
        }
        if previous.1 != immediate {
            values.insert(FLAGS[0].into(), json!(immediate));
        }
        if previous.2 != processing {
            values.insert(FLAGS[1].into(), json!(processing));
        }
        if values.is_empty() {
            values.insert("DataHistory".into(), json!(history));
        }
        publish(
            &runtime,
            &fixture,
            json!({"at":at,"ops":[{"op":"props.set","args":{"values":values}}]}),
        );
        let mut expected = original.clone();
        expected.insert(
            owner.clone(),
            initial
                .replace(
                    "<DataHistory>DontUse</DataHistory>",
                    &format!("<DataHistory>{history}</DataHistory>"),
                )
                .replace(
                    &format!("<{}>false</{}>", FLAGS[0], FLAGS[0]),
                    &format!("<{}>{immediate}</{}>", FLAGS[0], FLAGS[0]),
                )
                .replace(
                    &format!("<{}>false</{}>", FLAGS[1], FLAGS[1]),
                    &format!("<{}>{processing}</{}>", FLAGS[1], FLAGS[1]),
                )
                .into_bytes(),
        );
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.source),
            expected
        );
        let props = view(&runtime, &fixture, &at);
        assert_eq!(props["props"]["DataHistory"], history);
        assert_eq!(props["props"][FLAGS[0]], immediate);
        assert_eq!(props["props"][FLAGS[1]], processing);
        previous = (history, immediate, processing);
    }
    for key in ["DataHistory", FLAGS[0], FLAGS[1]] {
        let old_value = if key == "DataHistory" {
            "DontUse"
        } else {
            "false"
        };
        for value in [Value::Null, json!(1), json!("Unknown")] {
            let refused = call(
                &runtime,
                &fixture,
                ToolIdentity::Apply,
                json!({"at":at,"ops":[{"op":"props.set","args":{"values":{key:value}}}]}),
            );
            assert!(!refused.ok, "{refused:?}");
            assert_eq!(
                unica_workspace::test_support::tree_snapshot(&fixture.source),
                original
            );
        }
        let old = format!("<{key}>{old_value}</{key}>");
        for damaged in [
            String::new(),
            format!("<{key}/>"),
            format!("<{key}>Unknown</{key}>"),
            format!("<{key} xmlns=\"foreign\">{old_value}</{key}>"),
            format!("{old}{old}"),
            format!("<{key}><value>{old_value}</value></{key}>"),
        ] {
            std::fs::write(&path, initial.replace(&old, &damaged)).unwrap();
            let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
            let value = if key == "DataHistory" {
                json!("Use")
            } else {
                json!(true)
            };
            let refused = call(
                &runtime,
                &fixture,
                ToolIdentity::Apply,
                json!({"at":at,"ops":[{"op":"props.set","args":{"values":{key:value}}}]}),
            );
            assert!(!refused.ok, "{key} {damaged}: {refused:?}");
            assert_eq!(
                unica_workspace::test_support::tree_snapshot(&fixture.source),
                before
            );
        }
        std::fs::write(&path, &initial).unwrap();
    }
    let plan = call(
        &runtime,
        &fixture,
        ToolIdentity::Apply,
        json!({"at":at,"ops":[{"op":"props.set","args":{"values":{"DataHistory":"Use"}}}]}),
    );
    assert!(plan.ok, "{plan:?}");
    std::fs::write(
        &path,
        initial.replace("<Comment/>", "<Comment>Changed</Comment>"),
    )
    .unwrap();
    let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
    let refused = call(
        &runtime,
        &fixture,
        ToolIdentity::Apply,
        json!({"executionToken":plan.data.unwrap()["executionToken"]}),
    );
    assert!(!refused.ok, "{refused:?}");
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&fixture.source),
        before
    );
}

#[test]
fn data_history_catalog_preserves_neighbors_and_inactive_flags() {
    verify_history("Catalog", "Catalogs", "Codes");
}
#[test]
fn data_history_document_preserves_neighbors_and_inactive_flags() {
    verify_history("Document", "Documents", "Приход");
}
#[test]
fn data_history_register_preserves_neighbors_and_inactive_flags() {
    verify_history("InformationRegister", "InformationRegisters", "Цены");
}
