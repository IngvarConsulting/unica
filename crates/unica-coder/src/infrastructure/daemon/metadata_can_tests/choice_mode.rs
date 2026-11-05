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

#[test]
fn choice_mode_preserves_prefixed_namespaces_on_the_new_owners() {
    use unica_metadata::MetadataKind;
    for (kind, file) in [
        (MetadataKind::Enum, "Enums/Codes.xml"),
        (
            MetadataKind::ChartOfCharacteristicTypes,
            "ChartsOfCharacteristicTypes/Codes.xml",
        ),
        (
            MetadataKind::ChartOfCalculationTypes,
            "ChartsOfCalculationTypes/Codes.xml",
        ),
        (MetadataKind::ChartOfAccounts, "ChartsOfAccounts/Codes.xml"),
        (MetadataKind::ExchangePlan, "ExchangePlans/Codes.xml"),
    ] {
        let at = format!("main:{}.Codes", kind.as_str());
        let fixture = Fixture::new("workspace-choice-mode");
        let path = fixture.source.join(file);
        let initial = std::fs::read_to_string(&path).unwrap();
        // The owner's MDClasses elements move to the `md:` prefix without a
        // default namespace; ChoiceMode also carries its own declaration.
        let doc = roxmltree::Document::parse(&initial).unwrap();
        let mut inserts = Vec::new();
        for node in doc.descendants().filter(|node| {
            node.is_element()
                && node.tag_name().namespace() == Some("http://v8.1c.ru/8.3/MDClasses")
        }) {
            let range = node.range();
            inserts.push(range.start + 1);
            if let Some(close) = initial[range.clone()].rfind("</") {
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
        let old = "<md:ChoiceMode>BothWays</md:ChoiceMode>";
        assert_eq!(prefixed.matches(old).count(), 1, "{file}");
        let prefixed = prefixed.replace(
            old,
            "<md:ChoiceMode xmlns:note=\"urn:a>b\">BothWays</md:ChoiceMode>",
        );
        std::fs::write(&path, &prefixed).unwrap();
        let runtime = runtime();
        let mut expected = unica_workspace::test_support::tree_snapshot(&fixture.source);
        let changed = prefixed.replace(
            "<md:ChoiceMode xmlns:note=\"urn:a>b\">BothWays</md:ChoiceMode>",
            "<md:ChoiceMode xmlns:note=\"urn:a>b\">QuickChoice</md:ChoiceMode>",
        );
        expected.insert(PathBuf::from(file), changed.clone().into_bytes());
        let changes = unica_metadata::MetaPropertyChanges::convert(
            kind,
            vec![unica_metadata::MetaPropertyInput::new(
                "ChoiceMode",
                unica_metadata::MetaPropertyValue::String("QuickChoice".into()),
            )],
        )
        .unwrap();
        let mut edited = prefixed.clone();
        for _ in 0..2 {
            unica_metadata_designer::edit::apply_typed_properties(&mut edited, &changes, "")
                .unwrap();
            assert_eq!(edited, changed, "{file}");
        }
        for _ in 0..2 {
            publish(
                &runtime,
                &fixture,
                json!({"at":at,"ops":[{"op":"props.set","args":{"values":{"ChoiceMode":"QuickChoice"}}}]}),
            );
            assert_eq!(
                unica_workspace::test_support::tree_snapshot(&fixture.source),
                expected,
                "{file}"
            );
            assert_eq!(view(&runtime, &fixture, &at)["props"]["ChoiceMode"], "QuickChoice");
        }
    }
}
