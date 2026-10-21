use super::{write, RealReaderFixture, ViewRequest};
use serde_json::{json, Value};
use unica_workspace::test_support::tree_snapshot;

const CATALOG: &str = include_str!(
    "../../../../../../tests/fixtures/acceptance/workspace-catalog-tabular-properties/src/Catalogs/Items.xml"
);

fn fixture_with_catalog(xml: &str) -> RealReaderFixture {
    let fixture = RealReaderFixture::new();
    write(&fixture.source.join("Catalogs/Items.xml"), xml);
    fixture
}

fn read(fixture: &RealReaderFixture, at: &str) -> Value {
    let viewed = fixture.view_service().view(ViewRequest::new(at).unwrap());
    assert!(viewed.ok, "view must succeed at {at}");
    viewed.data.unwrap()
}

fn section_views(fixture: &RealReaderFixture) -> [Value; 2] {
    let collection = read(fixture, "main:Catalog.Items.TabularSection");
    let item = collection["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["at"] == "main:Catalog.Items.TabularSection.Lines")
        .expect("the Lines section must remain visible")
        .clone();
    let detail = read(fixture, "main:Catalog.Items.TabularSection.Lines");
    [item, detail]
}

#[test]
fn catalog_tabular_profile_reaches_public_collection_and_detail_without_writes() {
    let fixture = fixture_with_catalog(CATALOG);
    let before = tree_snapshot(&fixture.source);
    for section in section_views(&fixture) {
        assert_eq!(
            section["props"]["LineNumberLength"], 5,
            "the section must preserve the expected fixture facts"
        );
        assert_eq!(
            section["props"]["Use"], "ForItem",
            "the section must preserve the expected fixture facts"
        );
        assert_eq!(section["props"]["required"], true);
        assert_eq!(section["props"]["synonym"], "Order lines");
        assert_eq!(section["title"], "Order lines");
        assert_eq!(
            section["branches"],
            json!([{
                "at": "main:Catalog.Items.TabularSection.Lines.Attribute", "count": 1
            }, {
                "at": "main:Catalog.Items.TabularSection.Lines.StandardAttribute", "count": 1
            }])
        );
    }
    let sibling = read(&fixture, "main:Catalog.Items.TabularSection.Notes");
    assert_eq!(sibling["props"]["LineNumberLength"], 3);
    assert_eq!(sibling["props"]["Use"], "ForFolder");
    assert_eq!(sibling["props"]["required"], false);
    let column = read(
        &fixture,
        "main:Catalog.Items.TabularSection.Lines.Attribute.Quantity",
    );
    assert_eq!(column["props"]["required"], true);
    let column_type: Value =
        serde_json::from_str(column["props"]["type"].as_str().unwrap()).unwrap();
    assert_eq!(
        column_type["variants"][0]["kind"], "number",
        "the column type must preserve the expected qualifiers"
    );
    assert_eq!(
        column_type["variants"][0]["digits"], 10,
        "the column type must preserve the expected qualifiers"
    );
    assert_eq!(
        column_type["variants"][0]["fraction"], 2,
        "the column type must preserve the expected qualifiers"
    );
    assert_eq!(
        column_type["variants"][0]["sign"], "nonNegative",
        "the column type must preserve the expected qualifiers"
    );
    let sibling_column = read(
        &fixture,
        "main:Catalog.Items.TabularSection.Notes.Attribute.Text",
    );
    let sibling_type: Value =
        serde_json::from_str(sibling_column["props"]["type"].as_str().unwrap()).unwrap();
    assert_eq!(
        sibling_type["variants"][0]["kind"], "string",
        "the sibling type must preserve the expected qualifiers"
    );
    assert_eq!(
        sibling_type["variants"][0]["length"], 80,
        "the sibling type must preserve the expected qualifiers"
    );
    assert_eq!(
        sibling_type["variants"][0]["allowedLength"], "variable",
        "the sibling type must preserve the expected qualifiers"
    );
    assert_eq!(tree_snapshot(&fixture.source), before);
}

#[test]
fn catalog_tabular_profile_reads_each_use_value_without_inventing_hierarchy_rules() {
    for value in ["ForItem", "ForFolder", "ForFolderAndItem"] {
        for hierarchical in ["true", "false"] {
            let xml = CATALOG
                .replacen("<Use>ForItem</Use>", &format!("<Use>{value}</Use>"), 1)
                .replace(
                    "<Hierarchical>true</Hierarchical>",
                    &format!("<Hierarchical>{hierarchical}</Hierarchical>"),
                );
            let fixture = fixture_with_catalog(&xml);
            for section in section_views(&fixture) {
                assert_eq!(
                    section["props"]["Use"], value,
                    "the section must preserve the expected fixture facts"
                );
                assert_eq!(section["props"]["LineNumberLength"], 5);
            }
        }
    }
}

#[test]
fn catalog_tabular_profile_missing_facts_are_not_filled_from_sibling_or_template() {
    let xml = CATALOG
        .replacen("<LineNumberLength>5</LineNumberLength>", "", 1)
        .replacen("<Use>ForItem</Use>", "", 1);
    let fixture = fixture_with_catalog(&xml);
    let before = tree_snapshot(&fixture.source);
    for section in section_views(&fixture) {
        assert!(
            section["props"].get("LineNumberLength").is_none(),
            "the section must preserve the expected fixture facts"
        );
        assert!(
            section["props"].get("Use").is_none(),
            "the section must preserve the expected fixture facts"
        );
        assert_eq!(section["props"]["required"], true);
        assert!(
            !section["limits"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|limit| {
                    let text = limit.as_str().unwrap_or_default();
                    text.contains("LineNumberLength") || text.contains(".Use")
                }),
            "optional absent facts are not damaged"
        );
    }
    let sibling = read(&fixture, "main:Catalog.Items.TabularSection.Notes");
    assert_eq!(sibling["props"]["LineNumberLength"], 3);
    assert_eq!(sibling["props"]["Use"], "ForFolder");
    assert_eq!(tree_snapshot(&fixture.source), before);
}

#[test]
fn catalog_tabular_profile_damage_is_local_to_the_observed_fact() {
    for (field, original, replacements) in [
        ("LineNumberLength", "<LineNumberLength>5</LineNumberLength>", vec![
            "<LineNumberLength>broken</LineNumberLength>",
            "<LineNumberLength>-1</LineNumberLength>",
            "<LineNumberLength>4294967296</LineNumberLength>",
            "<LineNumberLength>5.5</LineNumberLength>",
            "<LineNumberLength/>",
            "<LineNumberLength xsi:nil=\"true\"/>",
            "<LineNumberLength>5</LineNumberLength><LineNumberLength>6</LineNumberLength>",
            "<LineNumberLength xmlns=\"urn:foreign\">5</LineNumberLength>",
            "<LineNumberLength>5</LineNumberLength><LineNumberLength xmlns=\"urn:foreign\">6</LineNumberLength>",
            "<LineNumberLength><v8:Type>5</v8:Type></LineNumberLength>",
        ]),
        ("Use", "<Use>ForItem</Use>", vec![
            "<Use>Unknown</Use>",
            "<Use/>",
            "<Use xsi:nil=\"true\"/>",
            "<Use>ForItem</Use><Use>ForFolder</Use>",
            "<Use xmlns=\"urn:foreign\">ForItem</Use>",
            "<Use>ForItem</Use><Use xmlns=\"urn:foreign\">ForFolder</Use>",
            "<Use><v8:Type>ForItem</v8:Type></Use>",
        ]),
    ] {
        for replacement in replacements {
            let xml = CATALOG.replacen(original, replacement, 1);
            let fixture = fixture_with_catalog(&xml);
            let before = tree_snapshot(&fixture.source);
            for section in section_views(&fixture) {
                assert!(section["props"].get(field).is_none(), "fixture {replacement}, field {field}");
                let (independent, expected) = if field == "Use" {
                    ("LineNumberLength", json!(5))
                } else {
                    ("Use", json!("ForItem"))
                };
                assert_eq!(section["props"][independent], expected, "fixture {replacement}, field {field}");
                assert_eq!(section["props"]["required"], true);
                assert_eq!(section["props"]["synonym"], "Order lines");
                assert!(section["limits"].as_array().into_iter().flatten()
                    .filter_map(Value::as_str).any(|limit| limit.contains(field)),
                    "the damaged fact must remain diagnostic: fixture {replacement}, field {field}");
            }
            let root = read(&fixture, "main:Catalog.Items");
            assert!(root["limits"].as_array().into_iter().flatten()
                .filter_map(Value::as_str).any(|limit| limit.contains(field)),
                "the owner must name the damaged fact too: fixture {replacement}, field {field}");
            let column = read(&fixture, "main:Catalog.Items.TabularSection.Lines.Attribute.Quantity");
            assert_eq!(column["props"]["required"], true);
            assert!(column["props"]["type"].is_string());
            let sibling = read(&fixture, "main:Catalog.Items.TabularSection.Notes");
            assert_eq!(sibling["props"]["LineNumberLength"], 3);
            assert_eq!(sibling["props"]["Use"], "ForFolder");
            assert_eq!(tree_snapshot(&fixture.source), before);
        }
    }
}

#[test]
fn catalog_tabular_profile_does_not_publish_unknown_or_wrong_context_facts() {
    let xml = CATALOG
        .replacen("<LineNumberLength>5</LineNumberLength>",
            "<Secret>do-not-publish</Secret><LineNumberLength>5</LineNumberLength>", 1)
        .replacen("<Name>Quantity</Name>",
            "<Name>Quantity</Name><LineNumberLength>7</LineNumberLength><Use>ForFolderAndItem</Use>", 1);
    let fixture = fixture_with_catalog(&xml);
    for section in section_views(&fixture) {
        assert!(
            section["props"].get("Secret").is_none(),
            "the section must preserve the expected fixture facts"
        );
        assert_eq!(section["props"]["LineNumberLength"], 5);
        assert_eq!(section["props"]["Use"], "ForItem");
    }
    let root = read(&fixture, "main:Catalog.Items");
    let column = read(
        &fixture,
        "main:Catalog.Items.TabularSection.Lines.Attribute.Quantity",
    );
    for wrong_context in [root, column] {
        assert!(
            wrong_context["props"].get("LineNumberLength").is_none(),
            "the profile must not publish facts in the wrong context"
        );
        assert!(
            wrong_context["props"].get("Use").is_none(),
            "the profile must not publish facts in the wrong context"
        );
    }

    let extension = fixture
        .extension_view_service()
        .view(ViewRequest::new("main:Catalog.Items.TabularSection.Lines").unwrap());
    assert!(extension.ok, "the extension view must succeed");
    let extension = extension.data.unwrap();
    assert!(
        extension["props"].get("LineNumberLength").is_none(),
        "the Catalog-only profile must not publish extension facts"
    );
    assert!(
        extension["props"].get("Use").is_none(),
        "the Catalog-only profile must not publish extension facts"
    );

    let document_xml = CATALOG
        .replace("<Catalog uuid=", "<Document uuid=")
        .replace("</Catalog>", "</Document>")
        .replace("<Name>Items</Name>", "<Name>Order</Name>");
    write(&fixture.source.join("Documents/Order.xml"), &document_xml);
    let document = read(&fixture, "main:Document.Order.TabularSection.Lines");
    assert!(
        document["props"].get("LineNumberLength").is_none(),
        "the Catalog-only profile must not publish document facts"
    );
    assert!(
        document["props"].get("Use").is_none(),
        "the Catalog-only profile must not publish document facts"
    );
}

#[test]
fn catalog_tabular_profile_requires_the_exact_declared_xml_version() {
    for declaration in [
        " version=\"2.19\"",
        " version=\"2.21\"",
        "",
        " version=\"2&#x2e;20\"",
    ] {
        let xml = CATALOG.replace(" version=\"2.20\"", declaration);
        let fixture = fixture_with_catalog(&xml);
        let before = tree_snapshot(&fixture.source);
        for section in section_views(&fixture) {
            assert!(
                section["props"].get("LineNumberLength").is_none(),
                "the pilot requires the exact declared version: {declaration}"
            );
            assert!(
                section["props"].get("Use").is_none(),
                "the pilot requires the exact declared version: {declaration}"
            );
            assert_eq!(section["props"]["required"], true);
            assert_eq!(section["props"]["synonym"], "Order lines");
            assert_eq!(
                section["branches"],
                json!([{
                    "at": "main:Catalog.Items.TabularSection.Lines.Attribute", "count": 1
                }, {
                    "at": "main:Catalog.Items.TabularSection.Lines.StandardAttribute", "count": 1
                }])
            );
        }
        let column = read(
            &fixture,
            "main:Catalog.Items.TabularSection.Lines.Attribute.Quantity",
        );
        assert_eq!(column["props"]["required"], true);
        let column_type: Value =
            serde_json::from_str(column["props"]["type"].as_str().unwrap()).unwrap();
        assert_eq!(column_type["variants"][0]["kind"], "number");
        assert_eq!(column_type["variants"][0]["digits"], 10);
        assert_eq!(column_type["variants"][0]["fraction"], 2);
        assert_eq!(tree_snapshot(&fixture.source), before);
    }
}
