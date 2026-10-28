use unica_metadata::{MetaCollection, MetaEditOperation, MetaElementUpdate};
fn register() -> String {
    include_str!("../../../../tests/fixtures/acceptance/workspace-calculation-base-dimension/src/CalculationRegisters/Accruals.xml")
        .trim_start_matches('\u{feff}').into()
}
fn operation(name: &str, value: bool) -> MetaEditOperation {
    MetaEditOperation::Update {
        collection: MetaCollection::Dimensions,
        scope: None,
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
            base_dimension: Some(value),
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
#[test]
fn base_dimension_roundtrip_preserves_bytes_and_records_actual_semantic_effects() {
    for eol in ["\n", "\r\n"] {
        let before = register().replace("\r\n", "\n").replace('\n', eol);
        let mut after = before.clone();
        let changed = apply(&mut after, &[operation("Employee", true)]).unwrap();
        assert_eq!(
            after,
            before.replacen(
                "<BaseDimension>false</BaseDimension>",
                "<BaseDimension>true</BaseDimension>",
                1
            )
        );
        assert_ne!(changed.effects[0].before, changed.effects[0].after);
        assert_eq!(
            changed.effects[0].before.as_ref().unwrap()[0]["BaseDimension"],
            false
        );
        assert_eq!(
            changed.effects[0].after.as_ref().unwrap()[0]["BaseDimension"],
            true
        );
        let saved = after.clone();
        let noop = apply(&mut after, &[operation("Employee", true)]).unwrap();
        assert_eq!(saved, after);
        assert_eq!(noop.effects[0].before, noop.effects[0].after);
        apply(&mut after, &[operation("Employee", false)]).unwrap();
        assert_eq!(after, before);
    }
}
#[test]
fn base_dimension_rename_keeps_flag_effect_identity() {
    let mut xml = register();
    let mut op = operation("Employee", true);
    let MetaEditOperation::Update { elements, .. } = &mut op else {
        unreachable!()
    };
    elements[0].new_name = Some("Person".into());
    let result = apply(&mut xml, &[op]).unwrap();
    assert_eq!(
        result.effects[0].before.as_ref().unwrap()[0]["BaseDimension"],
        false
    );
    assert_eq!(
        result.effects[0].after.as_ref().unwrap()[0]["name"],
        "Person"
    );
    assert_eq!(
        result.effects[0].after.as_ref().unwrap()[0]["BaseDimension"],
        true
    );
}
#[test]
fn base_dimension_refuses_damaged_existing_property_without_repair_and_batch_is_atomic() {
    for replacement in [
        "",
        "<BaseDimension/>",
        "<BaseDimension>1</BaseDimension>",
        "<BaseDimension>True</BaseDimension>",
        "<BaseDimension>Future</BaseDimension>",
        "<BaseDimension>true</BaseDimension><BaseDimension>true</BaseDimension>",
        "<BaseDimension xmlns=\"urn:foreign\">true</BaseDimension>",
        "<BaseDimension><!--keep-->true</BaseDimension>",
        "<BaseDimension><![CDATA[true]]></BaseDimension>",
    ] {
        let before = register().replace("<BaseDimension>false</BaseDimension>", replacement);
        let mut after = before.clone();
        assert!(
            apply(&mut after, &[operation("Employee", true)]).is_err(),
            "{replacement}"
        );
        assert_eq!(after, before);
    }
    let before = register();
    let mut after = before.clone();
    assert!(apply(
        &mut after,
        &[operation("Employee", true), operation("Missing", true)]
    )
    .is_err());
    assert_eq!(after, before);
}
#[test]
fn base_dimension_uses_namespace_identity_and_rejects_unsupported_format() {
    let before = register().replace(
        "<BaseDimension>false</BaseDimension>",
        "<m:BaseDimension xmlns:m=\"http://v8.1c.ru/8.3/MDClasses\">false</m:BaseDimension>",
    );
    let mut after = before.clone();
    apply(&mut after, &[operation("Employee", true)]).unwrap();
    assert_eq!(
        after,
        before.replacen(">false</m:BaseDimension>", ">true</m:BaseDimension>", 1)
    );
    for version in [
        "version=\"2.19\"",
        "version=\"2.&#50;0\"",
        "version=\"2.&#x32;0\"",
        "version=\"2.200\"",
        "version=\"02.20\"",
        "version=\"2.20 \"",
        "",
    ] {
        let before = register().replacen("version=\"2.20\"", version, 1);
        let mut after = before.clone();
        assert!(
            apply(&mut after, &[operation("Employee", true)]).is_err(),
            "{version}"
        );
        assert_eq!(after, before);
    }
}

#[test]
fn base_dimension_missing_sibling_does_not_block_selected_or_mixed_update() {
    let source = register();
    let selected_end = source.find("</Dimension>").unwrap() + "</Dimension>".len();
    let before = format!(
        "{}{}",
        &source[..selected_end],
        source[selected_end..].replace("<BaseDimension>false</BaseDimension>", "")
    );
    let mut after = before.clone();
    apply(&mut after, &[operation("Employee", true)]).unwrap();
    assert_eq!(
        after,
        before.replacen(
            "<BaseDimension>false</BaseDimension>",
            "<BaseDimension>true</BaseDimension>",
            1
        )
    );
    let mut mixed = operation("Employee", true);
    let MetaEditOperation::Update { elements, .. } = &mut mixed else {
        unreachable!()
    };
    let mut other = elements[0].clone();
    other.name = "Department".into();
    other.base_dimension = None;
    other.comment = Some("Independent".into());
    elements.push(other);
    let mut after = before.clone();
    let result = apply(&mut after, &[mixed]).unwrap();
    assert_eq!(
        result.effects[0].after.as_ref().unwrap()[0]["BaseDimension"],
        true
    );
    assert!(result.effects[0].after.as_ref().unwrap()[1]
        .get("BaseDimension")
        .is_none());
    assert!(after.contains("<Comment>Independent</Comment>"));
}
