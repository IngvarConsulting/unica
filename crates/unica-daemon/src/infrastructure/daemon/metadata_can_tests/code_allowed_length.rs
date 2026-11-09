use super::*;

#[test]
fn code_allowed_length_preserves_owner_sources_and_refuses_damaged_values() {
    let fixture = Fixture::blank();
    let runtime = runtime();
    for (kind, directory) in [
        ("Catalog", "Catalogs"),
        ("ChartOfCharacteristicTypes", "ChartsOfCharacteristicTypes"),
        ("ChartOfCalculationTypes", "ChartsOfCalculationTypes"),
        ("ExchangePlan", "ExchangePlans"),
    ] {
        publish(
            &runtime,
            &fixture,
            json!({"at":"main:Configuration","ops":[{"op":"object.create","args":{"values":{"kind":kind,"name":"Codes"}}}]}),
        );
        let at = format!("main:{kind}.Codes");
        let can = contract(&runtime, &fixture, &at, "props.set");
        assert_eq!(
            can["argsSchema"]["properties"]["values"]["properties"]["CodeAllowedLength"]["enum"],
            json!(["Variable", "Fixed"])
        );
        assert_parity(
            &can["argsSchema"],
            &fixture.binding(false),
            &at,
            "props.set",
            &[
                (json!({"values":{"CodeAllowedLength":"Fixed"}}), true),
                (json!({"values":{"CodeAllowedLength":false}}), false),
                (json!({"values":{"CodeAllowedLength":"Unknown"}}), false),
            ],
        );
        let owner = PathBuf::from(format!("{directory}/Codes.xml"));
        let path = fixture.source.join(&owner);
        let original = unica_workspace::test_support::tree_snapshot(&fixture.source);
        let initial = String::from_utf8(original[&owner].clone()).unwrap();
        let old = "<CodeAllowedLength>Variable</CodeAllowedLength>";
        assert!(initial.contains(old));
        for value in ["Fixed", "Fixed", "Variable"] {
            publish(
                &runtime,
                &fixture,
                json!({"at":at,"ops":[{"op":"props.set","args":{"values":{"CodeAllowedLength":value}}}]}),
            );
            let mut expected = original.clone();
            expected.insert(
                owner.clone(),
                initial
                    .replace(
                        old,
                        &format!("<CodeAllowedLength>{value}</CodeAllowedLength>"),
                    )
                    .into_bytes(),
            );
            assert_eq!(
                unica_workspace::test_support::tree_snapshot(&fixture.source),
                expected
            );
            assert_eq!(
                view(&runtime, &fixture, &at)["props"]["CodeAllowedLength"],
                value
            );
        }
        if kind == "ChartOfCalculationTypes" {
            let numeric =
                initial.replace("<CodeType>String</CodeType>", "<CodeType>Number</CodeType>");
            assert_ne!(numeric, initial);
            std::fs::write(&path, &numeric).unwrap();
            let mut expected = unica_workspace::test_support::tree_snapshot(&fixture.source);
            expected.insert(
                owner.clone(),
                numeric
                    .replace(old, "<CodeAllowedLength>Fixed</CodeAllowedLength>")
                    .into_bytes(),
            );
            publish(
                &runtime,
                &fixture,
                json!({"at":at,"ops":[{"op":"props.set","args":{"values":{"CodeAllowedLength":"Fixed"}}}]}),
            );
            assert_eq!(
                unica_workspace::test_support::tree_snapshot(&fixture.source),
                expected
            );
            assert_eq!(view(&runtime, &fixture, &at)["props"]["CodeType"], "Number");
            std::fs::write(&path, &initial).unwrap();
        }
        if kind == "Catalog" {
            continue;
        }
        for damaged in ["", "<CodeAllowedLength>Unknown</CodeAllowedLength>",
            "<CodeAllowedLength>Variable<!-- split -->invalid</CodeAllowedLength>",
            "<CodeAllowedLength xmlns=\"foreign\">Variable</CodeAllowedLength>", "<CodeAllowedLength><value>Variable</value></CodeAllowedLength>", "<CodeAllowedLength>Variable</CodeAllowedLength><CodeAllowedLength>Fixed</CodeAllowedLength>"] {
            std::fs::write(&path,initial.replace(old,damaged)).unwrap();
            let before=unica_workspace::test_support::tree_snapshot(&fixture.source);
            let result=call(&runtime,&fixture,ToolIdentity::Apply,json!({"at":at,"ops":[{"op":"props.set","args":{"values":{"CodeAllowedLength":"Fixed"}}}]}));
            assert!(!result.ok,"{kind} {damaged}: {result:?}");
            assert_eq!(unica_workspace::test_support::tree_snapshot(&fixture.source),before);
        }
        std::fs::write(&path, &initial).unwrap();
        let plan = call(
            &runtime,
            &fixture,
            ToolIdentity::Apply,
            json!({"at":at,"ops":[{"op":"props.set","args":{"values":{"CodeAllowedLength":"Fixed"}}}]}),
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
