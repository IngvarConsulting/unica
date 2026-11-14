use super::*;

const EXPORT: &str = include_str!(
    "../../../../../../tests/fixtures/formats/platform_xml/2.22/all_kinds/src/Queues/Probe.xml"
);

/// A 2.22 configuration with one language and the queue of the platform
/// export.
fn queue_fixture() -> Fixture {
    let fixture = Fixture::blank();
    for relative in ["Configuration.xml", "Languages/Русский.xml"] {
        let path = fixture.source.join(relative);
        let text = std::fs::read_to_string(&path).unwrap();
        let text = text
            .replace("version=\"2.20\"", "version=\"2.22\"")
            .replace(
                "</ChildObjects>",
                if relative == "Configuration.xml" {
                    "<Queue>Probe</Queue></ChildObjects>"
                } else {
                    "</ChildObjects>"
                },
            );
        std::fs::write(path, text).unwrap();
    }
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
    assert_eq!(node["props"]["Synonym"], "Проба");
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
        "Измерение"
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
