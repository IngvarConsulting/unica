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
