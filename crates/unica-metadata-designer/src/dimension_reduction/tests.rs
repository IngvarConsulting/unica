use unica_metadata::{MetaCollection, MetaEditOperation, MetaElementUpdate};
fn register() -> String {
    include_str!("../../../../tests/fixtures/acceptance/workspace-register-dimensions/src/InformationRegisters/Цены.xml")
        .trim_start_matches('\u{feff}').into()
}
fn operation(name: &str, value: &str) -> MetaEditOperation {
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
            type_reduction_mode: Some(value.into()),
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
fn reduction_roundtrip_preserves_bytes_and_records_actual_semantic_effects() {
    for eol in ["\n", "\r\n"] {
        let before = register().replace("\r\n", "\n").replace('\n', eol);
        let mut after = before.clone();
        let changed = apply(&mut after, &[operation("Товар", "Deny")]).unwrap();
        assert_eq!(
            after,
            before.replace(
                "<TypeReductionMode>TransformValues</TypeReductionMode>",
                "<TypeReductionMode>Deny</TypeReductionMode>"
            )
        );
        assert_ne!(changed.effects[0].before, changed.effects[0].after);
        assert_eq!(
            changed.effects[0].before.as_ref().unwrap()[0]["TypeReductionMode"],
            "TransformValues"
        );
        assert_eq!(
            changed.effects[0].after.as_ref().unwrap()[0]["TypeReductionMode"],
            "Deny"
        );
        let saved = after.clone();
        let noop = apply(&mut after, &[operation("Товар", "Deny")]).unwrap();
        assert_eq!(saved, after);
        assert_eq!(noop.effects[0].before, noop.effects[0].after);
        apply(&mut after, &[operation("Товар", "TransformValues")]).unwrap();
        assert_eq!(after, before);
    }
}
#[test]
fn reduction_accepts_delete_data_preimage_and_rename_but_never_writes_delete_data() {
    let before = register().replace(
        "<TypeReductionMode>TransformValues</TypeReductionMode>",
        "<TypeReductionMode>DeleteData</TypeReductionMode>",
    );
    for value in ["Deny", "TransformValues"] {
        let mut after = before.clone();
        let mut op = operation("Товар", value);
        let MetaEditOperation::Update { elements, .. } = &mut op else {
            unreachable!()
        };
        elements[0].new_name = Some("Product".into());
        let result = apply(&mut after, &[op]).unwrap();
        assert_eq!(
            result.effects[0].before.as_ref().unwrap()[0]["TypeReductionMode"],
            "DeleteData"
        );
        assert_eq!(
            result.effects[0].after.as_ref().unwrap()[0]["name"],
            "Product"
        );
        assert_eq!(
            result.effects[0].after.as_ref().unwrap()[0]["TypeReductionMode"],
            value
        );
    }
    for value in ["DeleteData", "deny", "Future", ""] {
        let mut after = before.clone();
        assert!(apply(&mut after, &[operation("Товар", value)]).is_err());
        assert_eq!(after, before);
    }
}
#[test]
fn reduction_refuses_damaged_existing_property_without_repair_and_batch_is_atomic() {
    for replacement in [
        "",
        "<TypeReductionMode/>",
        "<TypeReductionMode>Future</TypeReductionMode>",
        "<TypeReductionMode>Deny</TypeReductionMode><TypeReductionMode>Deny</TypeReductionMode>",
        "<TypeReductionMode xmlns=\"urn:foreign\">Deny</TypeReductionMode>",
        "<TypeReductionMode><!--keep-->Deny</TypeReductionMode>",
        "<TypeReductionMode><![CDATA[Deny]]></TypeReductionMode>",
    ] {
        let before = register().replace(
            "<TypeReductionMode>TransformValues</TypeReductionMode>",
            replacement,
        );
        let mut after = before.clone();
        assert!(
            apply(&mut after, &[operation("Товар", "Deny")]).is_err(),
            "{replacement}"
        );
        assert_eq!(after, before);
    }
    let before = register();
    let mut after = before.clone();
    assert!(apply(
        &mut after,
        &[operation("Товар", "Deny"), operation("Missing", "Deny")]
    )
    .is_err());
    assert_eq!(after, before);
}
#[test]
fn reduction_uses_namespace_identity_and_rejects_unsupported_format() {
    let before = register().replace("<TypeReductionMode>TransformValues</TypeReductionMode>", "<m:TypeReductionMode xmlns:m=\"http://v8.1c.ru/8.3/MDClasses\">TransformValues</m:TypeReductionMode>");
    let mut after = before.clone();
    apply(&mut after, &[operation("Товар", "Deny")]).unwrap();
    assert_eq!(
        after,
        before.replace(
            ">TransformValues</m:TypeReductionMode>",
            ">Deny</m:TypeReductionMode>"
        )
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
            apply(&mut after, &[operation("Товар", "Deny")]).is_err(),
            "{version}"
        );
        assert_eq!(after, before);
    }
}

#[test]
fn reduction_does_not_require_a_sibling_property() {
    for property in ["", "<TypeReductionMode>Future</TypeReductionMode>"] {
        let source = register();
        let start = source.find("<Dimension uuid=").unwrap();
        let end = start + source[start..].find("</Dimension>").unwrap() + "</Dimension>".len();
        let sibling = source[start..end]
            .replace("<Name>Товар</Name>", "<Name>Sibling</Name>")
            .replace(
                "<TypeReductionMode>TransformValues</TypeReductionMode>",
                property,
            );
        let uuid_start = sibling.find("uuid=\"").unwrap() + 6;
        let mut sibling = sibling;
        sibling.replace_range(
            uuid_start..uuid_start + 36,
            "11111111-1111-4111-8111-111111111111",
        );
        let mut xml = format!("{}{}{}", &source[..end], sibling, &source[end..]);
        let before = xml.clone();
        apply(&mut xml, &[operation("Товар", "Deny")]).unwrap();
        assert_eq!(
            xml,
            before.replacen(
                "<TypeReductionMode>TransformValues</TypeReductionMode>",
                "<TypeReductionMode>Deny</TypeReductionMode>",
                1
            )
        );
        assert!(xml.contains(&sibling));
        let mut mixed = operation("Товар", "Deny");
        let MetaEditOperation::Update { elements, .. } = &mut mixed else {
            unreachable!()
        };
        let mut other = elements[0].clone();
        other.name = "Sibling".into();
        other.type_reduction_mode = None;
        other.comment = Some("Independent".into());
        elements.push(other);
        let mut xml = before.clone();
        let result = apply(&mut xml, &[mixed]).unwrap();
        assert_eq!(
            result.effects[0].after.as_ref().unwrap()[0]["TypeReductionMode"],
            "Deny"
        );
        assert!(result.effects[0].after.as_ref().unwrap()[1]
            .get("TypeReductionMode")
            .is_none());
        assert!(xml.contains("<Comment>Independent</Comment>"));
    }
}
