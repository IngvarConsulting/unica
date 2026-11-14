use super::*;

const EXPORT: &str = include_str!(
    "../../../../../../tests/fixtures/formats/platform_xml/2.22/all_kinds/src/PaletteColors/Probe.xml"
);

/// A 2.22 configuration with one language and the palette colour of the
/// platform export.
fn palette_fixture() -> Fixture {
    let fixture = Fixture::blank();
    for relative in ["Configuration.xml", "Languages/Русский.xml"] {
        let path = fixture.source.join(relative);
        let text = std::fs::read_to_string(&path).unwrap();
        let text = text
            .replace("version=\"2.20\"", "version=\"2.22\"")
            .replace(
                "</ChildObjects>",
                if relative == "Configuration.xml" {
                    "<PaletteColor>Probe</PaletteColor></ChildObjects>"
                } else {
                    "</ChildObjects>"
                },
            );
        std::fs::write(path, text).unwrap();
    }
    std::fs::create_dir_all(fixture.source.join("PaletteColors")).unwrap();
    std::fs::write(fixture.source.join("PaletteColors/Probe.xml"), EXPORT).unwrap();
    fixture
}

fn set(runtime: &V5CanonicalInvocationRuntime, fixture: &Fixture, values: Value) {
    publish(
        runtime,
        fixture,
        json!({"at":"main:PaletteColor.Probe","ops":[{"op":"props.set","args":{"values":values}}]}),
    );
}

/// A palette colour offers its Color and Comment: a new colour changes only
/// the Color text, and setting the colour and comment of the platform export
/// back gives the export byte for byte.
/// rule: INV.SOURCE.WRITABLE-PROFILE
#[test]
fn a_palette_color_is_written_and_back_to_the_platform_export() {
    let fixture = palette_fixture();
    let runtime = runtime();
    let can = contract(&runtime, &fixture, "main:PaletteColor.Probe", "props.set");
    let values = can["argsSchema"]["properties"]["values"]["properties"]
        .as_object()
        .unwrap();
    assert_eq!(
        values.keys().map(String::as_str).collect::<BTreeSet<_>>(),
        BTreeSet::from(["Color", "Comment", "comment"])
    );

    let path = fixture.source.join("PaletteColors/Probe.xml");
    set(
        &runtime,
        &fixture,
        json!({"Color":"#FF0000","Comment":"changed"}),
    );
    let written = std::fs::read_to_string(&path).unwrap();
    assert_eq!(
        written,
        EXPORT
            .replace("<Color>#1A2B3C</Color>", "<Color>#FF0000</Color>")
            .replace("<Comment>all_kinds</Comment>", "<Comment>changed</Comment>")
    );
    let props = view(&runtime, &fixture, "main:PaletteColor.Probe")["props"].clone();
    assert_eq!(props["Color"], "#FF0000");
    assert_eq!(props["Comment"], "changed");

    set(
        &runtime,
        &fixture,
        json!({"Color":"#1A2B3C","comment":"all_kinds"}),
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), EXPORT);

    let before = unica_workspace::test_support::tree_snapshot(&fixture.workspace);
    for color in ["red", "#12345", ""] {
        let refused = call(
            &runtime,
            &fixture,
            ToolIdentity::Apply,
            json!({"at":"main:PaletteColor.Probe","ops":[{"op":"props.set","args":{"values":{"Color":color}}}]}),
        );
        assert!(!refused.ok, "{color}: {refused:?}");
    }
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&fixture.workspace),
        before
    );
}
