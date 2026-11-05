use super::*;

const OWNERS: &[(&str, &str)] = &[
    ("Catalog", "Catalogs"),
    ("Enum", "Enums"),
    ("ChartOfCharacteristicTypes", "ChartsOfCharacteristicTypes"),
    ("ChartOfCalculationTypes", "ChartsOfCalculationTypes"),
    ("ChartOfAccounts", "ChartsOfAccounts"),
    ("ExchangePlan", "ExchangePlans"),
];

fn choice_owners(with_forms: bool) -> (Fixture, V5CanonicalInvocationRuntime) {
    let fixture = Fixture::blank();
    let runtime = runtime();
    for (kind, _) in OWNERS {
        publish(
            &runtime,
            &fixture,
            json!({"at":"main:Configuration","ops":[{"op":"object.create","args":{"values":{"kind":kind,"name":"Codes"}}}]}),
        );
        if with_forms && *kind != "ChartOfCalculationTypes" {
            publish(
                &runtime,
                &fixture,
                json!({"at":format!("main:{kind}.Codes"),"ops":[{"op":"form.add","args":{"items":[{"name":"Choose","type":"ChoiceForm"}]}}]}),
            );
        }
    }
    (fixture, runtime)
}

fn change_quick_choice(
    runtime: &V5CanonicalInvocationRuntime,
    fixture: &Fixture,
    kind: &str,
    directory: &str,
    values: &[bool],
) {
    let at = format!("main:{kind}.Codes");
    let owner = PathBuf::from(format!("{directory}/Codes.xml"));
    let original = unica_workspace::test_support::tree_snapshot(&fixture.source);
    let initial = String::from_utf8(original[&owner].clone()).unwrap();
    let old = "<QuickChoice>false</QuickChoice>";
    assert_eq!(initial.matches(old).count(), 1);
    for value in values {
        publish(
            runtime,
            fixture,
            json!({"at":at,"ops":[{"op":"props.set","args":{"values":{"QuickChoice":value}}}]}),
        );
        let mut expected = original.clone();
        expected.insert(
            owner.clone(),
            initial
                .replace(old, &format!("<QuickChoice>{value}</QuickChoice>"))
                .into_bytes(),
        );
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.source),
            expected
        );
        assert_eq!(view(runtime, fixture, &at)["props"]["QuickChoice"], *value);
    }
}

#[test]
fn medium_quick_choice_is_boolean_preserves_forms_and_refuses_damaged_source() {
    let (fixture, runtime) = choice_owners(true);
    for (kind, directory) in OWNERS {
        let at = format!("main:{kind}.Codes");
        let can = contract(&runtime, &fixture, &at, "props.set");
        assert_eq!(
            can["argsSchema"]["properties"]["values"]["properties"]["QuickChoice"]["type"],
            "boolean"
        );
        change_quick_choice(&runtime, &fixture, kind, directory, &[true, true, false]);
        let original = unica_workspace::test_support::tree_snapshot(&fixture.source);
        for value in [
            json!("true"),
            json!("false"),
            json!("Auto"),
            json!("Use"),
            json!("DontUse"),
            json!(1),
            Value::Null,
        ] {
            let args = json!({"values":{"QuickChoice":value}});
            assert!(!jsonschema::validator_for(&can["argsSchema"])
                .unwrap()
                .is_valid(&args));
            let result = call(
                &runtime,
                &fixture,
                ToolIdentity::Apply,
                json!({"at":at,"ops":[{"op":"props.set","args":args}]}),
            );
            assert!(!result.ok, "{result:?}");
            assert_eq!(
                unica_workspace::test_support::tree_snapshot(&fixture.source),
                original
            );
        }
        if *kind == "Catalog" {
            continue;
        }
        let owner = PathBuf::from(format!("{directory}/Codes.xml"));
        let path = fixture.source.join(&owner);
        let initial = String::from_utf8(original[&owner].clone()).unwrap();
        let old = "<QuickChoice>false</QuickChoice>";
        for damaged in [
            "",
            "<QuickChoice/>",
            "<QuickChoice>Auto</QuickChoice>",
            "<QuickChoice><value>false</value></QuickChoice>",
            "<QuickChoice xmlns=\"foreign\">false</QuickChoice>",
            "<QuickChoice>false</QuickChoice><QuickChoice>true</QuickChoice>",
        ] {
            std::fs::write(&path, initial.replace(old, damaged)).unwrap();
            let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
            let result = call(
                &runtime,
                &fixture,
                ToolIdentity::Apply,
                json!({"at":at,"ops":[{"op":"props.set","args":{"values":{"QuickChoice":true}}}]}),
            );
            assert!(!result.ok, "{kind}: {damaged}: {result:?}");
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
            json!({"at":at,"ops":[{"op":"props.set","args":{"values":{"QuickChoice":true}}}]}),
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
        std::fs::write(&path, initial).unwrap();
    }
}

#[test]
fn quick_choice_keeps_inactive_values_without_changing_choice_mode() {
    let (fixture, runtime) = choice_owners(false);
    for (kind, directory) in OWNERS {
        let path = fixture.source.join(format!("{directory}/Codes.xml"));
        let original = std::fs::read_to_string(&path).unwrap();
        for mode in ["FromForm", "QuickChoice"] {
            std::fs::write(
                &path,
                original.replace(
                    "<ChoiceMode>BothWays</ChoiceMode>",
                    &format!("<ChoiceMode>{mode}</ChoiceMode>"),
                ),
            )
            .unwrap();
            change_quick_choice(&runtime, &fixture, kind, directory, &[true, false]);
            assert_eq!(
                view(&runtime, &fixture, &format!("main:{kind}.Codes"))["props"]["ChoiceMode"],
                mode
            );
        }
    }
}
