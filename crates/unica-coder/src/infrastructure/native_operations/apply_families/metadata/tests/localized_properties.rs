use super::*;
use serde_json::Value;

const CATALOG: &str = include_str!(
    "../../../../../../../../tests/fixtures/acceptance/catalog-attribute-tooltip/src/Catalogs/Items.xml"
);
const ATTRIBUTE: &str =
    "main:Catalog.Items.TabularSection.Представления.Attribute.ПараметрыПрописи";

fn fixture() -> MetadataFixture {
    let configuration =
        RU_CONFIGURATION_XML.replace("</ChildObjects>", "<Catalog>Items</Catalog></ChildObjects>");
    MetadataFixture::with_files(&[
        ("Configuration.xml", &configuration),
        ("Languages/Русский.xml", &language_xml("Русский", "ru")),
        ("Documents/Order.xml", ORDER_XML),
        ("Catalogs/Items.xml", CATALOG),
    ])
}

#[test]
fn localized_attribute_changes_preserve_other_languages_and_plan_the_right_field() {
    let fixture = fixture();
    let staged = plan_all(
        &fixture,
        &[(
            "attribute.set",
            json!({"at":ATTRIBUTE,"values":{
                "synonymTranslations":[
                    {"language":"ru","content":"Прописью"},
                    {"language":"en","content":"In words"}
                ],
                "tooltipTranslations":[{"language":"en","content":"Help & examples"}]
            }}),
        )],
    )
    .unwrap();
    let texts = staged_texts(&staged);
    let xml = &texts[Path::new("Catalogs/Items.xml")];
    assert!(xml.contains("<v8:content>Прописью</v8:content>"));
    assert!(xml.contains("<v8:content>In words</v8:content>"));
    assert!(xml.contains("<v8:content>Help &amp; examples</v8:content>"));
    assert!(xml.contains("<v8:content>Настройки представления суммы прописью</v8:content>"));
    assert_eq!(
        std::fs::read_to_string(fixture.binding.source_root().join("Catalogs/Items.xml")).unwrap(),
        CATALOG
    );
}

#[test]
fn localized_object_changes_see_secondary_edits_and_keep_legacy_string_writes() {
    let fixture = fixture();
    let staged = plan_all(&fixture, &[
        ("props.set", json!({"at":"main:Document.Order","values":{"Synonym":"Старое имя"}})),
        ("props.set", json!({"at":"main:Document.Order","values":{"Synonym":[{"language":"en","content":"Order & delivery"}]}})),
    ]).unwrap();
    let texts = staged_texts(&staged);
    let xml = &texts[Path::new("Documents/Order.xml")];
    assert!(xml.contains("<v8:content>Старое имя</v8:content>"));
    assert!(xml.contains("<v8:content>Order &amp; delivery</v8:content>"));
}

#[test]
fn localized_attribute_rejects_duplicate_languages_and_competing_synonyms() {
    let fixture = fixture();
    let row = json!({"language":"en","content":"Text"});
    for values in [
        json!({"synonymTranslations":[row, row]}),
        json!({"synonym":"Scalar","synonymTranslations":[row]}),
        json!({"synonymTranslations":Vec::<Value>::new()}),
    ] {
        assert!(
            parse_metadata_plan_operation(
                "attribute.set",
                &json!({"at":ATTRIBUTE,"values":values}),
                0,
                &fixture.binding,
            )
            .is_err(),
            "{values}"
        );
    }
}
