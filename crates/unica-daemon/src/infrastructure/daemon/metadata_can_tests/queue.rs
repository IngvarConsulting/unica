use super::*;

const EXPORT: &str = include_str!(
    "../../../../../../tests/fixtures/formats/platform_xml/2.22/all_kinds/src/Queues/Probe.xml"
);

/// A 2.22 configuration, of the 8.5.4 line, with one language and the
/// `registered` root entries.
fn line_8_5_4_fixture(registered: &str) -> Fixture {
    let fixture = Fixture::blank();
    for relative in ["Configuration.xml", "Languages/Русский.xml"] {
        let path = fixture.source.join(relative);
        let text = std::fs::read_to_string(&path).unwrap();
        let text = text
            .replace("version=\"2.20\"", "version=\"2.22\"")
            .replace(
                "</ChildObjects>",
                &if relative == "Configuration.xml" {
                    format!("{registered}</ChildObjects>")
                } else {
                    "</ChildObjects>".to_string()
                },
            );
        std::fs::write(path, text).unwrap();
    }
    fixture
}

/// A 2.22 configuration with one language and the queue of the platform
/// export.
fn queue_fixture() -> Fixture {
    let fixture = line_8_5_4_fixture("<Queue>Probe</Queue>");
    std::fs::create_dir_all(fixture.source.join("Queues")).unwrap();
    std::fs::write(fixture.source.join("Queues/Probe.xml"), EXPORT).unwrap();
    fixture
}

fn set(runtime: &V5CanonicalInvocationRuntime, fixture: &Fixture, values: Value) {
    publish(
        runtime,
        fixture,
        json!({"at":"main:Queue.Probe","ops":[{"op":"props.set","args":{"values":values}}]}),
    );
}

/// A queue offers its processing properties and Comment: new values change
/// only their own texts, and setting the values of the platform export back
/// gives the export byte for byte. Its dimensions and attributes are read.
/// rule: INV.SOURCE.WRITABLE-PROFILE
#[test]
fn a_queue_is_written_and_back_to_the_platform_export() {
    let fixture = queue_fixture();
    let runtime = runtime();
    let can = contract(&runtime, &fixture, "main:Queue.Probe", "props.set");
    let values = can["argsSchema"]["properties"]["values"]["properties"]
        .as_object()
        .unwrap();
    assert_eq!(
        values.keys().map(String::as_str).collect::<BTreeSet<_>>(),
        BTreeSet::from([
            "AutoProcessing",
            "Comment",
            "ProcessingPortionSize",
            "Transactioned",
            "comment",
        ])
    );

    let node = view(&runtime, &fixture, "main:Queue.Probe");
    assert_eq!(node["props"]["Synonym"], "ru = 'Проба'");
    assert_eq!(node["props"]["Transactioned"], true);
    assert_eq!(node["props"]["ProcessingPortionSize"], 1000);
    assert_eq!(node["props"]["AutoProcessing"], true);
    let branches = node["branches"].as_array().unwrap();
    for (at, count) in [
        ("main:Queue.Probe.Dimension", 1),
        ("main:Queue.Probe.Attribute", 1),
    ] {
        let branch = branches
            .iter()
            .find(|branch| branch["at"] == at)
            .unwrap_or_else(|| panic!("{at} is a branch of the queue: {node}"));
        assert_eq!(branch["count"], count, "{at}");
    }
    assert_eq!(
        view(&runtime, &fixture, "main:Queue.Probe.Dimension.Измерение")["props"]["synonym"],
        "ru = 'Измерение'"
    );

    let path = fixture.source.join("Queues/Probe.xml");
    set(
        &runtime,
        &fixture,
        json!({"Transactioned":false,"ProcessingPortionSize":500,"AutoProcessing":false,"Comment":"changed"}),
    );
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        EXPORT
            .replace("<Transactioned>true</", "<Transactioned>false</")
            .replace(
                "<ProcessingPortionSize>1000</",
                "<ProcessingPortionSize>500</"
            )
            .replace("<AutoProcessing>true</", "<AutoProcessing>false</")
            .replace("<Comment>all_kinds</Comment>", "<Comment>changed</Comment>")
    );
    let props = view(&runtime, &fixture, "main:Queue.Probe")["props"].clone();
    assert_eq!(props["Transactioned"], false);
    assert_eq!(props["ProcessingPortionSize"], 500);
    assert_eq!(props["AutoProcessing"], false);
    assert_eq!(props["Comment"], "changed");

    set(
        &runtime,
        &fixture,
        json!({"Transactioned":true,"ProcessingPortionSize":1000,"AutoProcessing":true,"comment":"all_kinds"}),
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), EXPORT);

    let before = unica_workspace::test_support::tree_snapshot(&fixture.workspace);
    for values in [
        json!({"ProcessingPortionSize":0}),
        json!({"ProcessingPortionSize":-1}),
        json!({"ProcessingPortionSize":"1000"}),
        json!({"Transactioned":"false"}),
        json!({"Name":"Other"}),
        json!({}),
    ] {
        let refused = call(
            &runtime,
            &fixture,
            ToolIdentity::Apply,
            json!({"at":"main:Queue.Probe","ops":[{"op":"props.set","args":{"values":values}}]}),
        );
        assert!(!refused.ok, "{values}: {refused:?}");
    }
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&fixture.workspace),
        before
    );
}

/// `object.create` writes a queue the 8.5.4 platform imports: its message and
/// manager generated types, the values the platform gives omitted
/// properties, and its registration in the configuration.
/// rule: INV.SOURCE.PROFILE-CAPABILITIES
#[test]
fn a_queue_is_created_with_its_generated_types_on_the_8_5_4_line() {
    let fixture = line_8_5_4_fixture("");
    let runtime = runtime();
    let can = contract(&runtime, &fixture, "main:Configuration", "object.create");
    let kinds = can["argsSchema"]["properties"]["values"]["properties"]["kind"]["enum"]
        .as_array()
        .unwrap();
    assert!(kinds.contains(&json!("Queue")), "{kinds:?}");

    publish(
        &runtime,
        &fixture,
        json!({"at":"main:Configuration","ops":[{"op":"object.create","args":{"values":{"kind":"Queue","name":"Inbox"}}}]}),
    );
    let descriptor = std::fs::read_to_string(fixture.source.join("Queues/Inbox.xml")).unwrap();
    let document = roxmltree::Document::parse(descriptor.trim_start_matches('\u{feff}')).unwrap();
    assert_eq!(document.root_element().attribute("version"), Some("2.22"));
    let queue = document
        .root_element()
        .children()
        .find(|node| node.is_element())
        .unwrap();
    assert_eq!(queue.tag_name().name(), "Queue");
    let generated = queue
        .descendants()
        .filter(|node| node.tag_name().name() == "GeneratedType")
        .map(|node| {
            (
                node.attribute("name").unwrap(),
                node.attribute("category").unwrap(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        generated,
        [
            ("QueueMessage.Inbox", "Record"),
            ("QueueManager.Inbox", "Manager")
        ]
    );
    let properties = queue
        .children()
        .find(|node| node.tag_name().name() == "Properties")
        .unwrap()
        .children()
        .filter(|node| node.is_element())
        .map(|node| node.tag_name().name())
        .collect::<Vec<_>>();
    assert_eq!(
        properties,
        [
            "Name",
            "Synonym",
            "Comment",
            "Transactioned",
            "ProcessingPortionSize",
            "AutoProcessing"
        ]
    );
    assert!(queue
        .children()
        .any(|node| node.tag_name().name() == "ChildObjects"));
    assert!(
        std::fs::read_to_string(fixture.source.join("Configuration.xml"))
            .unwrap()
            .contains("<Queue>Inbox</Queue>")
    );

    let node = view(&runtime, &fixture, "main:Queue.Inbox");
    assert_eq!(node["props"]["Transactioned"], true);
    assert_eq!(node["props"]["ProcessingPortionSize"], 1000);
    assert_eq!(node["props"]["AutoProcessing"], true);
}

fn refusal(runtime: &V5CanonicalInvocationRuntime, fixture: &Fixture, args: Value) -> Value {
    let before = unica_workspace::test_support::tree_snapshot(&fixture.workspace);
    let refused = call(runtime, fixture, ToolIdentity::Apply, args);
    assert!(!refused.ok, "{refused:?}");
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&fixture.workspace),
        before
    );
    refused.diagnostics[0].clone()
}

/// On the 8.3.27 line a kind of a later line is refused as one the platform
/// does not have, whichever operation names it; a kind Unica reads but does
/// not create, and a kind another operation creates, are each refused with
/// their own reason.
/// rule: INV.SOURCE.PROFILE-CAPABILITIES
#[test]
fn creation_refusals_tell_a_missing_kind_from_one_unica_does_not_create() {
    let fixture = Fixture::blank();
    let runtime = runtime();
    let create = |kind: &str| {
        refusal(
            &runtime,
            &fixture,
            json!({"at":"main:Configuration","ops":[{"op":"object.create","args":{"values":{"kind":kind,"name":"Probe"}}}]}),
        )
    };
    for (kind, message) in [
        (
            "Queue",
            "`Queue` is a metadata kind of 1C 8.5.4 and later; this source set is 1C 8.3.27, where the kind does not exist. Only a set re-exported from 8.5.4 or later can hold a queue.",
        ),
        (
            "PaletteColor",
            "`PaletteColor` is a metadata kind of 1C 8.5.1 and later; this source set is 1C 8.3.27, where the kind does not exist. Only a set re-exported from 8.5.1 or later can hold a palette color.",
        ),
        (
            "Bot",
            "Unica reads and edits metadata kind `Bot` but cannot create it yet; create the object in Designer and export the source set again",
        ),
        (
            "Role",
            "metadata kind `Role` is created by the `role.create` operation",
        ),
    ] {
        let refused = create(kind);
        assert_eq!(refused["code"], "bad_value", "{kind}: {refused}");
        assert_eq!(refused["message"], message, "{kind}: {refused}");
    }

    let refused = refusal(
        &runtime,
        &fixture,
        json!({"at":"main:Queue.Probe","ops":[{"op":"props.set","args":{"values":{"Comment":"x"}}}]}),
    );
    assert_eq!(refused["code"], "bad_value", "{refused}");
    assert_eq!(
        refused["message"],
        "`Queue` is a metadata kind of 1C 8.5.4 and later; this source set is 1C 8.3.27, where the kind does not exist. Only a set re-exported from 8.5.4 or later can hold a queue.",
    );
}

/// Borrowing a kind the parent's line does not have names both lines, not
/// an unregistered object.
/// rule: INV.SOURCE.PROFILE-CAPABILITIES
#[test]
fn borrowing_a_kind_of_a_later_line_names_the_parent_line() {
    let fixture = Fixture::with_extension();
    let runtime = runtime();
    let refused = refusal(
        &runtime,
        &fixture,
        json!({"at":"ext:Configuration","ops":[{"op":"object.borrow","args":{"from":"main:Queue.Probe"}}]}),
    );
    assert_eq!(refused["code"], "bad_value", "{refused}");
    assert_eq!(
        refused["message"],
        "`Queue` is a metadata kind of 1C 8.5.4 and later; this source set is 1C 8.3.27, where the kind does not exist. Only a set re-exported from 8.5.4 or later can hold a queue.",
    );
}
