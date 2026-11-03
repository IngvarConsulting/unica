use super::*;
#[test]
fn field_input_format_can_depends_on_observed_type() {
    let fixture = Fixture::new("workspace-field-input-format");
    let runtime = runtime();
    for (at, op, number, constant) in [
        (
            "main:Catalog.Codes.Attribute.Key",
            "attribute.set",
            false,
            false,
        ),
        (
            "main:Catalog.Codes.Attribute.Amount",
            "attribute.set",
            true,
            false,
        ),
        (
            "main:Catalog.Codes.TabularSection.Rows.Attribute.Text",
            "attribute.set",
            false,
            false,
        ),
        (
            "main:Catalog.Codes.TabularSection.Rows.Attribute.Amount",
            "attribute.set",
            true,
            false,
        ),
        ("main:Constant.Setting", "props.set", false, true),
        ("main:Constant.Amount", "props.set", true, true),
    ] {
        let can = contract(&runtime, &fixture, at, op);
        let fields = &can["argsSchema"]["properties"]["values"]["anyOf"][0]["properties"];
        for (xml, key, is_number) in [
            ("PasswordMode", "passwordMode", false),
            ("MultiLine", "multiLine", false),
            ("ExtendedEdit", "extendedEdit", false),
            ("Mask", "mask", false),
            ("MarkNegatives", "markNegatives", true),
            ("Format", "format", true),
            ("EditFormat", "editFormat", true),
        ] {
            assert_eq!(
                fields.get(if constant { xml } else { key }).is_some(),
                number == is_number,
                "{at} {key}: {fields}"
            );
        }
    }
    let can = contract(&runtime, &fixture, "main:Constant.Flag", "props.set");
    for key in [
        "PasswordMode",
        "MultiLine",
        "ExtendedEdit",
        "Mask",
        "MarkNegatives",
        "Format",
        "EditFormat",
    ] {
        assert!(
            can["argsSchema"]["properties"]["values"]["anyOf"][0]["properties"]
                .get(key)
                .is_none()
        );
    }
}

fn property_range(
    xml: &str,
    name: &str,
    tabular: bool,
    constant: bool,
    key: &str,
) -> std::ops::Range<usize> {
    let doc = roxmltree::Document::parse(xml).unwrap();
    let node = doc
        .descendants()
        .find(|node| {
            if !node.is_element()
                || node.tag_name().name() != if constant { "Constant" } else { "Attribute" }
            {
                return false;
            }
            let in_section = node.ancestors().any(|ancestor| {
                ancestor.is_element() && ancestor.tag_name().name() == "TabularSection"
            });
            in_section == tabular
                && node
                    .children()
                    .find(|n| n.is_element() && n.tag_name().name() == "Properties")
                    .and_then(|p| {
                        p.children()
                            .find(|n| n.is_element() && n.tag_name().name() == "Name")
                    })
                    .and_then(|n| n.text())
                    == Some(name)
        })
        .unwrap();
    node.children()
        .find(|n| n.is_element() && n.tag_name().name() == "Properties")
        .unwrap()
        .children()
        .find(|n| n.is_element() && n.tag_name().name() == key)
        .unwrap()
        .range()
}
fn verify_input_settings(
    at: &str,
    file: &str,
    name: &str,
    tabular: bool,
    constant: bool,
    number: bool,
) {
    let fixture = Fixture::new("workspace-field-input-format");
    let runtime = runtime();
    let path = fixture.source.join(file);
    let op = if constant {
        "props.set"
    } else {
        "attribute.set"
    };
    let original = unica_workspace::test_support::tree_snapshot(&fixture.source);
    let initial = String::from_utf8(original[&PathBuf::from(file)].clone()).unwrap();
    let cases = if number {
        vec![
            ("MarkNegatives", "markNegatives", json!(true), json!(false)),
            ("Format", "format", json!("ЧДЦ=2; ЧГ=0"), json!("")),
            ("EditFormat", "editFormat", json!("ЧДЦ=3"), json!("")),
        ]
    } else {
        vec![
            ("PasswordMode", "passwordMode", json!(true), json!(false)),
            ("MultiLine", "multiLine", json!(true), json!(false)),
            ("ExtendedEdit", "extendedEdit", json!(true), json!(false)),
            ("Mask", "mask", json!("999-999"), json!("")),
        ]
    };
    for (xml, member, value, reset) in cases {
        let key = if constant { xml } else { member };
        for value in [&value, &reset] {
            let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
            publish(
                &runtime,
                &fixture,
                json!({"at":at,"ops":[{"op":op,"args":{"values":{key:value}}}]}),
            );
            let after = unica_workspace::test_support::tree_snapshot(&fixture.source);
            let before_xml = String::from_utf8(before[&PathBuf::from(file)].clone()).unwrap();
            let after_xml = String::from_utf8(after[&PathBuf::from(file)].clone()).unwrap();
            let mut stripped_before = before_xml.clone();
            stripped_before.replace_range(
                property_range(&before_xml, name, tabular, constant, xml),
                "",
            );
            let mut stripped_after = after_xml.clone();
            stripped_after
                .replace_range(property_range(&after_xml, name, tabular, constant, xml), "");
            assert_eq!(stripped_before, stripped_after);
            let mut expected = before;
            expected.insert(PathBuf::from(file), after[&PathBuf::from(file)].clone());
            assert_eq!(after, expected);
            let viewed = view(&runtime, &fixture, at);
            let props = &viewed["props"];
            if matches!(xml, "Format" | "EditFormat") {
                let translations = props[xml].as_array().unwrap();
                assert!(translations
                    .iter()
                    .any(|t| t["language"] == "en" && t["content"] == "ND=1"));
                if value == "" {
                    assert!(!translations.iter().any(|t| t["language"] == "ru"));
                } else {
                    assert!(translations
                        .iter()
                        .any(|t| t["language"] == "ru" && &t["content"] == value));
                }
                let before_prop =
                    &before_xml[property_range(&before_xml, name, tabular, constant, xml)];
                let after_prop =
                    &after_xml[property_range(&after_xml, name, tabular, constant, xml)];
                let en_start = before_prop.find("<v8:item>").unwrap();
                let en_end = before_prop[en_start..].find("</v8:item>").unwrap()
                    + en_start
                    + "</v8:item>".len();
                assert!(after_prop.contains(&before_prop[en_start..en_end]));
            } else if value == "" {
                assert!(props.get(xml).is_none() || props[xml] == "");
            } else {
                assert_eq!(&props[xml], value);
            }
            if !constant {
                let collection = view(&runtime, &fixture, at.rsplit_once('.').unwrap().0);
                let row = collection["items"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|row| row["at"] == at)
                    .unwrap();
                assert_eq!(row["props"].get(xml), props.get(xml));
            }
            let before_repeat = unica_workspace::test_support::tree_snapshot(&fixture.source);
            publish(
                &runtime,
                &fixture,
                json!({"at":at,"ops":[{"op":op,"args":{"values":{key:value}}}]}),
            );
            assert_eq!(
                unica_workspace::test_support::tree_snapshot(&fixture.source),
                before_repeat
            );
        }
        let wrong = if value.is_boolean() {
            json!("true")
        } else {
            json!(false)
        };
        let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
        let refused = call(
            &runtime,
            &fixture,
            ToolIdentity::Apply,
            json!({"at":at,"ops":[{"op":op,"args":{"values":{key:wrong}}}]}),
        );
        assert!(!refused.ok, "{refused:?}");
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.source),
            before
        );
        let range = property_range(&initial, name, tabular, constant, xml);
        let old = &initial[range.clone()];
        for bad in [
            String::new(),
            format!("{old}{old}"),
            format!("<{xml} xmlns=\"foreign\">false</{xml}>"),
            format!("<{xml}><bad/></{xml}>"),
            if matches!(xml, "Format" | "EditFormat") {
                format!("<{xml}>raw text</{xml}>")
            } else if value.is_boolean() {
                format!("<{xml}>unknown</{xml}>")
            } else {
                format!("<{xml} bad=\"1\"/>")
            },
        ] {
            let mut broken = initial.clone();
            broken.replace_range(range.clone(), &bad);
            std::fs::write(&path, broken).unwrap();
            let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
            let refused = call(
                &runtime,
                &fixture,
                ToolIdentity::Apply,
                json!({"at":at,"ops":[{"op":op,"args":{"values":{key:value}}}]}),
            );
            assert!(!refused.ok, "{at} {xml} {bad}: {refused:?}");
            assert_eq!(
                unica_workspace::test_support::tree_snapshot(&fixture.source),
                before
            );
        }
        std::fs::write(&path, &initial).unwrap();
    }
    let invalid = if number {
        if constant {
            "PasswordMode"
        } else {
            "passwordMode"
        }
    } else if constant {
        "MarkNegatives"
    } else {
        "markNegatives"
    };
    let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
    let refused = call(
        &runtime,
        &fixture,
        ToolIdentity::Apply,
        json!({"at":at,"ops":[{"op":op,"args":{"values":{invalid:true}}}]}),
    );
    assert!(!refused.ok, "{refused:?}");
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&fixture.source),
        before
    );
}
#[test]
fn field_input_format_header_string() {
    verify_input_settings(
        "main:Catalog.Codes.Attribute.Key",
        "Catalogs/Codes.xml",
        "Key",
        false,
        false,
        false,
    );
}
#[test]
fn field_input_format_header_number() {
    verify_input_settings(
        "main:Catalog.Codes.Attribute.Amount",
        "Catalogs/Codes.xml",
        "Amount",
        false,
        false,
        true,
    );
}
#[test]
fn field_input_format_tabular_string() {
    verify_input_settings(
        "main:Catalog.Codes.TabularSection.Rows.Attribute.Text",
        "Catalogs/Codes.xml",
        "Text",
        true,
        false,
        false,
    );
}
#[test]
fn field_input_format_tabular_number() {
    verify_input_settings(
        "main:Catalog.Codes.TabularSection.Rows.Attribute.Amount",
        "Catalogs/Codes.xml",
        "Amount",
        true,
        false,
        true,
    );
}
#[test]
fn field_input_format_constant_string() {
    verify_input_settings(
        "main:Constant.Setting",
        "Constants/Setting.xml",
        "Setting",
        false,
        true,
        false,
    );
}
#[test]
fn field_input_format_constant_number() {
    verify_input_settings(
        "main:Constant.Amount",
        "Constants/Amount.xml",
        "Amount",
        false,
        true,
        true,
    );
}

#[test]
fn field_input_format_explicit_type_matches_can_and_preview() {
    for (at, op, type_key, number_key, string_key) in [
        (
            "main:Constant.Setting",
            "props.set",
            "Type",
            "MarkNegatives",
            "Mask",
        ),
        (
            "main:Catalog.Codes.Attribute.Key",
            "attribute.set",
            "type",
            "markNegatives",
            "mask",
        ),
    ] {
        let fixture = Fixture::new("workspace-field-input-format");
        let runtime = runtime();
        let number = json!({"variants":[{"kind":"number","digits":12,"fraction":2,"sign":"any"}]});
        let string = json!({"variants":[{"kind":"string","length":20,"allowedLength":"variable"}]});
        let boolean = json!({"variants":[{"kind":"boolean"}]});
        let can = contract(&runtime, &fixture, at, op);
        let wrong = json!({"values":{type_key:number,string_key:"999"}});
        assert!(!jsonschema::is_valid(&can["argsSchema"], &wrong));
        let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
        let refused = call(
            &runtime,
            &fixture,
            ToolIdentity::Apply,
            json!({"at":at,"ops":[{"op":op,"args":wrong}]}),
        );
        assert!(!refused.ok, "{refused:?}");
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.source),
            before
        );
        let type_only = json!({"values":{type_key:boolean}});
        assert!(jsonschema::is_valid(&can["argsSchema"], &type_only));
        let plan = call(
            &runtime,
            &fixture,
            ToolIdentity::Apply,
            json!({"at":at,"ops":[{"op":op,"args":type_only}]}),
        );
        assert!(plan.ok, "{plan:?}");
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.source),
            before
        );
        for args in [
            json!({"values":{type_key:number,number_key:true}}),
            json!({"values":{type_key:string,string_key:"999"}}),
        ] {
            let can = contract(&runtime, &fixture, at, op);
            assert!(
                jsonschema::is_valid(&can["argsSchema"], &args),
                "{can} {args}"
            );
            publish(
                &runtime,
                &fixture,
                json!({"at":at,"ops":[{"op":op,"args":args}]}),
            );
        }
    }
}

#[test]
fn field_input_format_retains_default_language_and_type_inputs() {
    for change_language in [true, false] {
        let fixture = Fixture::new("workspace-field-input-format");
        let runtime = runtime();
        let plan = call(
            &runtime,
            &fixture,
            ToolIdentity::Apply,
            json!({"at":"main:Constant.Amount","ops":[{"op":"props.set","args":{"values":{"Format":"ЧДЦ=2"}}}]}),
        );
        assert!(plan.ok, "{plan:?}");
        if change_language {
            let path = fixture.source.join("Languages/Русский.xml");
            let text = std::fs::read_to_string(&path).unwrap();
            assert!(text.contains("<LanguageCode>ru</LanguageCode>"));
            std::fs::write(
                path,
                text.replace(
                    "<LanguageCode>ru</LanguageCode>",
                    "<LanguageCode>de</LanguageCode>",
                ),
            )
            .unwrap();
        } else {
            publish(
                &runtime,
                &fixture,
                json!({"at":"main:Constant.Amount","ops":[{"op":"props.set","args":{"values":{"Type":{"variants":[{"kind":"string","length":20,"allowedLength":"variable"}]}}}}]}),
            );
        }
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
}

#[test]
fn field_input_format_preserves_local_namespace_bindings() {
    let fixture = Fixture::new("workspace-field-input-format");
    let runtime = runtime();
    let md = "http://v8.1c.ru/8.3/MDClasses";
    let core = "http://v8.1c.ru/8.1/data/core";
    for (at, file, name, constant) in [
        (
            "main:Constant.Amount",
            "Constants/Amount.xml",
            "Amount",
            true,
        ),
        (
            "main:Catalog.Codes.Attribute.Amount",
            "Catalogs/Codes.xml",
            "Amount",
            false,
        ),
    ] {
        let path = fixture.source.join(file);
        let original = std::fs::read_to_string(&path).unwrap();
        for property in [
            format!(r#"<v8:Format xmlns:v8="{md}"/>"#),
            format!(
                r#"<v8:Format xmlns:v8="{md}"><c:item xmlns:c="{core}"><c:lang>en</c:lang><c:content>ND=1</c:content></c:item></v8:Format>"#
            ),
            format!(
                r#"<Format xmlns:d="{core}"><d:item><d:lang>ru</d:lang><c:content xmlns:c="{core}" xmlns:note="urn:a>b">old</c:content></d:item></Format>"#
            ),
        ] {
            let mut input = original.clone();
            input.replace_range(
                property_range(&input, name, false, constant, "Format"),
                &property,
            );
            std::fs::write(&path, &input).unwrap();
            let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
            let op = if constant {
                "props.set"
            } else {
                "attribute.set"
            };
            let key = if constant { "Format" } else { "format" };
            publish(
                &runtime,
                &fixture,
                json!({"at":at,"ops":[{"op":op,"args":{"values":{key:"ЧДЦ=2"}}}]}),
            );
            let output = std::fs::read_to_string(&path).unwrap();
            let range = property_range(&output, name, false, constant, "Format");
            let mut expected = input;
            let prior = property_range(&expected, name, false, constant, "Format");
            expected.replace_range(prior, &output[range]);
            assert_eq!(expected, output);
            let mut expected_tree = before;
            expected_tree.insert(PathBuf::from(file), output.into_bytes());
            assert_eq!(
                unica_workspace::test_support::tree_snapshot(&fixture.source),
                expected_tree
            );
            let values = view(&runtime, &fixture, at)["props"]["Format"]
                .as_array()
                .unwrap()
                .clone();
            assert!(values
                .iter()
                .any(|v| v["language"] == "ru" && v["content"] == "ЧДЦ=2"));
            if property.contains("ND=1") {
                assert!(values
                    .iter()
                    .any(|v| v["language"] == "en" && v["content"] == "ND=1"));
            }
        }
        std::fs::write(path, original).unwrap();
    }
}
