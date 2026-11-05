use super::*;

const OWNER: &str = "main:CommonPicture.Probe";
const DESCRIPTOR: &str = "CommonPictures/Probe.xml";

fn args(values: Value) -> Value {
    json!({"at":OWNER,"ops":[{"op":"props.set","args":{"values":values}}]})
}

#[test]
fn common_picture_flags_publish_independently_and_preserve_all_other_bytes() {
    let runtime = runtime();
    let fixture = Fixture::new("workspace-common-picture");
    let contract = contract(&runtime, &fixture, OWNER, "props.set");
    assert_eq!(
        contract["argsSchema"]["properties"]["values"]["properties"]["AvailabilityForChoice"]
            ["type"],
        "boolean"
    );
    assert!(view(&runtime, &fixture, "main:CommonPicture")
        .to_string()
        .contains(OWNER));
    let path = fixture.source.join(DESCRIPTOR);
    let mut previous = std::fs::read(&path).unwrap();
    let mut old_choice = false;
    let mut old_appearance = false;
    for (values, choice, appearance) in [
        (json!({"AvailabilityForChoice":true}), true, false),
        (json!({"AvailabilityForChoice":false}), false, false),
        (json!({"AvailabilityForAppearance":true}), false, true),
        (json!({"AvailabilityForAppearance":false}), false, false),
        (
            json!({"AvailabilityForChoice":true,"AvailabilityForAppearance":true}),
            true,
            true,
        ),
        (
            json!({"availabilityForChoice":false,"availabilityForAppearance":false}),
            false,
            false,
        ),
        (json!({"AvailabilityForChoice":false}), false, false),
    ] {
        let image = unica_workspace::test_support::tree_snapshot(&fixture.source);
        let old = String::from_utf8(previous.clone()).unwrap();
        publish(&runtime, &fixture, args(values));
        let node = view(&runtime, &fixture, OWNER);
        assert_eq!(node["props"]["AvailabilityForChoice"], choice);
        assert_eq!(node["props"]["AvailabilityForAppearance"], appearance);
        let expected = old
            .replace(
                &format!("<AvailabilityForChoice>{old_choice}</AvailabilityForChoice>"),
                &format!("<AvailabilityForChoice>{choice}</AvailabilityForChoice>"),
            )
            .replace(
                &format!("<AvailabilityForAppearance>{old_appearance}</AvailabilityForAppearance>"),
                &format!("<AvailabilityForAppearance>{appearance}</AvailabilityForAppearance>"),
            );
        previous = std::fs::read(&path).unwrap();
        assert_eq!(previous, expected.as_bytes());
        old_choice = choice;
        old_appearance = appearance;
        for (relative, bytes) in image {
            if relative != Path::new(DESCRIPTOR) {
                assert_eq!(std::fs::read(fixture.source.join(relative)).unwrap(), bytes);
            }
        }
    }
}

#[test]
fn common_picture_bad_arguments_and_failed_batches_write_nothing() {
    let runtime = runtime();
    let fixture = Fixture::new("workspace-common-picture");
    let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
    for values in [
        json!({"AvailabilityForChoice":"false"}),
        json!({"AvailabilityForAppearance":"true"}),
        json!({"AvailabilityForChoice":1}),
        json!({"AvailabilityForChoice":null}),
        json!({"Unknown":true}),
        json!({"Comment":"no"}),
        json!({"AvailabilityForChoice":true,"availabilityForChoice":false}),
    ] {
        let result = call(&runtime, &fixture, ToolIdentity::Apply, args(values));
        assert!(!result.ok, "{result:?}");
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.source),
            before
        );
    }
    let mut wrong = args(json!({"AvailabilityForChoice":true}));
    wrong["at"] = json!("main:Language.Русский");
    assert!(!call(&runtime, &fixture, ToolIdentity::Apply, wrong).ok);
    let mut batch = args(json!({"AvailabilityForChoice":true}));
    batch["ops"]
        .as_array_mut()
        .unwrap()
        .push(json!({"op":"props.set","args":{"values":{"AvailabilityForAppearance":"false"}}}));
    assert!(!call(&runtime, &fixture, ToolIdentity::Apply, batch).ok);
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&fixture.source),
        before
    );
}

#[test]
fn common_picture_missing_damaged_and_unregistered_fields_refuse_without_writes() {
    let runtime = runtime();
    for replacement in [
        "",
        "<AvailabilityForChoice>broken</AvailabilityForChoice>",
        "<AvailabilityForChoice xmlns=\"urn:foreign\">false</AvailabilityForChoice>",
    ] {
        let fixture = Fixture::new("workspace-common-picture");
        let path = fixture.source.join(DESCRIPTOR);
        let text = std::fs::read_to_string(&path).unwrap().replace(
            "<AvailabilityForChoice>false</AvailabilityForChoice>",
            replacement,
        );
        std::fs::write(&path, text).unwrap();
        let node = view(&runtime, &fixture, OWNER);
        assert!(node["props"].get("AvailabilityForChoice").is_none());
        let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
        let result = call(
            &runtime,
            &fixture,
            ToolIdentity::Apply,
            args(json!({"AvailabilityForChoice":true})),
        );
        assert!(!result.ok, "{result:?}");
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.source),
            before
        );
    }
    let fixture = Fixture::new("workspace-common-picture");
    let path = fixture.source.join("Configuration.xml");
    std::fs::write(
        &path,
        std::fs::read_to_string(&path)
            .unwrap()
            .replace("<CommonPicture>Probe</CommonPicture>", ""),
    )
    .unwrap();
    let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
    assert!(
        !call(
            &runtime,
            &fixture,
            ToolIdentity::Apply,
            args(json!({"AvailabilityForChoice":true}))
        )
        .ok
    );
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&fixture.source),
        before
    );
}

fn assert_retained_race(relative: &str, value: bool) {
    let runtime = runtime();
    let fixture = Fixture::new("workspace-common-picture");
    let planned = call(
        &runtime,
        &fixture,
        ToolIdentity::Apply,
        args(json!({"AvailabilityForChoice":value})),
    );
    assert!(planned.ok, "{planned:?}");
    let path = fixture.source.join(relative);
    let mut bytes = std::fs::read(&path).unwrap();
    bytes.extend_from_slice(b"<!--concurrent-->");
    std::fs::write(path, bytes).unwrap();
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

#[test]
fn common_picture_changed_descriptor_invalidates_saved_write() {
    assert_retained_race(DESCRIPTOR, true);
}

#[test]
fn common_picture_changed_registration_invalidates_saved_write() {
    assert_retained_race("Configuration.xml", true);
}

#[test]
fn common_picture_changed_descriptor_invalidates_saved_noop() {
    assert_retained_race(DESCRIPTOR, false);
}

#[test]
fn common_picture_changed_registration_invalidates_saved_noop() {
    assert_retained_race("Configuration.xml", false);
}

#[test]
fn common_picture_requires_exact_writable_owner_versions() {
    let runtime = runtime();
    for relative in [DESCRIPTOR, "Configuration.xml"] {
        for version in ["2.21", "2.&#50;0"] {
            let fixture = Fixture::new("workspace-common-picture");
            let path = fixture.source.join(relative);
            std::fs::write(
                &path,
                std::fs::read_to_string(&path)
                    .unwrap()
                    .replace("version=\"2.20\"", &format!("version=\"{version}\"")),
            )
            .unwrap();
            let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
            assert!(
                !call(
                    &runtime,
                    &fixture,
                    ToolIdentity::Apply,
                    args(json!({"AvailabilityForChoice":true}))
                )
                .ok
            );
            assert_eq!(
                unica_workspace::test_support::tree_snapshot(&fixture.source),
                before
            );
        }
    }
}
