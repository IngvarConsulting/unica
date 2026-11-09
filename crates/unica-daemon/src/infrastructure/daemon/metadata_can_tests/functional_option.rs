use super::*;

fn args(values: Value) -> Value {
    json!({"at":"main:FunctionalOption.ByConstant", "ops":[{"op":"props.set", "args":{"values":values}}]})
}

#[test]
fn functional_option_properties_publish_exact_values_and_preserve_other_bytes() {
    mixed_comment_properties_preserve_bytes_and_noop();
    let runtime = runtime();
    let fixture = Fixture::new("workspace-functional-option");
    let path = fixture.source.join("FunctionalOptions/ByConstant.xml");
    let before = std::fs::read_to_string(&path).unwrap();
    let can = contract(
        &runtime,
        &fixture,
        "main:FunctionalOption.ByConstant",
        "props.set",
    );
    assert!(can.to_string().contains("PrivilegedGetMode"), "{can}");
    assert_eq!(
        view(&runtime, &fixture, "main:FunctionalOption.ByConstant")["props"]["Location"],
        "Constant.Enabled"
    );
    let mut previous = before;
    for (location, flag) in [
        ("Catalog.Options.Attribute.Enabled", false),
        (
            "Catalog.Options.TabularSection.Lines.Attribute.Enabled",
            true,
        ),
        ("Document.ProbeDoc.Attribute.Enabled", false),
        ("InformationRegister.Settings.Attribute.Extra", true),
        ("AccumulationRegister.ProbeAccum.Resource.Enabled", false),
        ("InformationRegister.Settings.Resource.Enabled", true),
        ("Constant.Enabled", false),
    ] {
        let before_image = unica_workspace::test_support::tree_snapshot(&fixture.source);
        publish(
            &runtime,
            &fixture,
            args(json!({"Location":location, "PrivilegedGetMode":flag})),
        );
        let node = view(&runtime, &fixture, "main:FunctionalOption.ByConstant");
        assert_eq!(node["props"]["Location"], location);
        assert_eq!(node["props"]["PrivilegedGetMode"], flag);
        let current = std::fs::read_to_string(&path).unwrap();
        let old =
            unica_metadata_designer::functional_option::read(&previous, "ByConstant").unwrap();
        assert_eq!(
            current,
            previous
                .replace(
                    &format!("<Location>{}</Location>", old.0),
                    &format!("<Location>{location}</Location>")
                )
                .replace(
                    &format!("<PrivilegedGetMode>{}</PrivilegedGetMode>", old.1),
                    &format!("<PrivilegedGetMode>{flag}</PrivilegedGetMode>")
                )
        );
        for (relative, bytes) in before_image {
            if relative != Path::new("FunctionalOptions/ByConstant.xml") {
                assert_eq!(std::fs::read(fixture.source.join(relative)).unwrap(), bytes);
            }
        }
        previous = current;
    }
    let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
    publish(
        &runtime,
        &fixture,
        args(json!({"location":"Constant.Enabled", "privilegedGetMode":false})),
    );
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&fixture.source),
        before
    );
}

#[test]
fn functional_option_invalid_inputs_and_stale_storage_write_nothing() {
    mixed_comment_refusals_and_comment_only_with_invalid_storage();
    let runtime = runtime();
    let fixture = Fixture::new("workspace-functional-option");
    let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
    for values in [
        json!({"Location":""}),
        json!({"Location":"Constant.Missing"}),
        json!({"Location":"Catalog.Options"}),
        json!({"Location":"Catalog.Options.Attribute.Missing"}),
        json!({"Location":"InformationRegister.Settings.Dimension.Enabled"}),
        json!({"Location":"../Enabled"}),
        json!({"PrivilegedGetMode":"true"}),
        json!({"PrivilegedGetMode":null}),
        json!({"Location":"Constant.Enabled", "location":"Constant.Enabled"}),
        json!({"Content":[]}),
    ] {
        let result = call(&runtime, &fixture, ToolIdentity::Apply, args(values));
        assert!(!result.ok, "{result:?}");
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.source),
            before
        );
    }
    let mut batch = args(json!({"PrivilegedGetMode":false}));
    batch["ops"]
        .as_array_mut()
        .unwrap()
        .push(json!({"op":"props.set", "args":{"values":{"Location":"Constant.Missing"}}}));
    let refused = call(&runtime, &fixture, ToolIdentity::Apply, batch);
    assert!(!refused.ok, "{refused:?}");
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&fixture.source),
        before
    );
    let planned = call(
        &runtime,
        &fixture,
        ToolIdentity::Apply,
        args(json!({"Location":"Constant.Enabled", "PrivilegedGetMode":false})),
    );
    assert!(planned.ok, "{planned:?}");
    let path = fixture.source.join("Constants/Enabled.xml");
    let bytes = std::fs::read(&path).unwrap();
    std::fs::write(
        &path,
        String::from_utf8(bytes)
            .unwrap()
            .replace("<Comment/>", "<Comment>Concurrent</Comment>"),
    )
    .unwrap();
    let raced = unica_workspace::test_support::tree_snapshot(&fixture.source);
    let result = call(
        &runtime,
        &fixture,
        ToolIdentity::Apply,
        json!({"executionToken":planned.data.unwrap()["executionToken"]}),
    );
    assert!(!result.ok, "{result:?}");
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&fixture.source),
        raced
    );
}

fn mixed_comment_properties_preserve_bytes_and_noop() {
    let runtime = runtime();
    let fixture = Fixture::new("workspace-functional-option");
    let path = fixture.source.join("FunctionalOptions/ByConstant.xml");
    let original = std::fs::read_to_string(&path).unwrap();
    let original = format!(
        "\u{feff}{}",
        original
            .trim_start_matches('\u{feff}')
            .replace("\r\n", "\n")
            .replace("\n", "\r\n")
            .replacen("<Name>", "\n<Name>", 1)
    );
    std::fs::write(&path, &original).unwrap();
    let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
    let values = json!({"comment":"Текст & <>\r\n\t", "location":"Catalog.Options.Attribute.Enabled", "privilegedGetMode":false});
    let preview = call(
        &runtime,
        &fixture,
        ToolIdentity::Apply,
        args(values.clone()),
    );
    assert!(preview.ok, "{preview:?}");
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&fixture.source),
        before
    );
    let result = call(
        &runtime,
        &fixture,
        ToolIdentity::Apply,
        json!({"executionToken":preview.data.unwrap()["executionToken"]}),
    );
    assert!(result.ok, "{result:?}");
    let expected = original
        .replace(
            "<Comment>Preserve issue 1650 comment</Comment>",
            "<Comment>Текст &amp; &lt;&gt;&#13;&#10;&#9;</Comment>",
        )
        .replace(
            "<Location>Constant.Enabled</Location>",
            "<Location>Catalog.Options.Attribute.Enabled</Location>",
        )
        .replace(
            "<PrivilegedGetMode>true</PrivilegedGetMode>",
            "<PrivilegedGetMode>false</PrivilegedGetMode>",
        );
    assert_eq!(std::fs::read(&path).unwrap(), expected.as_bytes());
    let node = view(&runtime, &fixture, "main:FunctionalOption.ByConstant");
    assert_eq!(node["props"]["Comment"], values["comment"]);
    assert_eq!(node["props"]["Location"], values["location"]);
    assert_eq!(node["props"]["PrivilegedGetMode"], false);
    for (relative, bytes) in before {
        if relative != Path::new("FunctionalOptions/ByConstant.xml") {
            assert_eq!(std::fs::read(fixture.source.join(relative)).unwrap(), bytes);
        }
    }
    let published = unica_workspace::test_support::tree_snapshot(&fixture.source);
    publish(&runtime, &fixture, args(values.clone()));
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&fixture.source),
        published
    );
    // Even a mixed no-op must retain the storage descriptor preimage.
    let preview = call(&runtime, &fixture, ToolIdentity::Apply, args(values));
    assert!(preview.ok, "{preview:?}");
    assert!(preview.data.as_ref().unwrap()["executionToken"].is_string());
    let storage = fixture.source.join("Catalogs/Options.xml");
    let text = std::fs::read_to_string(&storage).unwrap();
    let changed = text.replace("<Comment/>", "<Comment>Concurrent</Comment>");
    assert_ne!(
        changed, text,
        "the storage race must actually change its preimage"
    );
    std::fs::write(&storage, changed).unwrap();
    let raced = unica_workspace::test_support::tree_snapshot(&fixture.source);
    let stale = call(
        &runtime,
        &fixture,
        ToolIdentity::Apply,
        json!({"executionToken":preview.data.unwrap()["executionToken"]}),
    );
    assert!(!stale.ok, "{stale:?}");
    assert!(
        stale
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic["code"] == "stale_revision"),
        "{stale:?}"
    );
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&fixture.source),
        raced
    );
}

fn mixed_comment_refusals_and_comment_only_with_invalid_storage() {
    let runtime = runtime();
    let fixture = Fixture::new("workspace-functional-option");
    let path = fixture.source.join("FunctionalOptions/ByConstant.xml");
    let original = std::fs::read_to_string(&path).unwrap();
    for comment in [
        "<Comment/><Comment/>",
        "<Comment><Nested/></Comment>",
        "<Comment xmlns=\"foreign\"/>",
        "",
    ] {
        std::fs::write(
            &path,
            original.replace("<Comment>Preserve issue 1650 comment</Comment>", comment),
        )
        .unwrap();
        let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
        let rejected = call(
            &runtime,
            &fixture,
            ToolIdentity::Apply,
            args(
                json!({"Location":"Catalog.Options.Attribute.Enabled", "PrivilegedGetMode":false, "Comment":"bad"}),
            ),
        );
        assert!(!rejected.ok, "{rejected:?}");
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.source),
            before
        );
    }
    std::fs::write(&path, &original).unwrap();
    let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
    for values in [
        json!({"Location":"Catalog.Options.Attribute.Enabled", "Comment":"a", "comment":"b"}),
        json!({"Unknown":"x"}),
        json!({"Synonym":"x"}),
        json!({"Name":"x"}),
    ] {
        let rejected = call(&runtime, &fixture, ToolIdentity::Apply, args(values));
        assert!(!rejected.ok, "{rejected:?}");
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.source),
            before
        );
    }
    for (from, to) in [
        ("Constant.Enabled", "Constant.Missing"),
        ("<Location>Constant.Enabled</Location>", ""),
        (
            "<PrivilegedGetMode>true</PrivilegedGetMode>",
            "<PrivilegedGetMode>invalid</PrivilegedGetMode>",
        ),
    ] {
        let source = original.replace(from, to);
        std::fs::write(&path, &source).unwrap();
        publish(&runtime, &fixture, args(json!({"Comment":"independent"})));
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            source.replace("Preserve issue 1650 comment", "independent")
        );
        let node = view(&runtime, &fixture, "main:FunctionalOption.ByConstant");
        assert_eq!(node["props"]["Comment"], "independent");
    }
}
