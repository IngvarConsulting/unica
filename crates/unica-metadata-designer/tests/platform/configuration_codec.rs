use super::*;
const ROOT_DOCUMENT: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<CommandInterface xmlns="http://v8.1c.ru/8.3/xcf/extrnprops" version="2.20">
	<SubsystemsOrder>
		<Subsystem>Subsystem.Планирование</Subsystem>
		<Subsystem>Subsystem.Продажи</Subsystem>
	</SubsystemsOrder>
</CommandInterface>"#;
#[test]
fn legacy_home_page_wrapper_keeps_well_formed_wrong_root_compatibility() {
    let wrong_root = r#"<NotHomePage xmlns="http://v8.1c.ru/8.3/xcf/extrnprops" version="2.20"/>"#;
    let legacy = parse_cf_home_page_xml(wrong_root)
        .expect("the V12 wrapper historically treated any well-formed root as an empty layout");
    assert!(legacy.template.is_empty());
    assert!(legacy.left.is_empty());
    assert!(legacy.right.is_empty());
    assert!(parse_cf_home_page_xml_strict(wrong_root).is_err());
    assert!(parse_cf_home_page_xml("<broken>").is_none());
}

#[test]
fn cf_edit_child_object_parser_rejects_unsafe_metadata_names() {
    for value in ["Catalog.../Outside", "Catalog.Bad:Name"] {
        let error = cf_edit_parse_child_object(value).unwrap_err();
        assert!(error.contains("XML NCName"), "{value}: {error}");
        assert!(error.contains("path component"), "{value}: {error}");
    }
    assert!(cf_edit_parse_child_object("Catalog.")
        .unwrap_err()
        .contains("expected 'Type.Name'"));
    assert_eq!(
        cf_edit_parse_child_object("Catalog.Товары").unwrap(),
        ("Catalog", "Товары")
    );
}

#[test]
fn the_root_interface_is_the_declared_order_of_top_level_subsystems() {
    let parsed = parse_cf_command_interface_xml(ROOT_DOCUMENT).expect("документ разбирается");
    assert_eq!(
        parsed.subsystem_order,
        vec!["Subsystem.Планирование", "Subsystem.Продажи"],
        "порядок содержателен: он задаёт, как разделы встают в интерфейсе"
    );
}

#[test]
fn a_document_that_is_not_a_command_interface_is_refused_by_name() {
    let error = parse_cf_command_interface_xml(
        r#"<?xml version="1.0"?><MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses"/>"#,
    )
    .expect_err("чужой корень не принимается");
    assert!(
        error.contains("no CommandInterface root"),
        "отказ обязан назвать, чего не хватило: {error}"
    );
}

#[test]
fn an_interface_without_an_order_section_is_empty_not_a_failure() {
    let parsed = parse_cf_command_interface_xml(
        r#"<CommandInterface xmlns="http://v8.1c.ru/8.3/xcf/extrnprops"/>"#,
    )
    .expect("пустой интерфейс — законное состояние");
    assert!(parsed.subsystem_order.is_empty());
}

#[test]
fn configuration_information_preserves_explicit_flags_and_localized_values() {
    let xml = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core" version="2.20"><Configuration><Properties>
<Name>Information</Name><Version>1.2.3</Version>
<UseManagedFormInOrdinaryApplication>false</UseManagedFormInOrdinaryApplication>
<UseOrdinaryFormInManagedApplication>true</UseOrdinaryFormInManagedApplication>
<BriefInformation><v8:item><v8:lang>ru</v8:lang><v8:content>Кратко</v8:content></v8:item><v8:item><v8:lang>en</v8:lang><v8:content>Brief</v8:content></v8:item></BriefInformation>
<DetailedInformation><v8:item><v8:lang>ru</v8:lang><v8:content>Подробно</v8:content></v8:item></DetailedInformation>
<Copyright><v8:item><v8:lang>ru</v8:lang><v8:content>Автор 2026</v8:content></v8:item></Copyright>
<VendorInformationAddress><v8:item><v8:lang>en</v8:lang><v8:content>https://vendor.example</v8:content></v8:item></VendorInformationAddress>
<ConfigurationInformationAddress><v8:item><v8:lang>ru</v8:lang><v8:content>https://product.example</v8:content></v8:item></ConfigurationInformationAddress>
</Properties><ChildObjects/></Configuration></MetaDataObject>"#;
    let parsed = parse_cf_info_xml(
        xml,
        ConfigurationSupportData {
            state: unica_metadata::support_state::ConfigurationSupportState::NotSupported,
            editing_enabled: None,
            objects: None,
        },
        None,
        None,
        None,
    )
    .unwrap();
    let data = serde_json::to_value(parsed.data).unwrap();
    let properties = &data["properties"];
    assert_eq!(properties["useManagedFormInOrdinaryApplication"], false);
    assert_eq!(properties["useOrdinaryFormInManagedApplication"], true);
    assert_eq!(
        properties["briefInformation"],
        serde_json::json!([
            {"language":"ru", "content":"Кратко"}, {"language":"en", "content":"Brief"}
        ])
    );
    for (key, language, content) in [
        ("detailedInformation", "ru", "Подробно"),
        ("copyright", "ru", "Автор 2026"),
        ("vendorInformationAddress", "en", "https://vendor.example"),
        (
            "configurationInformationAddress",
            "ru",
            "https://product.example",
        ),
    ] {
        assert_eq!(
            properties[key],
            serde_json::json!([{"language":language,"content":content}])
        );
    }
    assert_eq!(data["version"], "1.2.3");
}

#[test]
fn configuration_information_rejects_duplicate_languages_and_malformed_localization() {
    for value in [
        "<v8:item><v8:lang>ru</v8:lang><v8:content>One</v8:content></v8:item><v8:item><v8:lang>ru</v8:lang><v8:content>Two</v8:content></v8:item>",
        "<v8:item><v8:lang>ru</v8:lang></v8:item>",
        "<v8:item><v8:lang>ru</v8:lang><v8:content>One<Nested/></v8:content></v8:item>",
        "bare text",
        "<item xmlns=\"urn:foreign\"><lang>ru</lang><content>Text</content></item>",
        "<v8:item><v8:lang>ru</v8:lang><v8:content>One</v8:content><v8:content>Two</v8:content></v8:item>",
    ] {
        let xml = format!(r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core" version="2.20"><Configuration><Properties><Name>Information</Name><BriefInformation>{value}</BriefInformation><UseManagedFormInOrdinaryApplication>false</UseManagedFormInOrdinaryApplication></Properties><ChildObjects/></Configuration></MetaDataObject>"#);
        let parsed = parse_cf_info_xml(&xml, ConfigurationSupportData {
            state: unica_metadata::support_state::ConfigurationSupportState::NotSupported,
            editing_enabled: None,
            objects: None,
        }, None, None, None).unwrap();
        let data = serde_json::to_value(parsed.data).unwrap();
        assert!(data["properties"]["briefInformation"].is_null());
        assert_eq!(data["properties"]["useManagedFormInOrdinaryApplication"], false);
        assert_eq!(data["propertyLimits"].as_array().unwrap().len(), 1);
        assert!(data["propertyLimits"][0].as_str().unwrap().contains("BriefInformation"));
    }
}
