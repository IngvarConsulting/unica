use super::*;

const EXPORT: &str = "../../tests/fixtures/formats/platform_xml/2.20/declared_creation";

/// The declared kinds outside the typed set that `object.create` writes on
/// every described line, with the directory and name of their platform
/// export.
const KINDS: &[(&str, &str, &str)] = &[
    ("SessionParameter", "SessionParameters", "SessionProbe"),
    ("CommonAttribute", "CommonAttributes", "Probe"),
    ("StyleItem", "StyleItems", "Probe"),
    ("SettingsStorage", "SettingsStorages", "Probe"),
    ("WSReference", "WSReferences", "Probe"),
    ("DocumentNumerator", "DocumentNumerators", "Sales"),
];

/// The descriptor with every UUID replaced: the object, type and value
/// identifiers are fresh in each creation.
fn without_uuids(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(|ch: char| ch.is_ascii_hexdigit()) {
        let candidate = &rest[at..];
        if candidate.len() >= 36
            && candidate.is_char_boundary(36)
            && uuid::Uuid::parse_str(&candidate[..36]).is_ok()
        {
            out.push_str(&rest[..at]);
            out.push_str("<uuid>");
            rest = &candidate[36..];
        } else {
            out.push_str(&rest[..=at]);
            rest = &rest[at + 1..];
        }
    }
    out.push_str(rest);
    out
}

/// Each kind is offered by `can`, written byte for byte as the platform
/// exports an object the template wrote, registered in the configuration and
/// read back by `view`.
/// rule: INV.SOURCE.PROFILE-CAPABILITIES
#[test]
fn declared_kinds_are_created_as_the_platform_exports_them() {
    let fixture = Fixture::blank();
    let runtime = runtime();
    let can = contract(&runtime, &fixture, "main:Configuration", "object.create");
    let offered = can["argsSchema"]["properties"]["values"]["properties"]["kind"]["enum"]
        .as_array()
        .unwrap()
        .clone();
    let export = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(EXPORT);
    for (kind, directory, name) in KINDS {
        assert!(offered.contains(&json!(kind)), "{kind}: {offered:?}");
        publish(
            &runtime,
            &fixture,
            json!({"at":"main:Configuration","ops":[{"op":"object.create","args":{"values":{"kind":kind,"name":name}}}]}),
        );
        let relative = format!("{directory}/{name}.xml");
        let written = std::fs::read_to_string(fixture.source.join(&relative)).unwrap();
        let platform = std::fs::read_to_string(export.join(&relative)).unwrap();
        assert_eq!(without_uuids(&written), without_uuids(&platform), "{kind}");
        assert!(
            std::fs::read_to_string(fixture.source.join("Configuration.xml"))
                .unwrap()
                .contains(&format!("<{kind}>{name}</{kind}>")),
            "{kind}"
        );
        let node = view(&runtime, &fixture, &format!("main:{kind}.{name}"));
        assert_eq!(
            (&node["title"], &node["kind"]),
            (&json!(name), &json!(kind)),
            "{node}"
        );
    }
}

#[test]
fn uuids_are_masked_and_the_rest_is_kept() {
    assert_eq!(
        without_uuids("<a uuid=\"1d66d9f6-0206-5440-a2f2-9a91db9621b7\">ab-1 #000000 Синоним</a>"),
        "<a uuid=\"<uuid>\">ab-1 #000000 Синоним</a>"
    );
}
