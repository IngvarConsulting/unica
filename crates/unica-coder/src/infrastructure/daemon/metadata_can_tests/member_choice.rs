use super::*;

const CASES: &[(&str, &str, &str)] = &[
    (
        "main:Catalog.Валюты.Attribute.ОсновнаяВалюта",
        "attribute.set",
        "Catalogs/Валюты.xml",
    ),
    (
        "main:Catalog.Валюты.TabularSection.Представления.Attribute.КодЯзыка",
        "attribute.set",
        "Catalogs/Валюты.xml",
    ),
    (
        "main:InformationRegister.Цены.Dimension.Товар",
        "dimension.set",
        "InformationRegisters/Цены.xml",
    ),
    (
        "main:InformationRegister.Цены.Resource.Цена",
        "resource.set",
        "InformationRegisters/Цены.xml",
    ),
];
fn request(at: &str, op: &str, values: Value) -> Value {
    json!({"at":at,"ops":[{"op":op,"args":{"values":values}}]})
}

#[test]
fn member_choice_roundtrip_preserves_identity_types_neighbors_and_noop() {
    for &(at, op, owner) in CASES {
        let runtime = runtime();
        let fixture = Fixture::new("workspace-member-choice");
        let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
        let schema = contract(&runtime, &fixture, at, op);
        for property in unica_metadata::MemberChoiceProperty::ALL {
            let key = property.argument();
            assert_eq!(
                schema["argsSchema"]["properties"]["values"]["properties"][key]["type"],
                "string"
            );
            let values = property
                .values()
                .unwrap_or(&["Catalog.Валюты.Form.ФормаСписка", ""]);
            if property.values().is_some() {
                assert_eq!(
                    schema["argsSchema"]["properties"]["values"]["properties"][key]["enum"],
                    json!(values)
                );
            }
            for value in values {
                publish(&runtime, &fixture, request(at, op, json!({key:value})));
                assert_eq!(
                    view(&runtime, &fixture, at)["props"][property.xml_name()],
                    *value
                );
                let post = unica_workspace::test_support::tree_snapshot(&fixture.source);
                publish(&runtime, &fixture, request(at, op, json!({key:value})));
                assert_eq!(
                    unica_workspace::test_support::tree_snapshot(&fixture.source),
                    post
                );
            }
        }
        let after = unica_workspace::test_support::tree_snapshot(&fixture.source);
        for (path, bytes) in &before {
            if path != Path::new(owner) {
                assert_eq!(&after[path], bytes);
            }
        }
        // Compare parsed descriptors after restoring just the five selected properties.
        let text = String::from_utf8(after[Path::new(owner)].clone()).unwrap();
        let doc = roxmltree::Document::parse(text.trim_start_matches('\u{feff}')).unwrap();
        let original = String::from_utf8(before[Path::new(owner)].clone()).unwrap();
        let old = roxmltree::Document::parse(original.trim_start_matches('\u{feff}')).unwrap();
        fn without_choice(xml: &str, name: &str) -> String {
            let mut result = xml.to_owned();
            let doc = roxmltree::Document::parse(xml).unwrap();
            let ranges = doc
                .descendants()
                .filter(|n| {
                    n.is_element()
                        && n.parent_element()
                            .and_then(|p| p.children().find(|n| n.tag_name().name() == "Name"))
                            .and_then(|n| n.text())
                            == Some(name)
                        && unica_metadata::MemberChoiceProperty::ALL
                            .iter()
                            .any(|p| p.xml_name() == n.tag_name().name())
                })
                .map(|n| n.range())
                .collect::<Vec<_>>();
            for range in ranges.into_iter().rev() {
                result.replace_range(range, "");
            }
            result
        }
        let name = at.rsplit('.').next().unwrap();
        assert_eq!(without_choice(&text, name), without_choice(&original, name));
        assert_eq!(
            doc.descendants()
                .filter_map(|n| n.attribute("uuid"))
                .collect::<Vec<_>>(),
            old.descendants()
                .filter_map(|n| n.attribute("uuid"))
                .collect::<Vec<_>>()
        );
        assert_eq!(
            doc.descendants()
                .filter(|n| n.is_element() && matches!(n.tag_name().name(), "Type" | "FillValue"))
                .map(|n| &text[n.range()])
                .collect::<Vec<_>>(),
            old.descendants()
                .filter(|n| n.is_element() && matches!(n.tag_name().name(), "Type" | "FillValue"))
                .map(|n| &original[n.range()])
                .collect::<Vec<_>>()
        );
    }
}

#[test]
fn member_choice_refuses_invalid_values_forms_context_and_malformed_preimage() {
    let runtime = runtime();
    let fixture = Fixture::new("workspace-member-choice");
    for &(at, op, _) in CASES {
        for values in [
            json!({"quickChoice":true}),
            json!({"createOnInput":"Unknown"}),
            json!({"choiceHistoryOnInput":"Use"}),
            json!({"choiceFoldersAndItems":"All"}),
            json!({"choiceForm":"Catalog.Валюты.Form.Missing"}),
            json!({"choiceForm":"Catalog.Валюты"}),
        ] {
            let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
            let result = call(
                &runtime,
                &fixture,
                ToolIdentity::Apply,
                request(at, op, values),
            );
            assert!(!result.ok, "{result:?}");
            assert_eq!(
                unica_workspace::test_support::tree_snapshot(&fixture.source),
                before
            );
        }
    }
    let result = call(
        &runtime,
        &fixture,
        ToolIdentity::Apply,
        request(
            "main:Catalog.Валюты.TabularSection.Представления",
            "tabularSection.set",
            json!({"quickChoice":"Use"}),
        ),
    );
    assert!(!result.ok, "{result:?}");
    let path = fixture.source.join("Catalogs/Валюты.xml");
    let original = std::fs::read_to_string(&path).unwrap();
    for damage in [
        "<QuickChoice/>",
        "<QuickChoice>Unknown</QuickChoice>",
        "<QuickChoice>Auto</QuickChoice><QuickChoice>Auto</QuickChoice>",
        "<QuickChoice xmlns=\"foreign\">Auto</QuickChoice>",
    ] {
        std::fs::write(
            &path,
            original.replace("<QuickChoice>Auto</QuickChoice>", damage),
        )
        .unwrap();
        let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
        let result = call(
            &runtime,
            &fixture,
            ToolIdentity::Apply,
            request(CASES[0].0, CASES[0].1, json!({"quickChoice":"Use"})),
        );
        assert!(!result.ok, "{damage}: {result:?}");
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.source),
            before
        );
    }
}

#[test]
fn member_choice_saved_token_guards_form_payload_registration_and_owner() {
    for dependency in [
        "Configuration.xml",
        "Catalogs/Валюты.xml",
        "Catalogs/Валюты/Forms/ФормаСписка.xml",
        "Catalogs/Валюты/Forms/ФормаСписка/Ext/Form.xml",
    ] {
        let runtime = runtime();
        let fixture = Fixture::new("workspace-member-choice");
        let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
        let preview = call(
            &runtime,
            &fixture,
            ToolIdentity::Apply,
            request(
                CASES[2].0,
                CASES[2].1,
                json!({"choiceForm":"Catalog.Валюты.Form.ФормаСписка"}),
            ),
        );
        assert!(preview.ok, "{preview:?}");
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.source),
            before
        );
        let token = preview.data.unwrap()["executionToken"].clone();
        let path = fixture.source.join(dependency);
        let mut bytes = std::fs::read(&path).unwrap();
        bytes.extend_from_slice(b"\n");
        std::fs::write(&path, bytes).unwrap();
        let concurrent = unica_workspace::test_support::tree_snapshot(&fixture.source);
        let result = call(
            &runtime,
            &fixture,
            ToolIdentity::Apply,
            json!({"executionToken":token}),
        );
        assert!(!result.ok, "{dependency}: {result:?}");
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.source),
            concurrent
        );
    }
}

#[test]
fn member_choice_checks_final_form_after_later_removal_including_noop() {
    for at in [
        CASES[0].0,
        "main:Catalog.Валюты.TabularSection.представления.Attribute.кодязыка",
    ] {
        for noop in [false, true] {
            let runtime = runtime();
            let fixture = Fixture::new("workspace-member-choice");
            let form = "Catalog.Валюты.Form.ФормаСписка";
            if noop {
                publish(
                    &runtime,
                    &fixture,
                    request(at, CASES[0].1, json!({"choiceForm":form})),
                );
            }
            std::fs::remove_file(fixture.source.join("Catalogs/Валюты/Forms/ФормаСписка.xml"))
                .unwrap();
            let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
            let result = call(
                &runtime,
                &fixture,
                ToolIdentity::Apply,
                request(at, "attribute.set", json!({"choiceForm":form})),
            );
            assert!(!result.ok, "{result:?}");
            assert!(result.summary.contains("choiceForm"), "{result:?}");
            assert_eq!(
                unica_workspace::test_support::tree_snapshot(&fixture.source),
                before
            );
        }
    }
}

#[test]
fn member_choice_exact_scope_preserves_same_named_neighbor_and_late_failure_is_atomic() {
    let runtime = runtime();
    let fixture = Fixture::new("workspace-member-choice");
    let path = fixture.source.join("Catalogs/Валюты.xml");
    let xml = std::fs::read_to_string(&path).unwrap();
    let section = xml.find("<TabularSection ").unwrap();
    let mut modified = xml[..section].to_owned();
    modified.push_str(
        &xml[section..]
            .replacen("<Name>КодЯзыка</Name>", "<Name>ОсновнаяВалюта</Name>", 1)
            .replacen(
                "<ChoiceForm/>",
                "<ChoiceForm>Catalog.Codes.Form.Missing</ChoiceForm>",
                1,
            ),
    );
    std::fs::write(&path, modified).unwrap();
    publish(
        &runtime,
        &fixture,
        request(
            CASES[0].0,
            CASES[0].1,
            json!({"choiceForm":"Catalog.Codes.Form.Choose"}),
        ),
    );
    let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
    let result = call(
        &runtime,
        &fixture,
        ToolIdentity::Apply,
        json!({"at":CASES[0].0,"ops":[
            {"op":"attribute.set","args":{"values":{"quickChoice":"Use"}}},
            {"op":"attribute.set","args":{"values":{"createOnInput":"Unknown"}}}
        ]}),
    );
    assert!(!result.ok, "{result:?}");
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&fixture.source),
        before
    );
}
