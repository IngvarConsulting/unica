use super::*;
use unica_workspace::test_support::tree_snapshot;

const ROOT: &str = "main:ExternalDataSource.AllKindsExternalSource";
const CUBE: &str = "main:ExternalDataSource.AllKindsExternalSource.Cube.Sales";
const PRODUCTS: &str =
    "main:ExternalDataSource.AllKindsExternalSource.Cube.Sales.DimensionTable.Products";
const ITEMS: &str = "main:ExternalDataSource.AllKindsExternalSource.Table.Items";
const ROOT_XML: &str = "ExternalDataSources/AllKindsExternalSource.xml";
const CUBE_XML: &str = "ExternalDataSources/AllKindsExternalSource/Cubes/Sales.xml";
const PRODUCTS_XML: &str =
    "ExternalDataSources/AllKindsExternalSource/Cubes/Sales/DimensionTables/Products.xml";
const ITEMS_XML: &str = "ExternalDataSources/AllKindsExternalSource/Tables/Items.xml";

fn fixture_with_support(policy: &str, locked: &str) -> Fixture {
    let fixture = Fixture::blank();
    let original = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/formats/platform_xml/2.20/all_kinds/src");
    for relative in [ROOT_XML, CUBE_XML, PRODUCTS_XML, ITEMS_XML] {
        let target = fixture.source.join(relative);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::copy(original.join(relative), target).unwrap();
    }
    let config = fixture.source.join("Configuration.xml");
    let contents = std::fs::read_to_string(&config).unwrap().replace(
        "</ChildObjects>",
        "<ExternalDataSource>AllKindsExternalSource</ExternalDataSource></ChildObjects>",
    );
    std::fs::write(config, contents).unwrap();
    std::fs::write(
        fixture.workspace.join(".v8-project.json"),
        json!({"editingAllowedCheck": policy}).to_string(),
    )
    .unwrap();
    let marker = fixture.source.join("Ext/ParentConfigurations.bin");
    std::fs::create_dir_all(marker.parent().unwrap()).unwrap();
    std::fs::write(
        marker,
        format!(
            "\u{feff}{{6,0,1,dddddddd-dddd-4ddd-8ddd-dddddddddddd,0,{locked},\"1.0\",\"Vendor\",\"VendorConf\",3,1,0,44444444-4444-4444-8444-444444444444,44444444-4444-4444-8444-444444444444,0,-1,{locked},{locked}}}"
        ),
    )
    .unwrap();
    fixture
}

/// rule: INV.SAFETY.APPLY-SUPPORT-GATE
#[test]
fn denied_external_source_edit_uses_the_uuid_of_the_named_object() {
    let fixture = fixture_with_support("deny", "be05a1ec-2f25-54df-816a-7baed4264299");
    let path = fixture.source.join(ROOT_XML);
    let xml = std::fs::read_to_string(&path).unwrap().replacen(
        "<MetaDataObject ",
        "<MetaDataObject uuid=\"88888888-8888-4888-8888-888888888888\" ",
        1,
    );
    std::fs::write(path, xml).unwrap();
    let before = tree_snapshot(&fixture.source);

    let refused = call(
        &runtime(),
        &fixture,
        ToolIdentity::Apply,
        json!({"at":ROOT,"ops":[{"op":"props.set","args":{"values":{"Comment":"Blocked"}}}]}),
    );

    assert!(!refused.ok, "{refused:?}");
    assert_eq!(refused.diagnostics[0]["code"], "invalid_state");
    assert!(refused
        .data
        .as_ref()
        .and_then(|data| data.get("executionToken"))
        .is_none());
    let choice = refused
        .human_choice
        .as_deref()
        .expect("names the locked root");
    assert_eq!(choice.code, "support.object_locked");
    assert_eq!(
        choice.data.as_ref().unwrap()["object"],
        "ExternalDataSource.AllKindsExternalSource"
    );
    assert_eq!(tree_snapshot(&fixture.source), before);
}

/// rule: INV.SAFETY.APPLY-SUPPORT-GATE
#[test]
fn locked_cube_warning_reaches_the_receipt_for_an_edited_dimension_table() {
    let fixture = fixture_with_support("warn", "4c078d32-b02e-5ced-b1e9-92bb77d916d4");
    let runtime = runtime();
    let cube_before = std::fs::read(fixture.source.join(CUBE_XML)).unwrap();
    let products = json!({"op":"props.set","args":{
        "at": PRODUCTS, "values": {"NameInDataSource":"new_products"}
    }});

    let receipt = publish(
        &runtime,
        &fixture,
        json!({"at":ROOT,"ops":[products.clone()]}),
    );

    let warnings: Vec<_> = receipt
        .warnings
        .iter()
        .filter(|warning| warning["code"] == "support_locked")
        .collect();
    assert_eq!(warnings.len(), 1, "{receipt:?}");
    assert_eq!(warnings[0]["at"], CUBE);
    let descriptor = fixture.source.join(PRODUCTS_XML);
    let products_after = std::fs::read(&descriptor).unwrap();
    assert!(String::from_utf8_lossy(&products_after)
        .contains("<NameInDataSource>new_products</NameInDataSource>"));
    assert_eq!(
        std::fs::read(fixture.source.join(CUBE_XML)).unwrap(),
        cube_before
    );

    let neighbor = json!({"op":"props.set","args":{
        "at": ITEMS, "values": {"NameInDataSource":"new_items"}
    }});
    let receipt = publish(
        &runtime,
        &fixture,
        json!({"at":ROOT,"ops":[products, neighbor]}),
    );

    assert!(
        receipt
            .warnings
            .iter()
            .all(|warning| warning["code"] != "support_locked"),
        "{receipt:?}"
    );
    assert_eq!(std::fs::read(descriptor).unwrap(), products_after);
    assert!(std::fs::read_to_string(fixture.source.join(ITEMS_XML))
        .unwrap()
        .contains("<NameInDataSource>new_items</NameInDataSource>"));
}

/// rule: INV.SAFETY.APPLY-SUPPORT-GATE
#[test]
fn a_locked_field_noop_does_not_warn_when_its_table_changes_in_the_same_xml() {
    const FIELD: &str = "main:ExternalDataSource.AllKindsExternalSource.Table.Items.Field.Code";

    let fixture = fixture_with_support("warn", "9ff1e2c8-e0b1-51c5-b64d-68b6b4c8f866");
    let runtime = runtime();
    let descriptor = fixture.source.join(ITEMS_XML);
    let before = std::fs::read_to_string(&descriptor).unwrap();

    let receipt = publish(
        &runtime,
        &fixture,
        json!({"at": ITEMS, "ops":[
            {"op": "props.set", "args":{"at": FIELD, "values":{"Comment":""}}},
            {"op": "props.set", "args":{"values":{"NameInDataSource":"new_items"}}}
        ]}),
    );

    assert!(
        receipt
            .warnings
            .iter()
            .all(|warning| warning["code"] != "support_locked"),
        "{receipt:?}"
    );
    assert_eq!(
        std::fs::read_to_string(&descriptor).unwrap(),
        before.replacen(
            "<NameInDataSource>items</NameInDataSource>",
            "<NameInDataSource>new_items</NameInDataSource>",
            1,
        )
    );

    let changed = publish(
        &runtime,
        &fixture,
        json!({"at": ITEMS, "ops":[
            {"op": "props.set", "args":{"at": FIELD, "values":{"Comment":"Real field edit"}}}
        ]}),
    );

    assert!(
        changed
            .warnings
            .iter()
            .any(|warning| { warning["code"] == "support_locked" && warning["at"] == FIELD }),
        "{changed:?}"
    );
}
