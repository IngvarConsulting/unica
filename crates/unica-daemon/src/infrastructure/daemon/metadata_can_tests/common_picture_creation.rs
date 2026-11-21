use super::*;

const NAME: &str = "EmptyPicture";
const AT: &str = "main:CommonPicture.EmptyPicture";

/// rule: INV.SOURCE.METADATA-MUTATION-VALIDITY
#[test]
fn create_an_empty_common_picture_preserves_the_existing_png() {
    let fixture = Fixture::new("workspace-root-comments");
    let runtime = runtime();
    let can = contract(&runtime, &fixture, "main:Configuration", "object.create");
    let schema = &can["argsSchema"];
    let kind = &schema["properties"]["values"]["properties"]["kind"];
    assert!(kind["description"]
        .as_str()
        .unwrap()
        .contains("CommonPicture creates an empty picture without an image."));
    let args = json!({"values":{"kind":"CommonPicture","name":NAME}});
    assert!(jsonschema::validator_for(schema).unwrap().is_valid(&args));

    let snapshot = || unica_workspace::test_support::tree_snapshot(&fixture.source);
    let before = snapshot();
    assert!(before.contains_key(Path::new("CommonPictures/Probe/Ext/Picture.xml")));
    assert!(before.contains_key(Path::new("CommonPictures/Probe/Ext/Picture/Picture.png")));
    let create = json!({"at":"main:Configuration","ops":[{"op":"object.create","args":args}]});
    publish(&runtime, &fixture, create.clone());
    let after = snapshot();
    let descriptor = PathBuf::from("CommonPictures/EmptyPicture.xml");
    let configuration = PathBuf::from("Configuration.xml");
    assert_eq!(
        changed_paths(&before, &after),
        (
            BTreeSet::from([descriptor.clone()]),
            BTreeSet::from([configuration.clone()]),
            BTreeSet::new(),
        )
    );
    assert!(!fixture.source.join("CommonPictures/EmptyPicture").exists());
    let cfg = std::str::from_utf8(&after[&configuration]).unwrap();
    assert_eq!(
        cfg.matches("<CommonPicture>EmptyPicture</CommonPicture>")
            .count(),
        1
    );

    let source = std::str::from_utf8(&after[&descriptor]).unwrap();
    let image = roxmltree::Document::parse(source.trim_start_matches('\u{feff}')).unwrap();
    let object = image.root_element().first_element_child().unwrap();
    assert!(object.has_tag_name(("http://v8.1c.ru/8.3/MDClasses", "CommonPicture")));
    assert!(uuid::Uuid::parse_str(object.attribute("uuid").unwrap()).is_ok());
    let properties = object.first_element_child().unwrap();
    assert!(properties.has_tag_name(("http://v8.1c.ru/8.3/MDClasses", "Properties")));
    for name in ["AvailabilityForChoice", "AvailabilityForAppearance"] {
        assert_eq!(
            properties
                .children()
                .find(|node| node.has_tag_name(name))
                .and_then(|node| node.text()),
            Some("false"),
            "{name}"
        );
    }
    let card = view(&runtime, &fixture, AT);
    assert_eq!(card["kind"], "CommonPicture");
    assert_eq!(card["title"], NAME);
    assert_eq!(card["props"]["AvailabilityForChoice"], false);

    let refused = call(&runtime, &fixture, ToolIdentity::Apply, create);
    assert!(!refused.ok, "{refused:?}");
    assert_eq!(refused.diagnostics[0]["code"], "invalid_state");
    assert!(refused
        .data
        .as_ref()
        .is_none_or(|data| data.get("executionToken").is_none()));
    assert_eq!(snapshot(), after);
}

/// rule: INV.SOURCE.PROFILE-CAPABILITIES
#[test]
fn common_picture_creation_is_not_offered_in_extensions() {
    let fixture = Fixture::with_extension();
    let runtime = runtime();
    let at = "ext:Configuration";
    let can = contract(&runtime, &fixture, at, "object.create");
    let kind = &can["argsSchema"]["properties"]["values"]["properties"]["kind"];
    assert!(!kind["enum"]
        .as_array()
        .unwrap()
        .contains(&json!("CommonPicture")));
    assert!(kind.get("description").is_none());
    let before = unica_workspace::test_support::tree_snapshot(&fixture.workspace);
    let refused = call(
        &runtime,
        &fixture,
        ToolIdentity::Apply,
        json!({"at":at,"ops":[{"op":"object.create","args":{"values":{"kind":"CommonPicture","name":NAME}}}]}),
    );
    assert!(!refused.ok, "{refused:?}");
    assert_eq!(refused.at.as_deref(), Some("ops[0].args.values.kind"));
    assert_eq!(refused.diagnostics[0]["code"], "bad_value");
    assert!(refused
        .data
        .as_ref()
        .is_none_or(|data| data.get("executionToken").is_none()));
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&fixture.workspace),
        before
    );
}
