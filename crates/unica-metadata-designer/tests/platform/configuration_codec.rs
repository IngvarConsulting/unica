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

/// Pure image-generation evidence; this does not exercise a workspace publisher.
/// rule: INV.SOURCE.WRITABLE-PROFILE
#[test]
fn configuration_init_images_round_trip_through_info_with_registered_language_identity() {
    let args = Map::from_iter([
        ("vendor".to_string(), serde_json::json!("Synthetic & Co")),
        ("version".to_string(), serde_json::json!("1.2.3")),
    ]);
    let images = cf_init_images(
        &args,
        "SyntheticConfiguration",
        "Synthetic <configuration>",
        "Version8_3_27",
        || "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa".to_string(),
    );
    let info = parse_cf_info_xml(
        &images.configuration,
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
    assert_eq!(info.data.name, "SyntheticConfiguration");
    assert_eq!(
        info.data.synonym.as_deref(),
        Some("Synthetic <configuration>")
    );
    assert_eq!(info.data.vendor.as_deref(), Some("Synthetic & Co"));
    assert_eq!(info.data.version.as_deref(), Some("1.2.3"));
    assert_eq!(info.data.format, "2.20");
    assert_eq!(
        info.data.properties.compatibility_mode.as_deref(),
        Some("Version8_3_27")
    );
    assert!(info.data.extension_purpose.is_none());
    assert_eq!(info.data.total_objects, 1);

    let configuration = Document::parse(&images.configuration).unwrap();
    let language = Document::parse(&images.language).unwrap();
    for document in [&configuration, &language] {
        let root = document.root_element();
        assert!(root.has_tag_name((CF_MD_NS, "MetaDataObject")));
        assert_eq!(root.attribute("version"), Some("2.20"));
        assert!(matches!(
            classify_root_version(root.attribute("version")).unwrap(),
            FormatCompatibility::Supported { .. }
        ));
    }
    let cfg = configuration
        .root_element()
        .children()
        .find(|node| node.has_tag_name((CF_MD_NS, "Configuration")))
        .unwrap();
    let lang = language
        .root_element()
        .children()
        .find(|node| node.has_tag_name((CF_MD_NS, "Language")))
        .unwrap();
    let language_props = lang
        .children()
        .find(|node| node.has_tag_name((CF_MD_NS, "Properties")))
        .unwrap();
    let language_name = cf_prop_text(language_props, "Name");
    assert_eq!(language_name, "Русский");
    assert_eq!(cf_prop_text(language_props, "LanguageCode"), "ru");
    assert_eq!(
        info.data.properties.default_language,
        Some(format!("Language.{language_name}"))
    );
    assert_eq!(info.registered_objects.len(), 1);
    assert_eq!(info.registered_objects[0].kind, "Language");
    assert_eq!(info.registered_objects[0].name, language_name);
    assert_eq!(
        cfg.attribute("uuid"),
        Some("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa")
    );
    let language_uuid = uuid::Uuid::parse_str(lang.attribute("uuid").unwrap()).unwrap();
    assert_ne!(language_uuid, uuid::Uuid::nil());
    assert_ne!(cfg.attribute("uuid"), lang.attribute("uuid"));
    let props = cfg
        .children()
        .find(|node| node.has_tag_name((CF_MD_NS, "Properties")))
        .unwrap();
    let roles = props
        .children()
        .find(|node| node.has_tag_name((CF_MD_NS, "DefaultRoles")))
        .unwrap();
    assert_eq!(roles.children().filter(|node| node.is_element()).count(), 0);

    let panels = Document::parse(&images.client_application_interface).unwrap();
    assert!(panels
        .root_element()
        .has_tag_name((CF_CAI_NS, "ClientApplicationInterface")));
    assert!(panels.root_element().attribute("version").is_none());
    assert_eq!(
        unica_source_designer::roots::platform_xml_publication_policy(
            CF_CAI_NS,
            "ClientApplicationInterface"
        ),
        Some(unica_source_designer::roots::PlatformXmlPublicationPolicy::Versionless)
    );
    assert_eq!(
        panels
            .root_element()
            .attribute(("http://www.w3.org/2001/XMLSchema-instance", "type")),
        Some("InterfaceLayouter")
    );
    let definitions = panels
        .root_element()
        .children()
        .filter(|node| node.has_tag_name((CF_CAI_NS, "panelDef")))
        .map(|node| node.attribute("id").unwrap())
        .collect::<Vec<_>>();
    let instances = panels
        .descendants()
        .filter(|node| node.has_tag_name((CF_CAI_NS, "panel")))
        .collect::<Vec<_>>();
    assert_eq!(instances.len(), 2);
    for panel in instances {
        assert!(uuid::Uuid::parse_str(panel.attribute("id").unwrap()).is_ok());
        let reference = panel
            .children()
            .find(|node| node.has_tag_name((CF_CAI_NS, "uuid")))
            .unwrap()
            .text()
            .unwrap();
        assert!(definitions.contains(&reference));
    }
}
