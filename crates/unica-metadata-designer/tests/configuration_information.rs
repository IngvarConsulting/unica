use serde_json::json;
use unica_metadata::support_state::{ConfigurationSupportData, ConfigurationSupportState};
use unica_metadata_designer::configuration::parse_cf_info_xml;

#[test]
fn configuration_information_preserves_flags_and_all_translations() {
    let xml = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core" version="2.20"><Configuration><Properties><Name>Example</Name><Version>1.2</Version><UseManagedFormInOrdinaryApplication>true</UseManagedFormInOrdinaryApplication><UseOrdinaryFormInManagedApplication>false</UseOrdinaryFormInManagedApplication><BriefInformation><v8:item><v8:lang>ru</v8:lang><v8:content>Кратко</v8:content></v8:item><v8:item><v8:lang>en</v8:lang><v8:content>Brief</v8:content></v8:item></BriefInformation><DetailedInformation><v8:item><v8:lang>ru</v8:lang><v8:content>Подробно</v8:content></v8:item></DetailedInformation><Copyright><v8:item><v8:lang>ru</v8:lang><v8:content>Автор</v8:content></v8:item></Copyright><VendorInformationAddress><v8:item><v8:lang>en</v8:lang><v8:content>https://example.org/vendor</v8:content></v8:item></VendorInformationAddress><ConfigurationInformationAddress><v8:item><v8:lang>en</v8:lang><v8:content>https://example.org/configuration</v8:content></v8:item></ConfigurationInformationAddress></Properties><ChildObjects/></Configuration></MetaDataObject>"#;
    let parsed = parse_cf_info_xml(
        xml,
        ConfigurationSupportData {
            state: ConfigurationSupportState::NotSupported,
            editing_enabled: None,
            objects: None,
        },
        None,
        None,
        None,
    )
    .unwrap();
    let value = serde_json::to_value(parsed.data).unwrap();
    let props = &value["properties"];
    assert_eq!(props["useManagedFormInOrdinaryApplication"], true);
    assert_eq!(props["useOrdinaryFormInManagedApplication"], false);
    assert_eq!(
        props["briefInformation"],
        json!([
            {"language":"ru","content":"Кратко"}, {"language":"en","content":"Brief"}
        ])
    );
    for (field, language, content) in [
        ("detailedInformation", "ru", "Подробно"),
        ("copyright", "ru", "Автор"),
        (
            "vendorInformationAddress",
            "en",
            "https://example.org/vendor",
        ),
        (
            "configurationInformationAddress",
            "en",
            "https://example.org/configuration",
        ),
    ] {
        assert_eq!(
            props[field],
            json!([{"language":language,"content":content}])
        );
    }
    assert_eq!(value["version"], "1.2");
}
