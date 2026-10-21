use super::{write, RealReaderFixture, ViewRequest};
use serde_json::{json, Value};
use unica_workspace::test_support::tree_snapshot;

const CATALOG: &str = include_str!(
    "../../../../../../tests/fixtures/acceptance/catalog-attribute-tooltip/src/Catalogs/Items.xml"
);
const SECTION: &str = "main:Catalog.Items.TabularSection.Представления";
const LANGUAGE: &str = "main:Catalog.Items.TabularSection.Представления.Attribute.КодЯзыка";
const SPELLING: &str = "main:Catalog.Items.TabularSection.Представления.Attribute.ПараметрыПрописи";

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

fn member_views(fixture: &RealReaderFixture, at: &str) -> [Value; 2] {
    let collection_at = at.rsplit_once('.').unwrap().0;
    let collection = read(fixture, collection_at);
    let row = collection["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["at"] == at)
        .unwrap_or_else(|| panic!("member must remain in {collection_at}: {at}"))
        .clone();
    let detail = read(fixture, row["at"].as_str().unwrap());
    [row, detail]
}

fn localized(node: &Value) -> Value {
    serde_json::from_str(
        node["props"]["ToolTip"]
            .as_str()
            .expect("public ToolTip must preserve localized XML as compact JSON"),
    )
    .expect("public ToolTip must contain valid compact JSON")
}

fn expected_tooltip() -> Value {
    json!({"kind": "localizedString", "values": [
        {"language": "ru", "content": "Настройки представления суммы прописью"},
        {"language": "en", "content": "Amount spelling settings & presentation"}
    ]})
}

fn has_tooltip_limit(node: &Value) -> bool {
    node["limits"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .any(|limit| limit.contains("ToolTip"))
}

fn assert_type(node: &Value, length: u64) {
    let observed: Value = serde_json::from_str(node["props"]["type"].as_str().unwrap()).unwrap();
    assert_eq!(observed["variants"][0]["kind"], "string");
    assert_eq!(observed["variants"][0]["length"], length);
    assert_eq!(observed["variants"][0]["allowedLength"], "variable");
}

fn assert_spelling_facts(node: &Value) {
    assert_eq!(node["at"], SPELLING);
    assert_eq!(node["props"]["synonym"], "Параметры прописи");
    assert_eq!(node["props"]["required"], false);
    assert_eq!(node["props"]["Indexing"], "DontIndex");
    assert_eq!(node["props"]["FullTextSearch"], "Use");
    assert_eq!(node["props"]["DataHistory"], "Use");
    assert_type(node, 200);
}

fn replace_spelling_tooltip(xml: &str, replacement: &str) -> String {
    let start = xml[xml.find("<Name>ПараметрыПрописи</Name>").unwrap()..]
        .find("<ToolTip>")
        .unwrap()
        + xml.find("<Name>ПараметрыПрописи</Name>").unwrap();
    let end = start + xml[start..].find("</ToolTip>").unwrap() + "</ToolTip>".len();
    let mut result = xml.to_string();
    result.replace_range(start..end, replacement);
    result
}

#[test]
fn catalog_attribute_tooltip_completes_six_facts_in_public_collection_and_detail_without_writes() {
    let fixture = fixture_with_catalog(CATALOG);
    let before = tree_snapshot(&fixture.source);
    for spelling in member_views(&fixture, SPELLING) {
        assert_eq!(
            localized(&spelling),
            expected_tooltip(),
            "public ToolTip must preserve every language and content"
        );
        assert_spelling_facts(&spelling);
        assert!(!has_tooltip_limit(&spelling));
    }
    for language in member_views(&fixture, LANGUAGE) {
        assert_eq!(language["props"]["Indexing"], "Index");
        assert_eq!(language["props"]["FullTextSearch"], "Use");
        assert_eq!(language["props"]["DataHistory"], "Use");
        assert_eq!(language["props"]["synonym"], "Код языка");
        assert_eq!(language["props"]["required"], false);
        assert_type(&language, 10);
        assert_eq!(
            localized(&language),
            json!({"kind": "localizedString", "values": []})
        );
    }
    for section in member_views(&fixture, SECTION) {
        assert_eq!(section["props"]["LineNumberLength"], 5);
        assert_eq!(section["props"]["Use"], "ForItem");
        assert_eq!(section["props"]["synonym"], "Представления");
        assert_eq!(section["props"]["required"], false);
        let attributes = section["branches"]
            .as_array()
            .unwrap()
            .iter()
            .find(|branch| branch["at"] == format!("{SECTION}.Attribute"))
            .expect("the section must retain its separate application attribute branch");
        assert_eq!(attributes["count"], 2);
    }
    let header = read(&fixture, "main:Catalog.Items.Attribute.Header");
    assert_type(&header, 40);
    assert_eq!(tree_snapshot(&fixture.source), before);
}

#[test]
fn catalog_attribute_tooltip_absence_and_explicit_empty_values_remain_distinct() {
    for (replacement, expected) in [
        ("", None),
        (
            "<ToolTip/>",
            Some(json!({"kind": "localizedString", "values": []})),
        ),
        (
            "<ToolTip></ToolTip>",
            Some(json!({"kind": "localizedString", "values": []})),
        ),
        (
            "<ToolTip> \n\t </ToolTip>",
            Some(json!({"kind": "localizedString", "values": []})),
        ),
        (
            "<ToolTip><v8:item><v8:lang>ru</v8:lang><v8:content/></v8:item></ToolTip>",
            Some(json!({"kind": "localizedString", "values": [{"language": "ru", "content": ""}]})),
        ),
    ] {
        let fixture = fixture_with_catalog(&replace_spelling_tooltip(CATALOG, replacement));
        let before = tree_snapshot(&fixture.source);
        for node in member_views(&fixture, SPELLING) {
            if let Some(expected) = &expected {
                assert_eq!(
                    localized(&node),
                    *expected,
                    "explicit empty value: {replacement}"
                );
            } else {
                assert!(
                    node["props"].get("ToolTip").is_none(),
                    "absence must not inherit another ToolTip"
                );
            }
            assert!(
                !has_tooltip_limit(&node),
                "optional absence or valid emptiness is not damage"
            );
            assert_spelling_facts(&node);
        }
        assert!(!has_tooltip_limit(&read(&fixture, "main:Catalog.Items")));
        assert_eq!(tree_snapshot(&fixture.source), before);
    }
}

#[test]
fn catalog_attribute_tooltip_damage_is_local_in_owner_row_and_detail() {
    let item =
        "<v8:item><v8:lang>ru</v8:lang><v8:content>Локальная подсказка</v8:content></v8:item>";
    for replacement in [
        "<ToolTip>broken</ToolTip>".to_string(),
        "<ToolTip xsi:nil=\"true\"/>".to_string(),
        format!("<ToolTip>{item}</ToolTip><ToolTip>{item}</ToolTip>"),
        format!("<ToolTip>{item}{item}</ToolTip>"),
        format!("<ToolTip>broken{item}</ToolTip>"),
        format!("<ToolTip xmlns=\"urn:foreign\">{item}</ToolTip>"),
        format!("<ToolTip>{item}</ToolTip><ToolTip xmlns=\"urn:foreign\"/>"),
        format!("<ToolTip extra=\"true\">{item}</ToolTip>"),
        "<ToolTip><v8:item><v8:lang>ru</v8:lang></v8:item></ToolTip>".to_string(),
        "<ToolTip><v8:item><v8:content>Текст</v8:content></v8:item></ToolTip>".to_string(),
        format!(
            "<ToolTip>{}</ToolTip>",
            item.replace(
                "<v8:lang>ru</v8:lang>",
                "<v8:lang>ru</v8:lang><v8:lang>en</v8:lang>"
            )
        ),
        format!(
            "<ToolTip>{}</ToolTip>",
            item.replace("<v8:content>", "<v8:content extra=\"true\">")
        ),
        format!(
            "<ToolTip>{}</ToolTip>",
            item.replace(
                "<v8:lang>ru</v8:lang>",
                "<v8:lang><v8:content>ru</v8:content></v8:lang>"
            )
        ),
        format!(
            "<ToolTip>{}</ToolTip>",
            item.replace("<v8:item>", "<v8:item xmlns:v8=\"urn:foreign\">")
        ),
    ] {
        let fixture = fixture_with_catalog(&replace_spelling_tooltip(CATALOG, &replacement));
        let before = tree_snapshot(&fixture.source);
        for node in member_views(&fixture, SPELLING) {
            assert!(
                node["props"].get("ToolTip").is_none(),
                "damaged ToolTip: {replacement}"
            );
            assert!(
                has_tooltip_limit(&node),
                "ToolTip damage must reach row and detail: {replacement}"
            );
            assert_spelling_facts(&node);
            for field in ["Indexing", "FullTextSearch", "DataHistory"] {
                assert!(
                    !node["limits"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .any(|limit| limit.as_str().unwrap_or_default().contains(field)),
                    "ToolTip damage must preserve {field}"
                );
            }
        }
        assert!(
            has_tooltip_limit(&read(&fixture, "main:Catalog.Items")),
            "owner must name local ToolTip damage: {replacement}"
        );
        let language = read(&fixture, LANGUAGE);
        assert_eq!(language["props"]["Indexing"], "Index");
        assert_eq!(
            localized(&language),
            json!({"kind": "localizedString", "values": []})
        );
        assert!(
            !has_tooltip_limit(&language),
            "a sibling must not inherit damage"
        );
        assert_type(&language, 10);
        let section = read(&fixture, SECTION);
        assert_eq!(section["props"]["LineNumberLength"], 5);
        assert_eq!(section["props"]["Use"], "ForItem");
        assert_eq!(tree_snapshot(&fixture.source), before);
    }
}

#[test]
fn catalog_attribute_tooltip_never_inherits_section_header_or_sibling_values() {
    let xml = replace_spelling_tooltip(CATALOG, "")
        .replacen("<ToolTip/>", "<ToolTip><v8:item><v8:lang>ru</v8:lang><v8:content>Подсказка части</v8:content></v8:item></ToolTip>", 1)
        .replacen("<Name>Header</Name>", "<Name>Header</Name><ToolTip><v8:item><v8:lang>ru</v8:lang><v8:content>Подсказка шапки</v8:content></v8:item></ToolTip>", 1)
        .replacen("<ToolTip/>", "<ToolTip><v8:item><v8:lang>ru</v8:lang><v8:content>Подсказка соседа</v8:content></v8:item></ToolTip>", 1);
    let fixture = fixture_with_catalog(&xml);
    let before = tree_snapshot(&fixture.source);
    for node in member_views(&fixture, SPELLING) {
        assert!(node["props"].get("ToolTip").is_none());
        assert!(!has_tooltip_limit(&node));
        assert_spelling_facts(&node);
    }
    for at in [SECTION, "main:Catalog.Items.Attribute.Header"] {
        assert!(
            read(&fixture, at)["props"].get("ToolTip").is_none(),
            "the attribute profile must not cross context at {at}"
        );
    }
    assert_eq!(
        localized(&read(&fixture, LANGUAGE)),
        json!({"kind": "localizedString", "values": [{"language": "ru", "content": "Подсказка соседа"}]})
    );
    assert_eq!(tree_snapshot(&fixture.source), before);
}

#[test]
fn catalog_attribute_tooltip_requires_proved_catalog_configuration_and_literal_xml_220() {
    for version in [
        " version=\"2.19\"",
        " version=\"2.21\"",
        "",
        " version=\"2&#x2e;20\"",
    ] {
        let fixture = fixture_with_catalog(&CATALOG.replace(" version=\"2.20\"", version));
        for node in member_views(&fixture, SPELLING) {
            assert!(
                node["props"].get("ToolTip").is_none(),
                "unsupported literal version: {version}"
            );
            assert_type(&node, 200);
        }
    }
    let fixture = fixture_with_catalog(CATALOG);
    let document = CATALOG
        .replace("<Catalog uuid=", "<Document uuid=")
        .replace("</Catalog>", "</Document>")
        .replace("<Name>Items</Name>", "<Name>Order</Name>");
    write(&fixture.source.join("Documents/Order.xml"), &document);
    let node = read(
        &fixture,
        &SPELLING.replace("Catalog.Items", "Document.Order"),
    );
    assert!(node["props"].get("ToolTip").is_none());
    assert_type(&node, 200);
    let extension = fixture
        .extension_view_service()
        .view(ViewRequest::new(SPELLING).unwrap());
    assert!(extension.ok);
    let node = extension.data.unwrap();
    assert!(node["props"].get("ToolTip").is_none());
    assert_type(&node, 200);
    assert!(
        !fixture
            .view_service()
            .view(ViewRequest::new(&SPELLING.replace("main:", "unknown:")).unwrap())
            .ok
    );
}

#[test]
fn catalog_attribute_tooltip_compact_limit_preserves_independent_facts() {
    let replacement = format!(
        "<ToolTip><v8:item><v8:lang>ru</v8:lang><v8:content>{}</v8:content></v8:item></ToolTip>",
        "я".repeat(2048)
    );
    let fixture = fixture_with_catalog(&replace_spelling_tooltip(CATALOG, &replacement));
    let before = tree_snapshot(&fixture.source);
    for node in member_views(&fixture, SPELLING) {
        assert!(
            node["props"].get("ToolTip").is_none(),
            "oversized compact JSON must stay outside props"
        );
        assert!(
            has_tooltip_limit(&node),
            "omitted observed ToolTip must have a visible limit"
        );
        assert_spelling_facts(&node);
    }
    assert_eq!(tree_snapshot(&fixture.source), before);
}
