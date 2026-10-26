//! #2161: the composition of a defined type is read at the type's own address;
//! an attribute of that type names it in its `type`, by the metadata path that
//! `apply` accepts. The qualified address is `<source set>:<metadataPath>`.
use super::{write, RealReaderFixture, ViewRequest};
use serde_json::{json, Value};

fn read(fixture: &RealReaderFixture, at: &str) -> Value {
    let view = fixture.view_service().view(ViewRequest::new(at).unwrap());
    assert!(view.ok, "{at}: {view:?}");
    view.data.unwrap()
}

#[test]
fn a_defined_type_attribute_names_the_type_whose_node_holds_the_composition() {
    let fixture = RealReaderFixture::new();
    let config_path = fixture.source.join("Configuration.xml");
    let config = std::fs::read_to_string(&config_path).unwrap();
    write(
        &config_path,
        &config.replacen(
            "<Catalog>Items</Catalog>",
            "<Catalog>Items</Catalog>\n\t\t\t<DefinedType>Money</DefinedType>",
            1,
        ),
    );
    write(
        &fixture.source.join("DefinedTypes/Money.xml"),
        r#"<?xml version="1.0" encoding="UTF-8"?><MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:xs="http://www.w3.org/2001/XMLSchema" version="2.20"><DefinedType uuid="22222222-2222-4222-8222-222222222222"><Properties><Name>Money</Name><Synonym/><Comment/><Type><v8:Type>xs:decimal</v8:Type><v8:NumberQualifiers><v8:Digits>15</v8:Digits><v8:FractionDigits>2</v8:FractionDigits><v8:AllowedSign>Any</v8:AllowedSign></v8:NumberQualifiers></Type></Properties></DefinedType></MetaDataObject>"#,
    );
    // The platform writes an attribute of a defined type as `v8:TypeSet`.
    let catalog_path = fixture.source.join("Catalogs/Items.xml");
    let catalog = std::fs::read_to_string(&catalog_path).unwrap();
    assert!(catalog.contains("<v8:Type>xs:decimal</v8:Type>"));
    write(
        &catalog_path,
        &catalog.replacen(
            "<v8:Type>xs:decimal</v8:Type>",
            r#"<v8:TypeSet xmlns:cfg="http://v8.1c.ru/8.1/data/enterprise/current-config">cfg:DefinedType.Money</v8:TypeSet>"#,
            1,
        ),
    );
    write(
        &fixture
            .source
            .join("Catalogs/Items/Forms/ItemForm/Ext/Form.xml"),
        r#"<?xml version="1.0" encoding="UTF-8"?><Form xmlns="http://v8.1c.ru/8.3/xcf/logform" xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:cfg="http://v8.1c.ru/8.1/data/enterprise/current-config" version="2.20"><ChildItems/><Attributes><Attribute name="Сумма" id="1"><Type><v8:TypeSet>cfg:DefinedType.Money</v8:TypeSet></Type></Attribute></Attributes></Form>"#,
    );

    let defined = read(&fixture, "main:DefinedType.Money");
    assert_eq!(defined["kind"], "DefinedType");
    assert_eq!(
        defined["props"]["type"]["variants"],
        json!([{"kind": "number", "digits": 15, "fraction": 2, "sign": "any"}])
    );

    let object_attribute = read(
        &fixture,
        "main:Catalog.Items.TabularSection.Lines.Attribute.Quantity",
    );
    assert_eq!(
        object_attribute["props"]["type"]["variants"],
        json!([{"kind": "definedType", "metadataPath": "DefinedType.Money"}])
    );
    let form_attribute = read(&fixture, "main:Catalog.Items.Form.ItemForm.Attribute.Сумма");
    assert_eq!(form_attribute["props"]["type"], "DefinedType.Money");

    // `<source set>:<metadataPath>` is the type's own node.
    let source_set = object_attribute["at"]
        .as_str()
        .unwrap()
        .split_once(':')
        .unwrap()
        .0;
    let named = format!(
        "{source_set}:{}",
        object_attribute["props"]["type"]["variants"][0]["metadataPath"]
            .as_str()
            .unwrap()
    );
    assert_eq!(read(&fixture, &named), defined);
}
