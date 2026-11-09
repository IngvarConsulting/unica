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

fn assert_prefixed_choice_ranges(runtime: &V5CanonicalInvocationRuntime) {
    let fixture = Fixture::new("workspace-member-choice");
    for owner in ["Catalogs/Валюты.xml", "InformationRegisters/Цены.xml"] {
        let path = fixture.source.join(owner);
        let text = std::fs::read_to_string(&path).unwrap();
        let doc = roxmltree::Document::parse(text.trim_start_matches('\u{feff}')).unwrap();
        let offset = text.len() - text.trim_start_matches('\u{feff}').len();
        let mut ranges = doc
            .descendants()
            .filter(|node| {
                node.is_element()
                    && node.tag_name().name() == "Properties"
                    && node.parent().is_some_and(|parent| {
                        matches!(
                            parent.tag_name().name(),
                            "Attribute" | "Dimension" | "Resource"
                        )
                    })
            })
            .map(|node| node.range())
            .collect::<Vec<_>>();
        ranges.sort_by_key(|range| std::cmp::Reverse(range.start));
        let mut prefixed = text.clone();
        for range in ranges {
            let replacement = text[offset + range.start..offset + range.end]
                .replacen(
                    "<Properties>",
                    "<md:Properties xmlns:md=\"http://v8.1c.ru/8.3/MDClasses\">",
                    1,
                )
                .replacen("</Properties>", "</md:Properties>", 1);
            let replacement = replacement
                .replacen(
                    "<QuickChoice>",
                    "<choice:QuickChoice xmlns:choice=\"http://v8.1c.ru/8.3/MDClasses\">",
                    1,
                )
                .replacen("</QuickChoice>", "</choice:QuickChoice>", 1);
            prefixed.replace_range(offset + range.start..offset + range.end, &replacement);
        }
        std::fs::write(path, prefixed).unwrap();
    }
    for &(at, op, _) in CASES {
        publish(
            runtime,
            &fixture,
            request(at, op, json!({"quickChoice":"Use"})),
        );
        assert_eq!(view(runtime, &fixture, at)["props"]["QuickChoice"], "Use");
        let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
        publish(
            runtime,
            &fixture,
            request(at, op, json!({"quickChoice":"Use"})),
        );
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.source),
            before
        );
    }
}

fn legacy_catalog_form(fixture: &Fixture, owner: &str, form: &str) -> (PathBuf, String) {
    let path = fixture.source.join(format!("Catalogs/{owner}.xml"));
    let descriptor = std::fs::read_to_string(
        fixture
            .source
            .join(format!("Catalogs/{owner}/Forms/{form}.xml")),
    )
    .unwrap();
    let descriptor = descriptor.trim_start_matches('\u{feff}');
    let doc = roxmltree::Document::parse(descriptor).unwrap();
    let child = doc
        .root_element()
        .children()
        .find(|n| n.is_element())
        .unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    let registration = format!("<Form>{form}</Form>");
    assert_eq!(text.matches(&registration).count(), 1);
    let legacy = text.replace(&registration, &descriptor[child.range()]);
    std::fs::write(&path, &legacy).unwrap();
    (path, legacy)
}

fn assert_legacy_choice_repair(runtime: &V5CanonicalInvocationRuntime) {
    let fixture = Fixture::new("workspace-member-choice");
    let (path, _) = legacy_catalog_form(&fixture, "Валюты", "ФормаСписка");
    let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
    let planned = call(
        runtime,
        &fixture,
        ToolIdentity::Apply,
        request(
            CASES[0].0,
            CASES[0].1,
            json!({"choiceForm":"Catalog.Валюты.Form.ФормаСписка"}),
        ),
    );
    assert!(planned.ok, "legacy form must be repaired: {planned:?}");
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&fixture.source),
        before
    );
    assert!(planned
        .warnings
        .iter()
        .any(|w| w["code"] == "legacy_inline_registration"));
    let published = call(
        runtime,
        &fixture,
        ToolIdentity::Apply,
        json!({"executionToken":planned.data.unwrap()["executionToken"]}),
    );
    assert!(published.ok, "{published:?}");
    let text = std::fs::read_to_string(path).unwrap();
    assert!(text.contains("<Form>ФормаСписка</Form>"));
    assert!(!text.contains("<Form uuid="));
    assert_eq!(
        view(runtime, &fixture, CASES[0].0)["props"]["ChoiceForm"],
        "Catalog.Валюты.Form.ФормаСписка"
    );
}

#[test]
fn medium_member_choice_roundtrip_preserves_identity_types_neighbors_and_noop() {
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
                    view(&runtime, &fixture, at)["props"][property.public_name()],
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
                            .any(|p| p.public_name() == n.tag_name().name())
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
fn medium_member_choice_refuses_invalid_values_forms_context_and_malformed_preimage() {
    let runtime = runtime();
    assert_prefixed_choice_ranges(&runtime);
    assert_legacy_choice_repair(&runtime);
    let fixture = Fixture::new("workspace-member-choice");
    for &(at, op, _) in CASES {
        for values in [
            json!({"quickChoice":true}),
            json!({"createOnInput":"Unknown"}),
            json!({"choiceHistoryOnInput":"Use"}),
            json!({"choiceFoldersAndItems":"All"}),
            json!({"choiceForm":"Catalog.Валюты.Form.Missing"}),
            json!({"choiceForm":"Catalog.Валюты"}),
            json!({"choiceForm":"Catalog.Codes.Form.Choose"}),
            json!({"choiceForm":"Document.Приход.Form.Choose"}),
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
        let offset = original.find("<Name>ОсновнаяВалюта</Name>").unwrap();
        let mut modified = original[..offset].to_owned();
        modified.push_str(&original[offset..].replacen(
            "<QuickChoice>DontUse</QuickChoice>",
            damage,
            1,
        ));
        assert_ne!(modified, original, "damage must affect the addressed field");
        std::fs::write(&path, modified).unwrap();
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
    std::fs::write(&path, &original).unwrap();
    let form_path = fixture
        .source
        .join("Catalogs/Валюты/Forms/ФормаСписка/Ext/Form.xml");
    let form = std::fs::read_to_string(&form_path).unwrap();
    for damaged in [
        form.replace(
            "<ChoiceMode>true</ChoiceMode>",
            "<ChoiceMode>false</ChoiceMode>",
        ),
        form.replace(
            "<ManualQuery>false</ManualQuery>",
            "<ManualQuery>true</ManualQuery>",
        ),
        form.replace(
            "</Form>",
            "<Events><Event name=\"ChoiceProcessing\">CustomChoice</Event></Events></Form>",
        ),
        form.replace(
            "</Form>",
            "<CommandName>CommonCommand.CustomChoice</CommandName></Form>",
        ),
    ] {
        std::fs::write(&form_path, damaged).unwrap();
        let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
        let result = call(
            &runtime,
            &fixture,
            ToolIdentity::Apply,
            request(
                CASES[0].0,
                CASES[0].1,
                json!({"choiceForm":"Catalog.Валюты.Form.ФормаСписка"}),
            ),
        );
        assert!(
            !result.ok && result.summary.contains("choiceForm"),
            "{result:?}"
        );
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.source),
            before
        );
    }
    std::fs::write(&form_path, form).unwrap();
    let module = form_path.with_extension("").join("Module.bsl");
    std::fs::create_dir_all(module.parent().unwrap()).unwrap();
    std::fs::write(&module, "Процедура Выбор()\nКонецПроцедуры\n").unwrap();
    let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
    let result = call(
        &runtime,
        &fixture,
        ToolIdentity::Apply,
        request(
            CASES[0].0,
            CASES[0].1,
            json!({"choiceForm":"Catalog.Валюты.Form.ФормаСписка"}),
        ),
    );
    assert!(
        !result.ok && result.summary.contains("choiceForm"),
        "{result:?}"
    );
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&fixture.source),
        before
    );
    std::fs::remove_file(module).unwrap();
    publish(
        &runtime,
        &fixture,
        request(
            CASES[0].0,
            CASES[0].1,
            json!({"choiceForm":"Catalog.Валюты.Form.ФормаСписка"}),
        ),
    );
    let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
    let result = call(
        &runtime,
        &fixture,
        ToolIdentity::Apply,
        request(
            CASES[0].0,
            CASES[0].1,
            json!({"type":{"variants":[{"kind":"boolean"}]},"fillValue":{"kind":"boolean","value":false}}),
        ),
    );
    assert!(
        !result.ok && result.summary.contains("choiceForm"),
        "{result:?}"
    );
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&fixture.source),
        before
    );
    publish(
        &runtime,
        &fixture,
        request(
            CASES[0].0,
            CASES[0].1,
            json!({"choiceForm":"","type":{"variants":[{"kind":"boolean"}]},"fillValue":{"kind":"boolean","value":false}}),
        ),
    );
    // Form ownership does not determine its selected type: a standard list
    // may belong to Codes while selecting currency references.
    let cross_owner = Fixture::new("workspace-member-choice");
    let (cross_path, cross_legacy) = legacy_catalog_form(&cross_owner, "Codes", "Choose");
    std::fs::copy(
        cross_owner
            .source
            .join("Catalogs/Валюты/Forms/ФормаСписка/Ext/Form.xml"),
        cross_owner
            .source
            .join("Catalogs/Codes/Forms/Choose/Ext/Form.xml"),
    )
    .unwrap();
    std::fs::write(
        cross_owner
            .source
            .join("Catalogs/Codes/Forms/Choose/Ext/Form/Module.bsl"),
        "// Standard list without custom handlers\n",
    )
    .unwrap();
    publish(
        &runtime,
        &cross_owner,
        request(
            CASES[0].0,
            CASES[0].1,
            json!({"choiceForm":"Catalog.Codes.Form.Choose"}),
        ),
    );
    assert_eq!(std::fs::read_to_string(cross_path).unwrap(), cross_legacy);
    assert_eq!(
        view(&runtime, &cross_owner, CASES[0].0)["props"]["ChoiceForm"],
        "Catalog.Codes.Form.Choose"
    );
}

#[test]
fn medium_member_choice_saved_token_guards_form_payload_registration_and_owner() {
    for dependency in [
        "Configuration.xml",
        "Catalogs/Валюты.xml",
        "Catalogs/Валюты/Forms/ФормаСписка.xml",
        "Catalogs/Валюты/Forms/ФормаСписка/Ext/Form.xml",
        "Catalogs/Валюты/Forms/ФормаСписка/Ext/Form/Module.bsl",
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
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut bytes = std::fs::read(&path).unwrap_or_default();
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
fn medium_member_choice_rejects_missing_form_on_assignment_including_noop() {
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
fn medium_member_choice_exact_scope_preserves_same_named_neighbor_and_late_failure_is_atomic() {
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
            json!({"choiceForm":"Catalog.Валюты.Form.ФормаСписка"}),
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

#[test]
fn medium_member_choice_preserves_untouched_copy_and_checks_explicit_update_after_rename() {
    let runtime = runtime();
    let fixture = Fixture::new("workspace-member-choice");
    let path = fixture.source.join("Catalogs/Валюты.xml");
    let xml = std::fs::read_to_string(&path).unwrap();
    let offset = xml.find("<Name>КодЯзыка</Name>").unwrap();
    let form = "Catalog.Codes.Form.Choose";
    let mut modified = xml[..offset].to_owned();
    modified.push_str(&xml[offset..].replacen(
        "<ChoiceForm/>",
        &format!("<ChoiceForm>{form}</ChoiceForm>"),
        1,
    ));
    assert_ne!(modified, xml);
    std::fs::write(path, modified).unwrap();
    publish(
        &runtime,
        &fixture,
        request(CASES[1].0, "attribute.copy", json!({"name":"Копия"})),
    );
    let copied = "main:Catalog.Валюты.TabularSection.Представления.Attribute.Копия";
    assert_eq!(
        view(&runtime, &fixture, copied)["props"]["ChoiceForm"],
        form
    );

    let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
    let result = call(
        &runtime,
        &fixture,
        ToolIdentity::Apply,
        json!({"at": "main:Catalog.Валюты.TabularSection.Представления", "ops": [
            {"op":"attribute.copy", "args":{"at":CASES[1].0,"values":{"name":"ЕщеКопия"}}},
            {"op":"attribute.set", "args":{"at":"main:Catalog.Валюты.TabularSection.Представления.Attribute.ЕщеКопия","values":{"choiceForm":form}}},
            {"op":"attribute.set", "args":{"at":"main:Catalog.Валюты.TabularSection.Представления.Attribute.ЕщеКопия","values":{"newName":"Итог"}}}
        ]}),
    );
    assert!(
        !result.ok && result.summary.contains("choiceForm"),
        "{result:?}"
    );
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&fixture.source),
        before
    );
}
