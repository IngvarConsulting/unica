use super::*;

#[test]
fn predefined_data_update_preserves_items_and_refuses_damaged_values() {
    let fixture = Fixture::blank();
    let runtime = runtime();
    for (kind, directory) in [
        ("Catalog", "Catalogs"),
        ("ChartOfAccounts", "ChartsOfAccounts"),
        ("ChartOfCharacteristicTypes", "ChartsOfCharacteristicTypes"),
        ("ChartOfCalculationTypes", "ChartsOfCalculationTypes"),
    ] {
        publish(
            &runtime,
            &fixture,
            json!({"at":"main:Configuration","ops":[{"op":"object.create","args":{"values":{"kind":kind,"name":"Codes"}}}]}),
        );
        let at = format!("main:{kind}.Codes");
        let mut item = json!({"id":"9e883a94-3155-499f-8c9e-1bdcc05b64ab","name":"First","code":"001","description":"Сохраняемый элемент"});
        if kind == "ChartOfCharacteristicTypes" {
            item["type"] = json!({"variants":[{"kind":"boolean"}]});
        }
        publish(
            &runtime,
            &fixture,
            json!({"at":at,"ops":[{"op":"predefinedItem.add","args":{"items":[item]}}]}),
        );
        let can = contract(&runtime, &fixture, &at, "props.set");
        assert_eq!(
            can["argsSchema"]["properties"]["values"]["properties"]["PredefinedDataUpdate"]["enum"],
            json!(["Auto", "AutoUpdate", "DontAutoUpdate"])
        );
        assert_parity(
            &can["argsSchema"],
            &fixture.binding(false),
            &at,
            "props.set",
            &[
                (
                    json!({"values":{"PredefinedDataUpdate":"DontAutoUpdate"}}),
                    true,
                ),
                (json!({"values":{"PredefinedDataUpdate":false}}), false),
                (json!({"values":{"PredefinedDataUpdate":"Unknown"}}), false),
            ],
        );
        let owner = PathBuf::from(format!("{directory}/Codes.xml"));
        let path = fixture.source.join(&owner);
        let original = unica_workspace::test_support::tree_snapshot(&fixture.source);
        let initial = String::from_utf8(original[&owner].clone()).unwrap();
        let old = "<PredefinedDataUpdate>Auto</PredefinedDataUpdate>";
        assert!(initial.contains(old));
        for value in ["DontAutoUpdate", "AutoUpdate", "Auto", "Auto"] {
            publish(
                &runtime,
                &fixture,
                json!({"at":at,"ops":[{"op":"props.set","args":{"values":{"PredefinedDataUpdate":value}}}]}),
            );
            let mut expected = original.clone();
            expected.insert(
                owner.clone(),
                initial
                    .replace(
                        old,
                        &format!("<PredefinedDataUpdate>{value}</PredefinedDataUpdate>"),
                    )
                    .into_bytes(),
            );
            assert_eq!(
                unica_workspace::test_support::tree_snapshot(&fixture.source),
                expected
            );
            assert_eq!(
                view(&runtime, &fixture, &at)["props"]["PredefinedDataUpdate"],
                value
            );
        }
        for damaged in [
            "",
            "<PredefinedDataUpdate>Unknown</PredefinedDataUpdate>",
            "<PredefinedDataUpdate>Auto<!-- split -->invalid</PredefinedDataUpdate>",
            "<PredefinedDataUpdate xmlns=\"foreign\">Auto</PredefinedDataUpdate>",
            "<PredefinedDataUpdate><value>Auto</value></PredefinedDataUpdate>",
            "<PredefinedDataUpdate>Auto</PredefinedDataUpdate><PredefinedDataUpdate>DontAutoUpdate</PredefinedDataUpdate>",
        ] {
            std::fs::write(&path, initial.replace(old, damaged)).unwrap();
            let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
            let result = call(
                &runtime,
                &fixture,
                ToolIdentity::Apply,
                json!({"at":at,"ops":[{"op":"props.set","args":{"values":{"PredefinedDataUpdate":"DontAutoUpdate"}}}]}),
            );
            assert!(!result.ok, "{kind} {damaged}: {result:?}");
            assert_eq!(
                unica_workspace::test_support::tree_snapshot(&fixture.source),
                before
            );
        }
        std::fs::write(&path, &initial).unwrap();
        let plan = call(
            &runtime,
            &fixture,
            ToolIdentity::Apply,
            json!({"at":at,"ops":[{"op":"props.set","args":{"values":{"PredefinedDataUpdate":"DontAutoUpdate"}}}]}),
        );
        assert!(plan.ok, "{plan:?}");
        std::fs::write(
            &path,
            initial.replace("<Comment/>", "<Comment>Changed</Comment>"),
        )
        .unwrap();
        let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
        let result = call(
            &runtime,
            &fixture,
            ToolIdentity::Apply,
            json!({"executionToken":plan.data.unwrap()["executionToken"]}),
        );
        assert!(!result.ok, "{result:?}");
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.source),
            before
        );
        std::fs::write(&path, initial).unwrap();
    }
}
