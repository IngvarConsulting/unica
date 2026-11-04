use super::*;

fn color_fixture(value: &str) -> Fixture {
    let fixture = Fixture::blank();
    let configuration = fixture.source.join("Configuration.xml");
    let xml = std::fs::read_to_string(&configuration).unwrap();
    std::fs::write(
        &configuration,
        xml.replace(
            "</ChildObjects>",
            "<StyleItem>Accent</StyleItem></ChildObjects>",
        ),
    )
    .unwrap();
    std::fs::create_dir_all(fixture.source.join("StyleItems")).unwrap();
    std::fs::write(fixture.source.join("StyleItems/Accent.xml"), format!("\u{feff}<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" xmlns:ui=\"http://v8.1c.ru/8.1/data/ui\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xmlns:web=\"http://v8.1c.ru/8.1/data/ui/colors/web\" version=\"2.20\">\r\n<StyleItem uuid=\"1de549ac-39a2-49ce-9f31-c13040173894\"><Properties><Name>Accent</Name><Synonym/><Comment>keep</Comment><Type>Color</Type><Value xsi:type=\"ui:Color\">{value}</Value></Properties></StyleItem>\r\n</MetaDataObject>")).unwrap();
    fixture
}

#[test]
fn canonical_style_color_is_discovered_and_published_without_other_byte_changes() {
    let fixture = color_fixture("#F0F0F0");
    let runtime = runtime();
    let at = "main:StyleItem.Accent";
    let observed = call(&runtime, &fixture, ToolIdentity::View, json!({"at":at}));
    assert!(observed.ok, "{observed:?}");
    assert!(
        observed.data.as_ref().unwrap()["can"]
            .as_array()
            .unwrap()
            .iter()
            .any(|op| op["op"] == "props.set"),
        "Color must expose its value operation: {observed:?}"
    );
    let contract = call(
        &runtime,
        &fixture,
        ToolIdentity::View,
        json!({"at":format!("{at}.Can.props.set")}),
    );
    assert!(contract.ok, "{contract:?}");
    let before = std::fs::read(fixture.source.join("StyleItems/Accent.xml")).unwrap();
    publish(
        &runtime,
        &fixture,
        json!({"at":at,"ops":[{"op":"props.set","args":{"values":{"Value":"web:Gray"}}}]}),
    );
    let expected = String::from_utf8(before)
        .unwrap()
        .replace("#F0F0F0", "web:Gray");
    assert_eq!(
        std::fs::read(fixture.source.join("StyleItems/Accent.xml")).unwrap(),
        expected.as_bytes()
    );
    let observed = call(&runtime, &fixture, ToolIdentity::View, json!({"at":at}));
    assert_eq!(
        observed.data.as_ref().unwrap()["props"]["Value"],
        "web:Gray"
    );
}

#[test]
fn canonical_style_color_cycles_three_forms_and_repeats_without_writes() {
    let fixture = color_fixture("#123456");
    let runtime = runtime();
    let at = "main:StyleItem.Accent";
    for value in ["web:Cream", "style:SpecialTextColor", "#ABCDEF"] {
        publish(
            &runtime,
            &fixture,
            json!({"at":at,"ops":[{"op":"props.set","args":{"values":{"Value":value}}}]}),
        );
        let observed = call(&runtime, &fixture, ToolIdentity::View, json!({"at":at}));
        assert_eq!(observed.data.unwrap()["props"]["Value"], value);
        let before = unica_workspace::test_support::tree_snapshot(&fixture.workspace);
        let plan = call(
            &runtime,
            &fixture,
            ToolIdentity::Apply,
            json!({"at":at,"ops":[{"op":"props.set","args":{"values":{"Value":value}}}]}),
        );
        assert!(plan.ok, "{plan:?}");
        let data = plan.data.unwrap();
        assert_eq!(data["effects"], 0);
        let executed = call(
            &runtime,
            &fixture,
            ToolIdentity::Apply,
            json!({"executionToken":data["executionToken"]}),
        );
        assert!(executed.ok, "{executed:?}");
        assert!(executed.changed.is_empty());
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.workspace),
            before
        );
    }
}

#[test]
fn canonical_non_color_and_opaque_values_have_no_color_contract() {
    for (old, new, value, code) in [
        (
            "<Type>Color</Type>",
            "<Type>Font</Type>",
            "#123456",
            "invalid_state",
        ),
        (
            "<Type>Color</Type>",
            "<Type>Border</Type>",
            "#123456",
            "invalid_state",
        ),
        (
            "xmlns:web=\"http://v8.1c.ru/8.1/data/ui/colors/web\"",
            "xmlns:web=\"urn:future\"",
            "web:Gray",
            "invalid_state",
        ),
        (
            "xmlns:web=\"http://v8.1c.ru/8.1/data/ui/colors/web\"",
            "xmlns:web=\"http://v8.1c.ru/8.1/data/ui/style\"",
            "web:CustomColor",
            "invalid_state",
        ),
        (
            "<Type>Color</Type>",
            "<Type>Color</Type>",
            "#GG0000",
            "invalid_source",
        ),
    ] {
        let fixture = color_fixture(value);
        let runtime = runtime();
        let path = fixture.source.join("StyleItems/Accent.xml");
        let raw = std::fs::read_to_string(&path).unwrap().replace(old, new);
        std::fs::write(&path, raw).unwrap();
        let at = "main:StyleItem.Accent";
        let observed = call(&runtime, &fixture, ToolIdentity::View, json!({"at":at}));
        assert!(observed.ok, "{observed:?}");
        assert!(!observed.data.unwrap()["can"]
            .as_array()
            .unwrap()
            .iter()
            .any(|op| op["op"] == "props.set"));
        let contract = call(
            &runtime,
            &fixture,
            ToolIdentity::View,
            json!({"at":format!("{at}.Can.props.set")}),
        );
        assert!(!contract.ok, "{contract:?}");
        let before = unica_workspace::test_support::tree_snapshot(&fixture.workspace);
        let rejected = call(
            &runtime,
            &fixture,
            ToolIdentity::Apply,
            json!({"at":at,"ops":[{"op":"props.set","args":{"values":{"Value":"#FFFFFF"}}}]}),
        );
        assert!(!rejected.ok, "{rejected:?}");
        assert_eq!(rejected.diagnostics[0]["code"], code, "{rejected:?}");
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.workspace),
            before
        );
        let bad_arg = call(
            &runtime,
            &fixture,
            ToolIdentity::Apply,
            json!({"at":at,"ops":[{"op":"props.set","args":{"values":{"Value":null}}}]}),
        );
        assert!(!bad_arg.ok);
        assert_eq!(
            bad_arg.diagnostics[0]["code"], "bad_value",
            "argument validation precedes descriptor state: {bad_arg:?}"
        );
    }
}

#[test]
fn canonical_style_color_refuses_inexact_owner_versions_and_missing_values() {
    for (file, from, to, code) in [
        (
            "StyleItems/Accent.xml",
            "version=\"2.20\"",
            "version=\"2.19\"",
            "invalid_source",
        ),
        (
            "StyleItems/Accent.xml",
            "version=\"2.20\"",
            "version=\"2.&#50;0\"",
            "invalid_source",
        ),
        (
            "Configuration.xml",
            "version=\"2.20\"",
            "version=\"2.19\"",
            "invalid_source",
        ),
        (
            "StyleItems/Accent.xml",
            "<Value xsi:type=\"ui:Color\">#123456</Value>",
            "",
            "invalid_source",
        ),
        (
            "Configuration.xml",
            "<StyleItem>Accent</StyleItem>",
            "",
            "not_found",
        ),
    ] {
        let fixture = color_fixture("#123456");
        let runtime = runtime();
        let p = fixture.source.join(file);
        let raw = std::fs::read_to_string(&p).unwrap();
        assert!(raw.contains(from));
        std::fs::write(&p, raw.replace(from, to)).unwrap();
        let before = unica_workspace::test_support::tree_snapshot(&fixture.workspace);
        let rejected = call(
            &runtime,
            &fixture,
            ToolIdentity::Apply,
            json!({"at":"main:StyleItem.Accent","ops":[{"op":"props.set","args":{"values":{"Value":"web:Gray"}}}]}),
        );
        assert!(!rejected.ok, "{rejected:?}");
        assert_eq!(
            rejected.diagnostics[0]["code"], code,
            "{file}: {rejected:?}"
        );
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.workspace),
            before
        );
    }
}

#[test]
fn canonical_style_color_bad_arguments_are_closed_and_saved_input_changes_are_fenced() {
    let fixture = color_fixture("#123456");
    let runtime = runtime();
    let at = "main:StyleItem.Accent";
    for values in [
        json!({"Value":"web:NotAColor"}),
        json!({"Value":"#123"}),
        json!({"Value":false}),
        json!({"Value":123}),
        json!({"Value":{}}),
        json!({"Value":[]}),
        json!({"Value":"#FFFFFF","Type":"Font"}),
        json!({"Comment":"out of scope"}),
    ] {
        let before = unica_workspace::test_support::tree_snapshot(&fixture.workspace);
        let refused = call(
            &runtime,
            &fixture,
            ToolIdentity::Apply,
            json!({"at":at,"ops":[{"op":"props.set","args":{"values":values}}]}),
        );
        assert!(!refused.ok, "{refused:?}");
        assert_eq!(refused.diagnostics[0]["code"], "bad_value");
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.workspace),
            before
        );
    }
    let missing = call(
        &runtime,
        &fixture,
        ToolIdentity::Apply,
        json!({"at":"main:StyleItem.Missing","ops":[{"op":"props.set","args":{"values":{"Value":"#FFFFFF"}}}]}),
    );
    assert!(!missing.ok);
    assert_eq!(missing.diagnostics[0]["code"], "not_found", "{missing:?}");
    let plan = call(
        &runtime,
        &fixture,
        ToolIdentity::Apply,
        json!({"at":at,"ops":[{"op":"props.set","args":{"values":{"Value":"web:Gray"}}}]}),
    );
    assert!(plan.ok, "{plan:?}");
    let path = fixture.source.join("StyleItems/Accent.xml");
    let external = std::fs::read_to_string(&path).unwrap().replace(
        "<Comment>keep</Comment>",
        "<Comment>external edit</Comment>",
    );
    std::fs::write(&path, external.as_bytes()).unwrap();
    let refused = call(
        &runtime,
        &fixture,
        ToolIdentity::Apply,
        json!({"executionToken":plan.data.unwrap()["executionToken"]}),
    );
    assert!(!refused.ok, "{refused:?}");
    assert_eq!(refused.diagnostics[0]["code"], "stale_revision");
    assert_eq!(std::fs::read(&path).unwrap(), external.as_bytes());
}

#[test]
fn canonical_style_color_contract_does_not_outlive_parent_format_admission() {
    for version in ["2.19", "2.21", "2.&#50;0"] {
        let fixture = color_fixture("#123456");
        let runtime = runtime();
        let path = fixture.source.join("Configuration.xml");
        let source = std::fs::read_to_string(&path).unwrap();
        std::fs::write(
            &path,
            source.replace("version=\"2.20\"", &format!("version=\"{version}\"")),
        )
        .unwrap();
        let observed = call(
            &runtime,
            &fixture,
            ToolIdentity::View,
            json!({"at":"main:StyleItem.Accent"}),
        );
        assert!(observed.ok, "{version}: {observed:?}");
        let data = observed.data.unwrap();
        assert_eq!(data["props"]["Value"], "#123456");
        assert!(
            !data["can"]
                .as_array()
                .unwrap()
                .iter()
                .any(|operation| operation["op"] == "props.set"),
            "{version}: unsupported parent cannot grant a Color writer: {data}"
        );
        let contract = call(
            &runtime,
            &fixture,
            ToolIdentity::View,
            json!({"at":"main:StyleItem.Accent.Can.props.set"}),
        );
        assert!(!contract.ok, "{version}: {contract:?}");
    }
}
