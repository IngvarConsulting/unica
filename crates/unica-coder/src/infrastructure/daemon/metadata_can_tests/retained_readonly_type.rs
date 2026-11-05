use super::*;
use unica_workspace::test_support::tree_snapshot;

const AT: &str = "main:Catalog.Values.Attribute.Value";
const DESCRIPTOR: &str = "Catalogs/Values.xml";

fn property_image(bytes: &[u8], name: &str) -> Vec<u8> {
    let text = std::str::from_utf8(bytes).unwrap();
    let body = text.trim_start_matches('\u{feff}');
    let xml = roxmltree::Document::parse(body).unwrap();
    let attribute = xml
        .descendants()
        .find(|node| {
            node.has_tag_name("Attribute")
                && node.children().any(|props| {
                    props.has_tag_name("Properties")
                        && props.children().any(|property| {
                            property.has_tag_name("Name") && property.text() == Some("Value")
                        })
                })
        })
        .unwrap();
    let property = attribute
        .children()
        .find(|node| node.has_tag_name("Properties"))
        .unwrap()
        .children()
        .find(|node| node.has_tag_name(name))
        .unwrap();
    let offset = text.len() - body.len();
    bytes[offset + property.range().start..offset + property.range().end].to_vec()
}

fn attribute_image(bytes: &[u8], name: &str) -> Vec<u8> {
    let text = std::str::from_utf8(bytes).unwrap();
    let body = text.trim_start_matches('\u{feff}');
    let xml = roxmltree::Document::parse(body).unwrap();
    let node = xml
        .descendants()
        .find(|node| {
            node.has_tag_name("Attribute")
                && node.children().any(|props| {
                    props.has_tag_name("Properties")
                        && props.children().any(|property| {
                            property.has_tag_name("Name") && property.text() == Some(name)
                        })
                })
        })
        .unwrap();
    let offset = text.len() - body.len();
    bytes[offset + node.range().start..offset + node.range().end].to_vec()
}

fn assert_read_only(runtime: &V5CanonicalInvocationRuntime, fixture: &Fixture) {
    let viewed = call(runtime, fixture, ToolIdentity::View, json!({"at":AT}));
    assert!(viewed.ok, "{viewed:?}");
    assert_eq!(
        viewed.data.as_ref().unwrap()["props"]["type"],
        json!({
            "variants":[{"kind":"characteristicTypes", "metadataPath":"ChartOfCharacteristicTypes.Properties"}],
            "mutationCapability":"readOnly"
        })
    );
}

#[test]
fn canonical_independent_attribute_edits_preserve_read_only_type_bytes() {
    for (property, xml_name) in [("comment", "Comment"), ("synonym", "Synonym")] {
        let fixture = Fixture::new("workspace-metadata-values");
        let runtime = runtime();
        assert_read_only(&runtime, &fixture);
        let before = tree_snapshot(&fixture.source);
        let descriptor_before = &before[Path::new(DESCRIPTOR)];
        let type_before = property_image(descriptor_before, "Type");
        assert_eq!(
            type_before,
            b"<Type><v8:TypeSet>cfg:Characteristic.Properties</v8:TypeSet></Type>"
        );
        let args = json!({"at":AT,"ops":[{"op":"attribute.set","args":{"values":{(property):"Independent property"}}}]});
        publish(&runtime, &fixture, args.clone());
        let after = tree_snapshot(&fixture.source);
        let descriptor_after = &after[Path::new(DESCRIPTOR)];
        assert_eq!(property_image(descriptor_after, "Type"), type_before);
        for neighbor in ["EmptyItem", "Flag"] {
            assert_eq!(
                attribute_image(descriptor_after, neighbor),
                attribute_image(descriptor_before, neighbor)
            );
        }
        let viewed = call(&runtime, &fixture, ToolIdentity::View, json!({"at":AT}));
        assert!(viewed.ok, "{viewed:?}");
        assert_eq!(
            viewed.data.as_ref().unwrap()["props"][property],
            "Independent property"
        );
        let changed_property = property_image(descriptor_after, xml_name);
        assert!(std::str::from_utf8(&changed_property)
            .unwrap()
            .contains("Independent property"));
        assert!(std::str::from_utf8(descriptor_after)
            .unwrap()
            .contains("uuid=\"c38560bd-f077-54d3-bfaa-7db00a8c6acd\""));
        for (path, bytes) in &before {
            if path != Path::new(DESCRIPTOR) {
                assert_eq!(&after[path], bytes, "independent edit changed {path:?}");
            }
        }
        assert_eq!(after.len(), before.len());
        assert_read_only(&runtime, &fixture);
        let planned_again = call(&runtime, &fixture, ToolIdentity::Apply, args);
        assert!(planned_again.ok, "{planned_again:?}");
        assert_eq!(tree_snapshot(&fixture.source), after);
    }
}

#[test]
fn canonical_fill_value_refuses_read_only_type_without_source_changes() {
    let fixture = Fixture::new("workspace-metadata-values");
    let runtime = runtime();
    assert_read_only(&runtime, &fixture);
    let before = tree_snapshot(&fixture.source);
    let refused = call(
        &runtime,
        &fixture,
        ToolIdentity::Apply,
        json!({
            "at":AT,"ops":[{"op":"attribute.set","args":{"values":{"fillValue":{"kind":"boolean","value":true}}}}]
        }),
    );
    assert!(!refused.ok, "{refused:?}");
    assert!(refused.data.is_none());
    assert_eq!(refused.at.as_deref(), Some("ops[0].args.values.type"));
    assert_eq!(refused.diagnostics[0]["outcome"], "fixCall");
    assert!(
        refused
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic["code"] == "bad_value"),
        "{refused:?}"
    );
    assert!(
        format!("{:?}", refused.diagnostics)
            .contains("existing metadata type is outside the public mutation algebra"),
        "{refused:?}"
    );
    assert_eq!(tree_snapshot(&fixture.source), before);
}
