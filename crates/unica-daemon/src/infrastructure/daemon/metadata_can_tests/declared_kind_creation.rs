use super::*;

/// The declared kinds outside the typed set that `object.create` writes on
/// every described line, with the properties their template adds after
/// `Name`, `Synonym` and `Comment` and whether the descriptor holds generated
/// types and `ChildObjects`.
const KINDS: &[(&str, &str, &[&str], bool, bool)] = &[
    (
        "SessionParameter",
        "SessionParameters",
        &["Type"],
        false,
        false,
    ),
    ("StyleItem", "StyleItems", &["Type", "Value"], false, false),
    (
        "SettingsStorage",
        "SettingsStorages",
        &[
            "DefaultSaveForm",
            "DefaultLoadForm",
            "AuxiliarySaveForm",
            "AuxiliaryLoadForm",
        ],
        true,
        true,
    ),
    ("WSReference", "WSReferences", &["LocationURL"], true, false),
    (
        "DocumentNumerator",
        "DocumentNumerators",
        &[
            "NumberType",
            "NumberLength",
            "NumberAllowedLength",
            "NumberPeriodicity",
            "CheckUnique",
        ],
        false,
        false,
    ),
];

fn element<'a, 'input>(
    node: roxmltree::Node<'a, 'input>,
    name: &str,
) -> Option<roxmltree::Node<'a, 'input>> {
    node.children()
        .find(|child| child.is_element() && child.tag_name().name() == name)
}

/// Each kind is offered by `can`, written with the values the platform export
/// gives an object whose descriptor omits them, registered in the
/// configuration and read back by `view`.
/// rule: INV.SOURCE.PROFILE-CAPABILITIES
#[test]
fn declared_kinds_are_created_with_the_platform_values() {
    let fixture = Fixture::blank();
    let runtime = runtime();
    let can = contract(&runtime, &fixture, "main:Configuration", "object.create");
    let offered = can["argsSchema"]["properties"]["values"]["properties"]["kind"]["enum"]
        .as_array()
        .unwrap()
        .clone();
    for (kind, directory, properties, generated, child_objects) in KINDS {
        assert!(offered.contains(&json!(kind)), "{kind}: {offered:?}");
        publish(
            &runtime,
            &fixture,
            json!({"at":"main:Configuration","ops":[{"op":"object.create","args":{"values":{"kind":kind,"name":"Created"}}}]}),
        );
        let descriptor =
            std::fs::read_to_string(fixture.source.join(format!("{directory}/Created.xml")))
                .unwrap();
        let document =
            roxmltree::Document::parse(descriptor.trim_start_matches('\u{feff}')).unwrap();
        assert_eq!(document.root_element().attribute("version"), Some("2.20"));
        let object = document
            .root_element()
            .children()
            .find(|node| node.is_element())
            .unwrap();
        assert_eq!(object.tag_name().name(), *kind);
        let written = element(object, "Properties")
            .unwrap()
            .children()
            .filter(|node| node.is_element())
            .map(|node| node.tag_name().name())
            .collect::<Vec<_>>();
        let expected = ["Name", "Synonym", "Comment"]
            .iter()
            .chain(properties.iter())
            .copied()
            .collect::<Vec<_>>();
        assert_eq!(written, expected, "{kind}");
        assert_eq!(
            element(object, "InternalInfo").is_some(),
            *generated,
            "{kind}"
        );
        assert_eq!(
            element(object, "ChildObjects").is_some(),
            *child_objects,
            "{kind}"
        );
        assert!(
            std::fs::read_to_string(fixture.source.join("Configuration.xml"))
                .unwrap()
                .contains(&format!("<{kind}>Created</{kind}>")),
            "{kind}"
        );
        let node = view(&runtime, &fixture, &format!("main:{kind}.Created"));
        assert_eq!(
            (&node["title"], &node["kind"]),
            (&json!("Created"), &json!(kind)),
            "{node}"
        );
    }
}

/// A common attribute carries the whole property set the platform writes: its
/// type, input, separation and indexing defaults.
/// rule: INV.SOURCE.PROFILE-CAPABILITIES
#[test]
fn a_common_attribute_is_created_with_the_platform_values() {
    let fixture = Fixture::blank();
    let runtime = runtime();
    publish(
        &runtime,
        &fixture,
        json!({"at":"main:Configuration","ops":[{"op":"object.create","args":{"values":{"kind":"CommonAttribute","name":"Created"}}}]}),
    );
    let descriptor =
        std::fs::read_to_string(fixture.source.join("CommonAttributes/Created.xml")).unwrap();
    for property in [
        "<AutoUse>DontUse</AutoUse>",
        "<DataSeparation>DontUse</DataSeparation>",
        "<Indexing>DontIndex</Indexing>",
        "<FullTextSearch>Use</FullTextSearch>",
        "<DataHistory>Use</DataHistory>",
        "<Content/>",
    ] {
        assert!(descriptor.contains(property), "{property}\n{descriptor}");
    }
    assert!(
        std::fs::read_to_string(fixture.source.join("Configuration.xml"))
            .unwrap()
            .contains("<CommonAttribute>Created</CommonAttribute>")
    );
    let node = view(&runtime, &fixture, "main:CommonAttribute.Created");
    assert_eq!(node["title"], "Created", "{node}");
    assert_eq!(node["props"]["Indexing"], "DontIndex", "{node}");
}
