use super::apply_typed_operations_to_image_with_seed;
use serde_json::{json, Value};
use unica_metadata::{MetaCollection, MetaEditOperation};

fn catalog() -> String {
    include_str!("../../../../tests/fixtures/acceptance/workspace-catalog-tabular-properties/src/Catalogs/Items.xml")
        .trim_start_matches('\u{feff}').to_string()
}

fn operation(mut values: Value) -> MetaEditOperation {
    values["name"] = json!("Lines");
    MetaEditOperation::update(
        MetaCollection::TabularSections,
        None,
        vec![unica_metadata::request::parse_element_update(&values, "values").unwrap()],
    )
    .unwrap()
}

fn apply(xml: &str, values: Value) -> Result<String, unica_metadata::request::MetaFailure> {
    let mut image = xml.to_string();
    apply_typed_operations_to_image_with_seed(&mut image, &[operation(values)], "", b"1689")?;
    Ok(image)
}

#[test]
fn tabular_properties_change_only_the_named_existing_values_and_roundtrip_bytes() {
    let original = catalog();
    let mut before = original.clone();
    for value in ["ForFolder", "ForFolderAndItem", "ForItem"] {
        let after = apply(&before, json!({"Use":value})).unwrap();
        let expected = before.replacen(
            if before.contains("<Use>ForFolderAndItem</Use>") {
                "<Use>ForFolderAndItem</Use>"
            } else if before.contains("<Use>ForItem</Use>") {
                "<Use>ForItem</Use>"
            } else {
                "<Use>ForFolder</Use>"
            },
            &format!("<Use>{value}</Use>"),
            1,
        );
        assert_eq!(after, expected);
        assert_eq!(apply(&after, json!({"Use":value})).unwrap(), after);
        before = after;
    }
    assert_eq!(before, original);
    let unchecked = apply(&original, json!({"required":false})).unwrap();
    assert_eq!(
        unchecked,
        original.replacen(
            "<FillChecking>ShowError</FillChecking>",
            "<FillChecking>DontCheck</FillChecking>",
            1
        )
    );
    assert_eq!(
        apply(&unchecked, json!({"required":true})).unwrap(),
        original
    );
}

#[test]
fn tabular_properties_refuse_missing_duplicate_foreign_and_malformed_preimages() {
    for (field, original, variants) in [
        (
            "Use",
            "<Use>ForItem</Use>",
            vec![
                "",
                "<Use/>",
                "<Use>Unknown</Use>",
                "<Use>ForItem</Use><Use>ForItem</Use>",
                "<Use xmlns=\"urn:foreign\">ForItem</Use>",
                "<Use x=\"1\">ForItem</Use>",
                "<Use><Use>ForItem</Use></Use>",
            ],
        ),
        (
            "required",
            "<FillChecking>ShowError</FillChecking>",
            vec![
                "",
                "<FillChecking/>",
                "<FillChecking>Unknown</FillChecking>",
                "<FillChecking>ShowError</FillChecking><FillChecking>ShowError</FillChecking>",
                "<FillChecking xmlns=\"urn:foreign\">ShowError</FillChecking>",
                "<FillChecking x=\"1\">ShowError</FillChecking>",
                "<FillChecking><FillChecking>ShowError</FillChecking></FillChecking>",
            ],
        ),
    ] {
        for variant in variants {
            let damaged = catalog().replacen(original, variant, 1);
            let values = json!({"Use":"ForFolder", "required":false});
            let mut image = damaged.clone();
            let failure = apply_typed_operations_to_image_with_seed(
                &mut image,
                &[operation(values)],
                "",
                b"1689",
            )
            .unwrap_err();
            assert_eq!(image, damaged, "{field}: {variant}");
            assert_eq!(
                failure.diagnostics[0].code,
                unica_metadata::MetaDiagnosticCode::InvalidArguments
            );
        }
    }
}

#[test]
fn tabular_property_updates_accept_native_owner_matrix_and_restore_every_byte() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/acceptance/workspace-tabular-section-properties/src");
    for (kind, name, uses, required) in [
        ("Catalogs", "Items", true, true),
        ("Catalogs", "FlatItems", true, true),
        ("ChartsOfCharacteristicTypes", "Properties", true, false),
        ("Documents", "Order", false, true),
        ("BusinessProcesses", "Process", false, true),
        ("ExchangePlans", "Exchange", false, true),
        ("Reports", "Report", false, true),
        ("DataProcessors", "Processor", false, true),
    ] {
        let original =
            std::fs::read_to_string(root.join(kind).join(format!("{name}.xml"))).unwrap();
        let original = original.trim_start_matches('\u{feff}');
        let mut image = original.to_string();
        if uses {
            for value in ["ForFolder", "ForFolderAndItem", "ForItem"] {
                image = apply(&image, json!({"Use":value})).unwrap();
            }
        } else {
            assert!(apply(&image, json!({"Use":"ForItem"})).is_err());
        }
        if required {
            image = apply(&image, json!({"required":true})).unwrap();
            image = apply(&image, json!({"required":false})).unwrap();
        } else {
            assert!(apply(&image, json!({"required":true})).is_err());
        }
        assert_eq!(image, original, "{kind}/{name}");
    }
}

#[test]
fn tabular_properties_keep_explicit_metadata_prefixes() {
    let original = catalog().replace(
        "xmlns=",
        "xmlns:md=\"http://v8.1c.ru/8.3/MDClasses\" xmlns=",
    );
    let original = original
        .replace("<Use>", "<md:Use>")
        .replace("</Use>", "</md:Use>")
        .replace("<TabularSection ", "<md:TabularSection ")
        .replace("</TabularSection>", "</md:TabularSection>");
    let image = apply(&original, json!({"Use":"ForFolder"})).unwrap();
    assert_eq!(
        image,
        original.replacen("<md:Use>ForItem</md:Use>", "<md:Use>ForFolder</md:Use>", 1)
    );
}

#[test]
fn tabular_use_semantic_effect_observes_the_requested_property() {
    let before = catalog();
    let after = apply(&before, json!({"Use":"ForFolder"})).unwrap();
    for name in ["Lines", "lines"] {
        let mut op = operation(json!({"Use":"ForFolder"}));
        if let MetaEditOperation::Update { elements, .. } = &mut op {
            elements[0].name = name.into();
        }
        let old = super::typed_operation_effect_value(&before, &op, false)
            .unwrap()
            .unwrap();
        let new = super::typed_operation_effect_value(&after, &op, true)
            .unwrap()
            .unwrap();
        assert_eq!(old[0]["properties"][0]["value"]["value"], "ForItem");
        assert_eq!(new[0]["properties"][0]["value"]["value"], "ForFolder");
    }
}
