use unica_metadata::{MetaCollection, MetaEditOperation, MetaElementUpdate, MetaScope};
const INFO: &str = include_str!("../../../../tests/fixtures/acceptance/workspace-register-filling-values/src/InformationRegisters/Facts.xml");
const CATALOG: &str = include_str!(
    "../../../../tests/fixtures/acceptance/workspace-member-filling-values/src/Catalogs/Items.xml"
);
const REPORT: &str = include_str!("../../../../tests/fixtures/acceptance/workspace-member-filling-values/src/Reports/FillReport.xml");
const PROCESSOR: &str = include_str!("../../../../tests/fixtures/acceptance/workspace-member-filling-values/src/DataProcessors/FillProcessor.xml");
fn op(
    collection: MetaCollection,
    scope: Option<&str>,
    name: &str,
    value: bool,
) -> MetaEditOperation {
    MetaEditOperation::Update {
        collection,
        scope: scope.map(|name| MetaScope {
            tabular_section: name.into(),
        }),
        elements: vec![MetaElementUpdate {
            name: name.into(),
            new_name: None,
            synonym: None,
            comment: None,
            r#type: None,
            required: None,
            fill_value: None,
            position: None,
            indexing: None,
            deny_incomplete_values: None,
            fill_from_filling_value: Some(value),
        }],
    }
}
fn apply(
    xml: &mut String,
    operations: &[MetaEditOperation],
) -> Result<crate::edit::MetaEditCounts, unica_metadata::request::MetaFailure> {
    crate::edit::apply_typed_operations_with_uuid(xml, operations, "ru", &mut || {
        "11111111-1111-4111-8111-111111111111".into()
    })
}
fn native_cases() -> Vec<(
    &'static str,
    MetaCollection,
    Option<&'static str>,
    &'static str,
)> {
    vec![
        (INFO, MetaCollection::Attributes, None, "AttributeValue"),
        (INFO, MetaCollection::Dimensions, None, "DimensionValue"),
        (INFO, MetaCollection::Resources, None, "ResourceValue"),
        (CATALOG, MetaCollection::Attributes, None, "FillProbe"),
        (
            REPORT,
            MetaCollection::Attributes,
            Some("Rows"),
            "FillProbe",
        ),
        (
            PROCESSOR,
            MetaCollection::Attributes,
            Some("Rows"),
            "FillProbe",
        ),
    ]
}
fn token_range(xml: &str, name: &str) -> std::ops::Range<usize> {
    let doc = roxmltree::Document::parse(xml).unwrap();
    let member = doc
        .descendants()
        .find(|node| {
            node.is_element()
                && matches!(
                    node.tag_name().name(),
                    "Attribute" | "Dimension" | "Resource"
                )
                && node
                    .children()
                    .find(|p| p.is_element() && p.tag_name().name() == "Properties")
                    .is_some_and(|p| {
                        p.children().any(|n| {
                            n.is_element()
                                && n.tag_name().name() == "Name"
                                && n.text() == Some(name)
                        })
                    })
        })
        .unwrap();
    member
        .descendants()
        .find(|n| n.is_element() && n.tag_name().name() == "FillFromFillingValue")
        .unwrap()
        .first_child()
        .unwrap()
        .range()
}
#[test]
fn filling_six_native_contexts_roundtrip_preserve_independent_bytes_and_effects() {
    for (source, collection, scope, name) in native_cases() {
        for eol in ["\n", "\r\n"] {
            let before = source
                .trim_start_matches('\u{feff}')
                .replace("\r\n", "\n")
                .replace('\n', eol);
            let original = &before[token_range(&before, name)];
            let initial = original == "true";
            assert!(matches!(original, "true" | "false"));
            let mut after = before.clone();
            let result = apply(&mut after, &[op(collection, scope, name, !initial)]).unwrap();
            let mut expected = before.clone();
            expected.replace_range(
                token_range(&before, name),
                if initial { "false" } else { "true" },
            );
            assert_eq!(after, expected, "{name}");
            assert_eq!(
                result.effects[0].before.as_ref().unwrap()[0]["FillFromFillingValue"],
                initial
            );
            assert_eq!(
                result.effects[0].after.as_ref().unwrap()[0]["FillFromFillingValue"],
                !initial
            );
            assert_ne!(result.effects[0].before, result.effects[0].after);
            let noop = apply(&mut after, &[op(collection, scope, name, !initial)]).unwrap();
            assert_eq!(after, expected);
            assert_eq!(noop.effects[0].before, noop.effects[0].after);
            apply(&mut after, &[op(collection, scope, name, initial)]).unwrap();
            assert_eq!(after, before);
        }
    }
}
#[test]
fn filling_refuses_damaged_or_absent_property_and_exact_version_without_partial_edit() {
    for replacement in ["","<FillFromFillingValue/>","<FillFromFillingValue>1</FillFromFillingValue>","<FillFromFillingValue>true</FillFromFillingValue><FillFromFillingValue>true</FillFromFillingValue>","<FillFromFillingValue xmlns=\"urn:foreign\">true</FillFromFillingValue>","<FillFromFillingValue><!--keep-->true</FillFromFillingValue>","<FillFromFillingValue><![CDATA[true]]></FillFromFillingValue>"] {
        let mut before=INFO.trim_start_matches('\u{feff}').to_string();
        let range=token_range(&before,"DimensionValue");
        before.replace_range(range.start-"<FillFromFillingValue>".len()..range.end+"</FillFromFillingValue>".len(),replacement);
        let mut after=before.clone();assert!(apply(&mut after,&[op(MetaCollection::Dimensions,None,"DimensionValue",false)]).is_err());assert_eq!(after,before);
    }
    for version in [
        "version=\"2.19\"",
        "version=\"2.&#50;0\"",
        "version=\"2.&#x32;0\"",
        "version=\"2.200\"",
        "",
    ] {
        let before = INFO
            .trim_start_matches('\u{feff}')
            .replacen("version=\"2.20\"", version, 1);
        let mut after = before.clone();
        assert!(apply(
            &mut after,
            &[op(
                MetaCollection::Dimensions,
                None,
                "DimensionValue",
                false
            )]
        )
        .is_err());
        assert_eq!(after, before);
    }
    let before = INFO.trim_start_matches('\u{feff}').to_string();
    let mut after = before.clone();
    assert!(apply(
        &mut after,
        &[
            op(MetaCollection::Dimensions, None, "DimensionValue", false),
            op(MetaCollection::Resources, None, "Missing", false)
        ]
    )
    .is_err());
    assert_eq!(after, before);
}
#[test]
fn filling_rename_and_master_true_preserve_independent_fields() {
    let mut source = INFO
        .trim_start_matches('\u{feff}')
        .replace("<Master>false</Master>", "<Master>true</Master>");
    let mut operation = op(MetaCollection::Dimensions, None, "DimensionValue", false);
    let MetaEditOperation::Update { elements, .. } = &mut operation else {
        unreachable!()
    };
    elements[0].new_name = Some("Renamed".into());
    let result = apply(&mut source, &[operation]).unwrap();
    assert_eq!(
        result.effects[0].after.as_ref().unwrap()[0]["name"],
        "Renamed"
    );
    assert_eq!(
        result.effects[0].after.as_ref().unwrap()[0]["FillFromFillingValue"],
        false
    );
    assert!(source.contains("<Master>true</Master>"));
}

#[test]
fn filling_mixed_selected_members_do_not_require_unrequested_sibling_flag() {
    for (source, collection, scope, name) in native_cases()
        .into_iter()
        .filter(|(_, collection, _, _)| *collection == MetaCollection::Attributes)
    {
        let source = source.trim_start_matches('\u{feff}');
        let doc = roxmltree::Document::parse(source).unwrap();
        let node = doc
            .descendants()
            .find(|n| n.is_element() && n.tag_name().name() == "Name" && n.text() == Some(name))
            .unwrap()
            .parent_element()
            .unwrap()
            .parent_element()
            .unwrap();
        let range = node.range();
        let mut sibling = source[range.clone()]
            .replace(&format!("<Name>{name}</Name>"), "<Name>Other</Name>")
            .replace("<FillFromFillingValue>true</FillFromFillingValue>", "")
            .replace("<FillFromFillingValue>false</FillFromFillingValue>", "");
        let uuid_start = sibling.find("uuid=\"").unwrap() + 6;
        sibling.replace_range(
            uuid_start..uuid_start + 36,
            "11111111-1111-4111-8111-111111111111",
        );
        let before = format!(
            "{}{}{}",
            &source[..range.end],
            sibling,
            &source[range.end..]
        );
        let mut operation = op(collection, scope, name, false);
        let MetaEditOperation::Update { elements, .. } = &mut operation else {
            unreachable!()
        };
        let mut other = elements[0].clone();
        other.name = "Other".into();
        other.fill_from_filling_value = None;
        other.comment = Some("Independent".into());
        elements.push(other);
        let mut after = before.clone();
        let result = apply(&mut after, &[operation]).unwrap();
        assert_eq!(
            result.effects[0].after.as_ref().unwrap()[0]["FillFromFillingValue"],
            false
        );
        assert!(result.effects[0].after.as_ref().unwrap()[1]
            .get("FillFromFillingValue")
            .is_none());
        assert!(after.contains("<Comment>Independent</Comment>"));
    }
}
#[test]
fn filling_property_accepts_namespace_alias_without_rewriting_surrounding_xml() {
    let before=CATALOG.trim_start_matches('\u{feff}').replace("<FillFromFillingValue>true</FillFromFillingValue>","<m:FillFromFillingValue xmlns:m=\"http://v8.1c.ru/8.3/MDClasses\">true</m:FillFromFillingValue>");
    let mut after = before.clone();
    apply(
        &mut after,
        &[op(MetaCollection::Attributes, None, "FillProbe", false)],
    )
    .unwrap();
    assert_eq!(
        after,
        before.replace(
            ">true</m:FillFromFillingValue>",
            ">false</m:FillFromFillingValue>"
        )
    );
}
