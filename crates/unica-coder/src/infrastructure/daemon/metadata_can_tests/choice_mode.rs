use super::*;

#[test]
fn choice_mode_preserves_other_choices_and_refuses_damaged_values() {
    let fixture = Fixture::blank();
    let runtime = runtime();
    for (kind, directory) in [
        ("Catalog", "Catalogs"),
        ("Enum", "Enums"),
        ("ChartOfAccounts", "ChartsOfAccounts"),
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
        // The public form builder does not yet support calculation charts.
        // Other owners exercise preservation of nonempty form files as well.
        if kind != "ChartOfCalculationTypes" {
            publish(
                &runtime,
                &fixture,
                json!({"at":at,"ops":[{"op":"form.add","args":{"items":[{"name":"Choose","type":"ChoiceForm"}]}}]}),
            );
        }
        let can = contract(&runtime, &fixture, &at, "props.set");
        assert_eq!(
            can["argsSchema"]["properties"]["values"]["properties"]["ChoiceMode"]["enum"],
            json!(["BothWays", "FromForm", "QuickChoice"])
        );
        assert_parity(
            &can["argsSchema"],
            &fixture.binding(false),
            &at,
            "props.set",
            &[
                (json!({"values":{"ChoiceMode":"FromForm"}}), true),
                (json!({"values":{"ChoiceMode":false}}), false),
                (json!({"values":{"ChoiceMode":"Unknown"}}), false),
            ],
        );
        let owner = PathBuf::from(format!("{directory}/Codes.xml"));
        let path = fixture.source.join(&owner);
        let original = unica_workspace::test_support::tree_snapshot(&fixture.source);
        let initial = String::from_utf8(original[&owner].clone()).unwrap();
        let old = "<ChoiceMode>BothWays</ChoiceMode>";
        assert!(initial.contains(old));
        for value in ["FromForm", "QuickChoice", "BothWays", "BothWays"] {
            publish(
                &runtime,
                &fixture,
                json!({"at":at,"ops":[{"op":"props.set","args":{"values":{"ChoiceMode":value}}}]}),
            );
            let mut expected = original.clone();
            expected.insert(
                owner.clone(),
                initial
                    .replace(old, &format!("<ChoiceMode>{value}</ChoiceMode>"))
                    .into_bytes(),
            );
            assert_eq!(
                unica_workspace::test_support::tree_snapshot(&fixture.source),
                expected
            );
            assert_eq!(view(&runtime, &fixture, &at)["props"]["ChoiceMode"], value);
        }
        if kind == "Catalog" {
            continue;
        }
        for damaged in [
            "",
            "<ChoiceMode>Unknown</ChoiceMode>",
            "<ChoiceMode>BothWays<!-- split -->invalid</ChoiceMode>",
            "<ChoiceMode xmlns=\"foreign\">BothWays</ChoiceMode>",
            "<ChoiceMode><value>BothWays</value></ChoiceMode>",
            "<ChoiceMode>BothWays</ChoiceMode><ChoiceMode>FromForm</ChoiceMode>",
        ] {
            std::fs::write(&path, initial.replace(old, damaged)).unwrap();
            let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
            let result = call(
                &runtime,
                &fixture,
                ToolIdentity::Apply,
                json!({"at":at,"ops":[{"op":"props.set","args":{"values":{"ChoiceMode":"FromForm"}}}]}),
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
            json!({"at":at,"ops":[{"op":"props.set","args":{"values":{"ChoiceMode":"FromForm"}}}]}),
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
