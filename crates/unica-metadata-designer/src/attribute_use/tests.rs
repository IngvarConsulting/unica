use serde_json::json;
use unica_metadata::{request::parse_element_update, MetaCollection, MetaEditOperation};

#[test]
fn header_use_productive_decoder_refuses_extension_before_planning() {
    use unica_source_contract::context::{OperationSite, RegistryContext};
    let registry = unica_operations::OperationRegistry::new(
        &[],
        &[],
        RegistryContext::new(|_, _| -> Option<OperationSite<'_>> { unreachable!() }),
    );
    let context = crate::decode::MetadataDecodeContext {
        source_set_name: "extension",
        source_kind: unica_source_contract::SourceSetKind::Extension,
        registry,
    };
    let error = crate::decode::parse_attribute_set(
        json!({"at":"extension:Catalog.Items.Attribute.Flag", "values":{"use":"ForFolder"}})
            .as_object()
            .unwrap(),
        0,
        &context,
    )
    .unwrap_err();
    assert_eq!(
        error.kind(),
        unica_source_contract::proposal::ApplyPlanErrorKind::BadValue
    );
    assert_eq!(error.path(), Some("ops[0].args.values.use"));
}

fn catalog() -> String {
    include_str!(
        "../../../../tests/fixtures/acceptance/workspace-attribute-use/src/Catalogs/Items.xml"
    )
    .into()
}
fn operation(value: &str) -> MetaEditOperation {
    MetaEditOperation::update(
        MetaCollection::Attributes,
        None,
        vec![parse_element_update(&json!({"name":"Flag", "use":value}), "values").unwrap()],
    )
    .unwrap()
}
fn apply(
    xml: &mut String,
    ops: &[MetaEditOperation],
) -> Result<crate::edit::MetaEditCounts, unica_metadata::request::MetaFailure> {
    crate::edit::apply_typed_operations_with_uuid(xml, ops, "ru", &mut || unreachable!())
}

#[test]
fn header_use_cycle_preserves_every_other_byte_and_effects_and_noop() {
    let traits = include_str!("../../../../tests/fixtures/acceptance/workspace-attribute-use/src/ChartsOfCharacteristicTypes/Traits.xml");
    for source in [catalog(), traits.into()] {
        for eol in ["\n", "\r\n"] {
            let before = source.replace("\r\n", "\n").replace('\n', eol);
            let mut xml = before.clone();
            for (old, value) in [
                ("ForItem", "ForFolder"),
                ("ForFolder", "ForFolderAndItem"),
                ("ForFolderAndItem", "ForItem"),
            ] {
                let expected = xml.replacen(
                    &format!("<Use>{old}</Use>"),
                    &format!("<Use>{value}</Use>"),
                    1,
                );
                let effects = apply(&mut xml, &[operation(value)]).unwrap().effects;
                assert_eq!(xml, expected);
                assert_eq!(effects[0].before.as_ref().unwrap()[0]["Use"], old);
                assert_eq!(effects[0].after.as_ref().unwrap()[0]["Use"], value);
                let saved = xml.clone();
                let noop = apply(&mut xml, &[operation(value)]).unwrap();
                assert_eq!(xml, saved);
                assert_eq!(noop.effects[0].before, noop.effects[0].after);
            }
            assert_eq!(xml, before);
        }
    }
}

#[test]
fn header_use_with_rename_reads_postimage_name() {
    let mut xml = catalog();
    let mut op = operation("ForFolder");
    let MetaEditOperation::Update { elements, .. } = &mut op else {
        unreachable!()
    };
    elements[0].new_name = Some("Renamed".into());
    let effects = apply(&mut xml, &[op]).unwrap().effects;
    assert_eq!(effects[0].after.as_ref().unwrap()[0]["name"], "Renamed");
    assert_eq!(effects[0].after.as_ref().unwrap()[0]["Use"], "ForFolder");
}

#[test]
fn header_use_refuses_damage_missing_and_foreign_nodes_without_partial_changes() {
    for replacement in [
        "",
        "<Use/>",
        "<Use>Future</Use>",
        "<Use>\u{a0}ForItem</Use>",
        "<Use>ForItem</Use><Use>ForItem</Use>",
        "<Use xmlns=\"urn:foreign\">ForItem</Use>",
        "<Use extra=\"1\">ForItem</Use>",
        "<Use>For<!--split-->Item</Use>",
        "<Use><Value>ForItem</Value></Use>",
    ] {
        let before = catalog().replacen("<Use>ForItem</Use>", replacement, 1);
        let mut xml = before.clone();
        assert!(
            apply(&mut xml, &[operation("ForFolder")]).is_err(),
            "{replacement}"
        );
        assert_eq!(xml, before);
    }
    let before = catalog();
    let mut xml = before.clone();
    let mut invalid = operation("ForItem");
    let MetaEditOperation::Update { elements, .. } = &mut invalid else {
        unreachable!()
    };
    elements[0].attribute_use = Some("Future".into());
    assert!(apply(&mut xml, &[operation("ForFolder"), invalid]).is_err());
    assert_eq!(xml, before);
}

#[test]
fn header_use_preserves_xml_whitespace_around_the_literal() {
    let before = catalog().replacen("<Use>ForItem</Use>", "<Use> \tForItem\n</Use>", 1);
    let mut xml = before.clone();
    apply(&mut xml, &[operation("ForFolder")]).unwrap();
    assert_eq!(xml, before.replacen(" \tForItem\n", " \tForFolder\n", 1));
}

#[test]
fn header_use_observer_retains_fact_and_does_not_invent_absence_or_repair_damage() {
    fn read(
        xml: &str,
        owner: &str,
        source: Option<unica_source_contract::SourceSetKind>,
    ) -> unica_metadata::validation_subject::MetaLocalInfo {
        let target = unica_address::source_target::MetadataAddress::parse(
            unica_address::source_target::PLATFORM_XML_8_3_27_FORMAT_2_20,
            owner,
        )
        .unwrap();
        crate::info::parse_typed_meta_local_info_for_source(
            xml.as_bytes(),
            &target,
            unica_metadata::MetaSupportStatus::Supported,
            source.unwrap(),
        )
        .unwrap()
    }
    use unica_source_contract::SourceSetKind;
    for (source, owner) in [(catalog(), "Catalog.Items"), (include_str!("../../../../tests/fixtures/acceptance/workspace-attribute-use/src/ChartsOfCharacteristicTypes/Traits.xml").into(), "ChartOfCharacteristicTypes.Traits")] {
        let source = source.replace("<Hierarchical>true</Hierarchical>", "<Hierarchical>false</Hierarchical>");
        for value in unica_metadata::ATTRIBUTE_USE_VALUES {
            let xml = source.replacen("<Use>ForItem</Use>", &format!("<Use>{value}</Use>"), 1);
            let info = read(&xml, owner, Some(SourceSetKind::Configuration));
            assert!(info.collections.attributes[0].properties.iter().any(|p| p.name == "Use" && serde_json::to_value(&p.value).unwrap()["value"] == *value));
        }
        for (replacement, damaged) in [("", false), ("<Use>Future</Use>", true), ("<Use>ForItem</Use><Use>ForFolder</Use>", true)] {
            let xml = source.replacen("<Use>ForItem</Use>", replacement, 1);
            let info = read(&xml, owner, Some(SourceSetKind::Configuration));
            assert_eq!(info.collections.attributes.len(), 2);
            let a = &info.collections.attributes[0];
            assert!(!a.properties.iter().any(|p| p.name == "Use"));
            assert_eq!(!a.property_limits.is_empty(), damaged);
            assert!(info.collections.attributes[1].properties.iter().any(|p| p.name == "Use"));
        }
    }
}
