use crate::domain::cancellation::CancellationToken;
use crate::domain::code_intelligence::ProviderDeadline;
use std::time::{Duration, Instant};

use super::info::{predefined_code_type_for_info, registrar_scan_checkpoint, typed_elements};
use super::xml_model::meta_info_child;

#[test]
fn typed_info_reads_simple_form_template_and_command_references() {
    let xml = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses"><Catalog><ChildObjects><Form>ItemForm</Form><Template>Print</Template><Command>Refresh</Command></ChildObjects></Catalog></MetaDataObject>"#;
    let document = roxmltree::Document::parse(xml).unwrap();
    let object = document.root_element().first_element_child().unwrap();
    let children = meta_info_child(object, "ChildObjects");

    let forms = typed_elements(xml, children, "Form", false);
    let templates = typed_elements(xml, children, "Template", false);
    let commands = typed_elements(xml, children, "Command", false);

    assert_eq!(forms[0].name, "ItemForm");
    assert_eq!(templates[0].name, "Print");
    assert_eq!(commands[0].name, "Refresh");
}

#[test]
fn typed_info_marks_nonphysical_children_without_properties_incomplete() {
    let xml = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses"><Catalog><ChildObjects><Attribute>LooksValidButIsNot</Attribute></ChildObjects></Catalog></MetaDataObject>"#;
    let document = roxmltree::Document::parse(xml).unwrap();
    let object = document.root_element().first_element_child().unwrap();
    let children = meta_info_child(object, "ChildObjects");

    let attributes = typed_elements(xml, children, "Attribute", false);
    let value = serde_json::to_value(&attributes[0]).unwrap();

    assert_eq!(value["name"], "LooksValidButIsNot");
    assert_eq!(value["incomplete"], true);
}

#[test]
fn typed_info_preserves_structured_children_with_missing_or_empty_names_as_incomplete() {
    let xml = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses"><Catalog><ChildObjects><Attribute><Properties><Synonym/></Properties></Attribute><Attribute><Properties><Name> </Name></Properties></Attribute></ChildObjects></Catalog></MetaDataObject>"#;
    let document = roxmltree::Document::parse(xml).unwrap();
    let object = document.root_element().first_element_child().unwrap();
    let children = meta_info_child(object, "ChildObjects");

    let attributes = typed_elements(xml, children, "Attribute", false);
    let value = serde_json::to_value(&attributes).unwrap();

    assert_eq!(value.as_array().unwrap().len(), 2);
    assert_eq!(value[0]["name"], "");
    assert_eq!(value[0]["incomplete"], true);
    assert_eq!(value[1]["name"], "");
    assert_eq!(value[1]["incomplete"], true);
}

#[test]
fn typed_info_retains_owner_code_type_for_predefined_validation() {
    let xml = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses"><Catalog><Properties><CodeType>Number</CodeType></Properties></Catalog></MetaDataObject>"#;
    let document = roxmltree::Document::parse(xml).unwrap();
    let properties = document
        .root_element()
        .first_element_child()
        .and_then(|object| meta_info_child(object, "Properties"));
    assert_eq!(
        predefined_code_type_for_info(properties, crate::domain::metadata::MetadataKind::Catalog,)
            .as_deref(),
        Some("Number")
    );
}

#[test]
fn registrar_scan_checkpoint_distinguishes_deadline_and_cancellation() {
    let active = CancellationToken::new();
    assert!(registrar_scan_checkpoint(
        ProviderDeadline::new(Instant::now() + Duration::from_secs(1)),
        &active,
    )
    .is_ok());

    let timeout = registrar_scan_checkpoint(
        ProviderDeadline::new(Instant::now() - Duration::from_millis(1)),
        &active,
    )
    .unwrap_err();
    assert_eq!(timeout.kind(), std::io::ErrorKind::TimedOut);

    let cancelled = CancellationToken::new();
    cancelled.cancel();
    let cancellation = registrar_scan_checkpoint(
        ProviderDeadline::new(Instant::now() + Duration::from_secs(1)),
        &cancelled,
    )
    .unwrap_err();
    assert_eq!(cancellation.kind(), std::io::ErrorKind::Interrupted);
}
