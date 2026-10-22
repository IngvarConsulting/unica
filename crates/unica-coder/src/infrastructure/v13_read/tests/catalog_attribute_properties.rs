use super::{write, RealReaderFixture, ViewRequest};
use serde_json::{json, Value};
use unica_workspace::test_support::tree_snapshot;

const CATALOG: &str = include_str!(
    "../../../../../../tests/fixtures/acceptance/catalog-attribute-properties/src/Catalogs/Items.xml"
);
const SECTION: &str = "main:Catalog.Items.TabularSection.Представления";
const LANGUAGE: &str = "main:Catalog.Items.TabularSection.Представления.Attribute.КодЯзыка";
const SPELLING: &str = "main:Catalog.Items.TabularSection.Представления.Attribute.ПараметрыПрописи";
const ENUMS: [(&str, &str); 3] = [
    ("Indexing", "Index"),
    ("FullTextSearch", "Use"),
    ("DataHistory", "Use"),
];

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

fn attribute_views(fixture: &RealReaderFixture) -> [Value; 2] {
    let section = read(fixture, SECTION);
    let collection_at = section["branches"]
        .as_array()
        .unwrap()
        .iter()
        .find_map(|branch| {
            branch["at"]
                .as_str()
                .filter(|at| at.ends_with(".Attribute"))
        })
        .expect("the section must offer its Attribute branch");
    let collection = read(fixture, collection_at);
    let item = collection["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["at"] == LANGUAGE)
        .expect("КодЯзыка must remain in the public collection")
        .clone();
    let detail = read(fixture, item["at"].as_str().unwrap());
    [item, detail]
}

fn assert_string_type(node: &Value, length: u64) {
    let observed: Value = serde_json::from_str(node["props"]["type"].as_str().unwrap()).unwrap();
    assert_eq!(observed["variants"][0]["kind"], "string");
    assert_eq!(observed["variants"][0]["length"], length);
    assert_eq!(observed["variants"][0]["allowedLength"], "variable");
}

#[test]
fn catalog_attribute_profile_reaches_public_collection_and_detail_without_writes() {
    let fixture = fixture_with_catalog(CATALOG);
    let before = tree_snapshot(&fixture.source);
    for attribute in attribute_views(&fixture) {
        for (field, expected) in ENUMS {
            assert_eq!(
                attribute["props"][field], expected,
                "public {field} must preserve the XML observation"
            );
        }
        assert_eq!(attribute["at"], LANGUAGE);
        assert_eq!(attribute["props"]["required"], false);
        assert_eq!(attribute["props"]["synonym"], "Код языка");
        assert_string_type(&attribute, 10);
    }
    let section = read(&fixture, SECTION);
    assert_eq!(section["props"]["LineNumberLength"], 5);
    assert_eq!(section["props"]["Use"], "ForItem");
    assert_eq!(section["props"]["required"], false);
    assert_eq!(
        section["branches"],
        json!([
            {"at": format!("{SECTION}.Attribute"), "count": 2},
            {"at": format!("{SECTION}.StandardAttribute"), "count": 1}
        ])
    );
    let sibling = read(&fixture, SPELLING);
    assert_eq!(sibling["props"]["required"], false);
    assert_eq!(sibling["props"]["synonym"], "Параметры прописи");
    assert_string_type(&sibling, 200);
    assert_eq!(sibling["props"]["Indexing"], "DontIndex");
    assert_eq!(sibling["props"]["FullTextSearch"], "Use");
    assert_eq!(sibling["props"]["DataHistory"], "Use");
    assert_eq!(
        serde_json::from_str::<Value>(sibling["props"]["ToolTip"].as_str().unwrap()).unwrap(),
        json!({"kind": "localizedString", "values": [{
            "language": "ru", "content": "Настройки представления суммы прописью"
        }]})
    );
    assert_eq!(tree_snapshot(&fixture.source), before);
}

#[test]
fn catalog_attribute_profile_preserves_all_seven_field_value_combinations() {
    for (field, original, values) in [
        (
            "Indexing",
            "Index",
            &["DontIndex", "Index", "IndexWithAdditionalOrder"][..],
        ),
        ("FullTextSearch", "Use", &["Use", "DontUse"][..]),
        ("DataHistory", "Use", &["Use", "DontUse"][..]),
    ] {
        for expected in values {
            let xml = CATALOG.replacen(
                &format!("<{field}>{original}</{field}>"),
                &format!("<{field}>{expected}</{field}>"),
                1,
            );
            let fixture = fixture_with_catalog(&xml);
            for attribute in attribute_views(&fixture) {
                assert_eq!(
                    attribute["props"][field], *expected,
                    "public {field} must preserve each accepted token"
                );
                for (independent, value) in ENUMS.into_iter().filter(|(name, _)| *name != field) {
                    assert_eq!(
                        attribute["props"][independent], value,
                        "changing {field} must preserve {independent}"
                    );
                }
                assert_string_type(&attribute, 10);
            }
        }
    }
}

fn has_limit(node: &Value, field: &str) -> bool {
    node["limits"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .any(|limit| limit.contains(&format!(".properties.{field}")))
}

fn assert_independent_facts(fixture: &RealReaderFixture, attribute: &Value) {
    assert_eq!(attribute["at"], LANGUAGE);
    assert_eq!(attribute["props"]["required"], false);
    assert_eq!(attribute["props"]["synonym"], "Код языка");
    assert_string_type(attribute, 10);
    let section = read(fixture, SECTION);
    assert_eq!(section["props"]["LineNumberLength"], 5);
    assert_eq!(section["props"]["Use"], "ForItem");
    assert_eq!(section["props"]["required"], false);
    let sibling = read(fixture, SPELLING);
    assert_eq!(sibling["props"]["Indexing"], "DontIndex");
    assert_eq!(sibling["props"]["FullTextSearch"], "Use");
    assert_eq!(sibling["props"]["DataHistory"], "Use");
    assert_eq!(sibling["props"]["required"], false);
    assert_string_type(&sibling, 200);
    let header = read(fixture, "main:Catalog.Items.Attribute.Header");
    assert_string_type(&header, 40);
}

#[test]
fn catalog_attribute_profile_absence_is_not_filled_from_parent_sibling_or_template() {
    for absent in [
        vec!["Indexing"],
        vec!["FullTextSearch"],
        vec!["DataHistory"],
        vec!["Indexing", "FullTextSearch", "DataHistory"],
    ] {
        let mut xml = CATALOG.to_string();
        for (field, original) in ENUMS
            .into_iter()
            .filter(|(field, _)| absent.contains(field))
        {
            xml = xml.replacen(&format!("<{field}>{original}</{field}>"), "", 1);
        }
        let fixture = fixture_with_catalog(&xml);
        let before = tree_snapshot(&fixture.source);
        for attribute in attribute_views(&fixture) {
            for (field, expected) in ENUMS {
                if absent.contains(&field) {
                    assert!(
                        attribute["props"].get(field).is_none(),
                        "absent {field} must stay absent"
                    );
                    assert!(
                        !has_limit(&attribute, field),
                        "optional absent {field} must not be diagnosed as damaged"
                    );
                } else {
                    assert_eq!(
                        attribute["props"][field], expected,
                        "absent neighbours must preserve {field}"
                    );
                }
            }
            assert_independent_facts(&fixture, &attribute);
        }
        let owner = read(&fixture, "main:Catalog.Items");
        for field in absent {
            assert!(
                !has_limit(&owner, field),
                "optional absent {field} must not damage the owner"
            );
        }
        assert_eq!(tree_snapshot(&fixture.source), before);
    }
}

#[test]
fn catalog_attribute_profile_damage_limits_only_the_named_fact_in_owner_row_and_detail() {
    for (field, original) in ENUMS {
        for replacement in [
            format!("<{field}/>"),
            format!("<{field}></{field}>"),
            format!("<{field} xsi:nil=\"true\"/>"),
            format!("<{field}>Unknown</{field}>"),
            format!("<{field}>{original}</{field}><{field}>{original}</{field}>"),
            format!("<{field} extra=\"true\">{original}</{field}>"),
            format!("<{field}><v8:Type>{original}</v8:Type></{field}>"),
            format!("<{field} xmlns=\"urn:foreign\">{original}</{field}>"),
            format!(
                "<{field}>{original}</{field}><{field} xmlns=\"urn:foreign\">{original}</{field}>"
            ),
        ] {
            let xml = CATALOG.replacen(&format!("<{field}>{original}</{field}>"), &replacement, 1);
            let fixture = fixture_with_catalog(&xml);
            let before = tree_snapshot(&fixture.source);
            for attribute in attribute_views(&fixture) {
                assert!(
                    attribute["props"].get(field).is_none(),
                    "damaged {field} must not be published: {replacement}"
                );
                assert!(
                    has_limit(&attribute, field),
                    "row and detail must name damaged {field}: {replacement}"
                );
                for (independent, expected) in ENUMS.into_iter().filter(|(name, _)| *name != field)
                {
                    assert_eq!(
                        attribute["props"][independent], expected,
                        "damage to {field} must preserve {independent}"
                    );
                    assert!(
                        !has_limit(&attribute, independent),
                        "damage to {field} must not be assigned to {independent}"
                    );
                }
                assert_independent_facts(&fixture, &attribute);
            }
            let owner = read(&fixture, "main:Catalog.Items");
            assert!(
                has_limit(&owner, field),
                "Catalog must name the damaged {field}: {replacement}"
            );
            assert_eq!(tree_snapshot(&fixture.source), before);
        }
    }
}

#[test]
fn catalog_attribute_profile_does_not_cross_owner_section_or_header_contexts() {
    let facts = "<Indexing>IndexWithAdditionalOrder</Indexing><FullTextSearch>DontUse</FullTextSearch><DataHistory>DontUse</DataHistory>";
    let xml = CATALOG
        .replacen("<Owners/>", &format!("{facts}<Owners/>"), 1)
        .replacen("<LineNumberLength>5</LineNumberLength>", &format!("{facts}<LineNumberLength>5</LineNumberLength>"), 1)
        .replacen("<Name>Header</Name>", &format!("<Name>Header</Name>{facts}"), 1)
        .replacen("<Name>КодЯзыка</Name>", "<Name>КодЯзыка</Name><Secret>private</Secret><LineNumberLength>7</LineNumberLength><Use>ForFolder</Use>", 1);
    let fixture = fixture_with_catalog(&xml);
    let before = tree_snapshot(&fixture.source);
    let owner = read(&fixture, "main:Catalog.Items");
    assert!(
        owner["props"].get("Indexing").is_none(),
        "the Attribute profile must not publish Indexing on the Catalog owner"
    );
    for field in ["FullTextSearch", "DataHistory"] {
        assert_eq!(
            owner["props"][field], "DontUse",
            "the Catalog must preserve its own {field}, independently of the Attribute's Use"
        );
    }
    for at in [
        SECTION.to_string(),
        "main:Catalog.Items.Attribute.Header".to_string(),
    ] {
        let node = read(&fixture, &at);
        for (field, _) in ENUMS {
            assert!(
                node["props"].get(field).is_none(),
                "the Attribute-only enum profile must not publish {field} at {at}"
            );
        }
    }
    for attribute in attribute_views(&fixture) {
        for (field, expected) in ENUMS {
            assert_eq!(attribute["props"][field], expected);
        }
        for field in ["Secret", "LineNumberLength", "Use"] {
            assert!(
                attribute["props"].get(field).is_none(),
                "the Attribute profile must not publish {field}"
            );
        }
    }
    assert_eq!(tree_snapshot(&fixture.source), before);
}

#[test]
fn catalog_attribute_profile_requires_a_catalog_in_an_ordinary_configuration() {
    let fixture = fixture_with_catalog(CATALOG);
    let document_xml = CATALOG
        .replace("<Catalog uuid=", "<Document uuid=")
        .replace("</Catalog>", "</Document>")
        .replace("<Name>Items</Name>", "<Name>Order</Name>");
    write(&fixture.source.join("Documents/Order.xml"), &document_xml);
    let document = read(
        &fixture,
        "main:Document.Order.TabularSection.Представления.Attribute.КодЯзыка",
    );
    for (field, _) in ENUMS {
        assert!(
            document["props"].get(field).is_none(),
            "the Catalog-only profile must not publish Document {field}"
        );
    }
    assert_string_type(&document, 10);
    let extension = fixture
        .extension_view_service()
        .view(ViewRequest::new(LANGUAGE).unwrap());
    assert!(extension.ok, "the extension member must remain addressable");
    let extension = extension.data.unwrap();
    for (field, _) in ENUMS {
        assert!(
            extension["props"].get(field).is_none(),
            "the Configuration-only profile must not publish CFE {field}"
        );
    }
    assert_string_type(&extension, 10);
    let unknown = fixture
        .view_service()
        .view(ViewRequest::new(&LANGUAGE.replace("main:", "unknown:")).unwrap());
    assert!(
        !unknown.ok,
        "an undeclared source must not reuse the main profile"
    );
}

#[test]
fn catalog_attribute_profile_requires_the_literal_supported_xml_version() {
    for declaration in [
        " version=\"2.19\"",
        " version=\"2.21\"",
        "",
        " version=\"2&#x2e;20\"",
        " version=\"2.2&#48;\"",
    ] {
        let xml = CATALOG.replace(" version=\"2.20\"", declaration);
        let fixture = fixture_with_catalog(&xml);
        let before = tree_snapshot(&fixture.source);
        for attribute in attribute_views(&fixture) {
            for (field, _) in ENUMS {
                assert!(
                    attribute["props"].get(field).is_none(),
                    "{field} requires literal XML 2.20: {declaration}"
                );
            }
            assert_eq!(attribute["props"]["required"], false);
            assert_eq!(attribute["props"]["synonym"], "Код языка");
            assert_string_type(&attribute, 10);
        }
        let sibling = read(&fixture, SPELLING);
        assert_string_type(&sibling, 200);
        assert_eq!(tree_snapshot(&fixture.source), before);
    }
}
