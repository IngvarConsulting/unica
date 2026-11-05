use super::*;

fn xml(kind: MetadataKind, properties: &str) -> String {
    format!(
        "\u{feff}<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.20\">\r\n\t<{} uuid=\"12345678-1234-1234-1234-123456789012\">\r\n\t\t<Properties>\r\n\t\t\t<Name>Example</Name>\r\n\t\t\t<DescriptionLength>50</DescriptionLength>\r\n\t\t\t{properties}\r\n\t\t</Properties>\r\n\t\t<ChildObjects/>\r\n\t</{}>\r\n</MetaDataObject>\r\n",
        kind.as_str(), kind.as_str()
    )
}

fn apply(
    xml: &mut String,
    kind: MetadataKind,
    name: &str,
    value: MetaPropertyValue,
) -> Result<MetaEditCounts, unica_metadata::request::MetaFailure> {
    let changes =
        MetaPropertyChanges::convert(kind, vec![MetaPropertyInput::new(name, value)]).unwrap();
    apply_typed_operations_with_uuid(
        xml,
        &[MetaEditOperation::SetProperties { values: changes }],
        "ru",
        &mut || panic!("scalar edit must not allocate UUIDs"),
    )
}

#[test]
fn presentation_round_trips_preserve_bytes_and_noop_effects() {
    for kind in [
        MetadataKind::Catalog,
        MetadataKind::ChartOfAccounts,
        MetadataKind::ChartOfCharacteristicTypes,
        MetadataKind::ChartOfCalculationTypes,
        MetadataKind::ExchangePlan,
        MetadataKind::Task,
    ] {
        let original = xml(
            kind,
            "<DefaultPresentation>AsDescription</DefaultPresentation>",
        );
        let alternate = if kind == MetadataKind::Task {
            "AsNumber"
        } else {
            "AsCode"
        };
        let mut actual = original.clone();
        let counts = apply(
            &mut actual,
            kind,
            "DefaultPresentation",
            MetaPropertyValue::String(alternate.into()),
        )
        .unwrap();
        assert_eq!(counts.modified, 1, "{kind:?}");
        assert_eq!(
            actual,
            original.replace(">AsDescription<", &format!(">{alternate}<"))
        );
        let changed = actual.clone();
        let repeated = apply(
            &mut actual,
            kind,
            "DefaultPresentation",
            MetaPropertyValue::String(alternate.into()),
        )
        .unwrap();
        assert_eq!(repeated.effects[0].before, repeated.effects[0].after);
        assert_eq!(actual, changed);
        apply(
            &mut actual,
            kind,
            "DefaultPresentation",
            MetaPropertyValue::String("AsDescription".into()),
        )
        .unwrap();
        assert_eq!(actual, original);
    }
    for hierarchical in [true, false] {
        let original = xml(MetadataKind::ChartOfCharacteristicTypes, &format!("<Hierarchical>{hierarchical}</Hierarchical><FoldersOnTop>true</FoldersOnTop><DefaultPresentation>AsDescription</DefaultPresentation>"));
        let mut actual = original.clone();
        apply(
            &mut actual,
            MetadataKind::ChartOfCharacteristicTypes,
            "FoldersOnTop",
            MetaPropertyValue::Boolean(false),
        )
        .unwrap();
        assert_eq!(
            actual,
            original.replace("<FoldersOnTop>true", "<FoldersOnTop>false")
        );
        apply(
            &mut actual,
            MetadataKind::ChartOfCharacteristicTypes,
            "FoldersOnTop",
            MetaPropertyValue::Boolean(true),
        )
        .unwrap();
        assert_eq!(actual, original);
    }
    let original = xml(
        MetadataKind::Task,
        "<DefaultPresentation>As<!-- keep -->Description</DefaultPresentation>",
    );
    let mut actual = original.clone();
    apply(
        &mut actual,
        MetadataKind::Task,
        "DefaultPresentation",
        MetaPropertyValue::String("AsDescription".into()),
    )
    .unwrap();
    assert_eq!(actual, original);
}

#[test]
fn presentation_rejects_damaged_or_missing_preimages_without_partial_edits() {
    let kind = MetadataKind::Task;
    for property in [
        "", "<DefaultPresentation/>",
        "<DefaultPresentation>AsCode</DefaultPresentation>",
        "<DefaultPresentation>Broken</DefaultPresentation>",
        "<DefaultPresentation><Value>AsDescription</Value></DefaultPresentation>",
        "<DefaultPresentation extra=\"x\">AsDescription</DefaultPresentation>",
        "<DefaultPresentation xmlns=\"urn:foreign\">AsDescription</DefaultPresentation>",
        "<DefaultPresentation>AsDescription</DefaultPresentation><DefaultPresentation>AsDescription</DefaultPresentation>",
    ] {
        let mut actual = xml(kind, property);
        let original = actual.clone();
        let operations = [
            MetaEditOperation::SetProperties { values: MetaPropertyChanges::convert(kind, vec![MetaPropertyInput::new("DescriptionLength", MetaPropertyValue::UnsignedInteger(99))]).unwrap() },
            MetaEditOperation::SetProperties { values: MetaPropertyChanges::convert(kind, vec![MetaPropertyInput::new("DefaultPresentation", MetaPropertyValue::String("AsNumber".into()))]).unwrap() },
        ];
        assert!(apply_typed_operations_with_uuid(&mut actual, &operations, "ru", &mut || unreachable!()).is_err(), "{property}");
        assert_eq!(actual, original, "{property}");
    }
}

#[test]
fn task_presentation_uses_its_own_enum_in_both_xml_validators() {
    for (kind, value, valid) in [
        (MetadataKind::Task, "AsNumber", true),
        (MetadataKind::Task, "AsCode", false),
        (MetadataKind::Catalog, "AsNumber", false),
    ] {
        let source = xml(
            kind,
            &format!("<DefaultPresentation>{value}</DefaultPresentation>"),
        );
        assert_eq!(
            crate::format_contract::validate_metadata_8_3_27_enum_contract(&source, "test").is_ok(),
            valid
        );
        let doc = Document::parse(source.trim_start_matches('\u{feff}')).unwrap();
        let properties = doc
            .descendants()
            .find(|node| node.has_tag_name("Properties"));
        let mut report = crate::validation::MetaValidationReporter::new(10);
        crate::validation::meta_validate_check_property_values(&mut report, properties);
        assert_eq!(report.errors.is_empty(), valid, "{kind:?}: {value}");
    }
}

#[test]
fn folders_on_top_rejects_damaged_preimages_atomically() {
    let kind = MetadataKind::ChartOfCharacteristicTypes;
    for property in [
        "",
        "<FoldersOnTop/>",
        "<FoldersOnTop>yes</FoldersOnTop>",
        "<FoldersOnTop><Value>true</Value></FoldersOnTop>",
        "<FoldersOnTop extra=\"x\">true</FoldersOnTop>",
        "<FoldersOnTop xmlns=\"urn:foreign\">true</FoldersOnTop>",
        "<FoldersOnTop>true</FoldersOnTop><FoldersOnTop>false</FoldersOnTop>",
    ] {
        let mut actual = xml(
            kind,
            &format!("<DefaultPresentation>AsDescription</DefaultPresentation>{property}"),
        );
        let original = actual.clone();
        let values = MetaPropertyChanges::convert(
            kind,
            vec![
                MetaPropertyInput::new(
                    "DefaultPresentation",
                    MetaPropertyValue::String("AsCode".into()),
                ),
                MetaPropertyInput::new("FoldersOnTop", MetaPropertyValue::Boolean(false)),
            ],
        )
        .unwrap();
        assert!(
            apply_typed_operations_with_uuid(
                &mut actual,
                &[MetaEditOperation::SetProperties { values }],
                "ru",
                &mut || unreachable!()
            )
            .is_err(),
            "{property}"
        );
        assert_eq!(actual, original);
    }
}
