use super::*;

const NAME: &str = "EmptyBinaryProbe";
const AT: &str = "main:CommonTemplate.EmptyBinaryProbe";
const REGISTRATION: &str = "<CommonTemplate>EmptyBinaryProbe</CommonTemplate>";

/// rule: INV.SOURCE.METADATA-MUTATION-VALIDITY
#[test]
fn create_an_empty_common_binary_template_and_remove_it() {
    let fixture = Fixture::blank();
    let runtime = runtime();
    let args = json!({"values":{"kind":"CommonTemplate","name":NAME}});
    let create = json!({"at":"main:Configuration","ops":[{"op":"object.create","args":args}]});
    let can = contract(&runtime, &fixture, "main:Configuration", "object.create");
    assert!(jsonschema::validator_for(&can["argsSchema"])
        .unwrap()
        .is_valid(&args));
    assert_eq!(
        can["argsSchema"]["properties"]["values"]["properties"]["kind"]["description"],
        "CommonTemplate creates an empty BinaryData template without a content file."
    );

    let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
    publish(&runtime, &fixture, create.clone());
    let created = unica_workspace::test_support::tree_snapshot(&fixture.source);
    let descriptor = PathBuf::from("CommonTemplates").join(format!("{NAME}.xml"));
    let configuration = PathBuf::from("Configuration.xml");
    assert_eq!(
        changed_paths(&before, &created),
        (
            BTreeSet::from([descriptor.clone()]),
            BTreeSet::from([configuration.clone()]),
            BTreeSet::new(),
        )
    );
    assert!(!fixture.source.join("CommonTemplates").join(NAME).exists());
    let xml = std::str::from_utf8(&created[&descriptor]).unwrap();
    let document = roxmltree::Document::parse(xml.trim_start_matches('\u{feff}')).unwrap();
    let root = document.root_element();
    let object = root.first_element_child().unwrap();
    assert_eq!(root.attribute("version"), Some("2.20"));
    assert!(object.has_tag_name(("http://v8.1c.ru/8.3/MDClasses", "CommonTemplate")));
    assert!(uuid::Uuid::parse_str(object.attribute("uuid").unwrap()).is_ok());
    assert_eq!(
        object
            .descendants()
            .find(|node| node.has_tag_name("TemplateType"))
            .and_then(|node| node.text()),
        Some("BinaryData")
    );
    assert_eq!(
        std::str::from_utf8(&created[&configuration])
            .unwrap()
            .matches(REGISTRATION)
            .count(),
        1
    );

    let node = view(&runtime, &fixture, AT);
    assert_eq!(node["kind"], "CommonTemplate", "{node}");
    assert_eq!(node["props"]["format"], "BinaryData", "{node}");
    let (status, diagnostics) = check(&runtime, &fixture, AT);
    assert!(
        diagnostics
            .iter()
            .all(|diagnostic| diagnostic["severity"] != "error"),
        "{status}: {diagnostics:?}"
    );
    let duplicate = call(&runtime, &fixture, ToolIdentity::Apply, create);
    assert!(!duplicate.ok, "{duplicate:?}");
    assert_eq!(duplicate.diagnostics[0]["code"], "invalid_state");
    assert!(duplicate
        .data
        .as_ref()
        .is_none_or(|data| data.get("executionToken").is_none()));
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&fixture.source),
        created
    );

    contract(&runtime, &fixture, AT, "object.remove");
    publish(
        &runtime,
        &fixture,
        json!({"at":AT,"ops":[{"op":"object.remove"}]}),
    );
    let removed = unica_workspace::test_support::tree_snapshot(&fixture.source);
    assert_eq!(
        changed_paths(&created, &removed),
        (
            BTreeSet::new(),
            BTreeSet::from([configuration.clone()]),
            BTreeSet::from([descriptor])
        )
    );
    assert!(!std::str::from_utf8(&removed[&configuration])
        .unwrap()
        .contains(REGISTRATION));
}

/// rule: INV.SOURCE.PROFILE-CAPABILITIES
#[test]
fn common_template_creation_offers_only_the_main_configuration_and_fixed_type() {
    let fixture = Fixture::with_extension();
    let runtime = runtime();
    let create =
        |at, values| json!({"at":at,"ops":[{"op":"object.create","args":{"values":values}}]});
    let ext = "ext:Configuration";
    let kinds = &contract(&runtime, &fixture, ext, "object.create")["argsSchema"]["properties"]
        ["values"]["properties"]["kind"]["enum"];
    assert!(!kinds.as_array().unwrap().contains(&json!("CommonTemplate")));
    let before = unica_workspace::test_support::tree_snapshot(&fixture.workspace);
    let refused = call(
        &runtime,
        &fixture,
        ToolIdentity::Apply,
        create(ext, json!({"kind":"CommonTemplate","name":NAME})),
    );
    assert!(!refused.ok, "{refused:?}");
    assert_eq!(refused.at.as_deref(), Some("ops[0].args.values.kind"));
    assert_eq!(refused.diagnostics[0]["code"], "bad_value");

    let refused = call(
        &runtime,
        &fixture,
        ToolIdentity::Apply,
        create(
            "main:Configuration",
            json!({"kind":"CommonTemplate","name":NAME,"templateType":"SpreadsheetDocument"}),
        ),
    );
    assert!(!refused.ok, "{refused:?}");
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
