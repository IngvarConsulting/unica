use super::*;

fn args(values: Value) -> Value {
    json!({"at":"main:FunctionalOption.ByConstant", "ops":[{"op":"props.set", "args":{"values":values}}]})
}

#[test]
fn functional_option_properties_publish_exact_values_and_preserve_other_bytes() {
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
