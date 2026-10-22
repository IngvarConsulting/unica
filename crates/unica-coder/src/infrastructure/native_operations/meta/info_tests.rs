use std::time::{Duration, Instant};
use unica_metadata_designer::info_read::registrar_scan_checkpoint;
use unica_work_control::cancellation::CancellationToken;
use unica_work_control::provider_deadline::ProviderDeadline;

use super::info::typed_elements;

use unica_metadata::{
    MetaPropertyChanges, MetaPropertyInput, MetaPropertyValue, MetaSupportStatus, MetadataKind,
};

fn typed_local_info_descriptor(name: &str) -> Vec<u8> {
    format!(
        r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Catalog uuid="11111111-1111-1111-1111-111111111111"><Properties><Name>{name}</Name></Properties><ChildObjects/></Catalog></MetaDataObject>"#
    )
    .into_bytes()
}

#[test]
fn typed_local_info_core_projects_the_exact_descriptor_image() {
    let target = unica_address::source_target::MetadataAddress::parse(
        unica_address::source_target::PLATFORM_XML_8_3_27_FORMAT_2_20,
        "Catalog.Products",
    )
    .unwrap();

    let info = unica_metadata_designer::info::parse_typed_meta_local_info(
        &typed_local_info_descriptor("Products"),
        &target,
        MetaSupportStatus::Locked,
    )
    .expect("exact descriptor image is valid");

    assert_eq!(info.metadata_path, target);
    assert_eq!(info.kind, MetadataKind::Catalog);
    assert_eq!(info.name, "Products");
    assert_eq!(info.support, MetaSupportStatus::Locked);
}

#[test]
fn typed_long_reference_type_preserves_every_xml_variant_before_public_compaction() {
    let xml = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures/acceptance/workspace-type-size/src/AccumulationRegisters/Movements.xml"));
    let target = unica_address::source_target::MetadataAddress::parse(
        unica_address::source_target::PLATFORM_XML_8_3_27_FORMAT_2_20,
        "AccumulationRegister.Movements",
    )
    .unwrap();
    let info = unica_metadata_designer::info::parse_typed_meta_local_info(
        xml.as_bytes(),
        &target,
        MetaSupportStatus::Supported,
    )
    .unwrap();
    let ty = info.collections.attributes[0].r#type.as_ref().unwrap();
    assert_eq!(ty.variants.len(), 20);
    assert!(serde_json::to_string(ty).unwrap().len() > 2048);
    let control = info.collections.dimensions[0].r#type.as_ref().unwrap();
    assert_eq!(control.variants.len(), 9);
    assert!(serde_json::to_string(control).unwrap().len() < 2048);
    let document = roxmltree::Document::parse(xml).unwrap();
    let source = document
        .descendants()
        .find(|node| node.is_element() && node.tag_name().name() == "Attribute")
        .unwrap();
    let expected = source
        .descendants()
        .filter(|node| {
            node.is_element()
                && node.tag_name().name() == "Type"
                && node.tag_name().namespace() == Some("http://v8.1c.ru/8.1/data/core")
        })
        .map(|node| {
            node.text()
                .unwrap()
                .replace("cfg:DocumentRef.", "Document.")
        })
        .collect::<Vec<_>>();
    let data = serde_json::to_value(ty).unwrap();
    for (variant, expected) in data["variants"].as_array().unwrap().iter().zip(expected) {
        assert_eq!(variant["metadataPath"], expected);
    }
}

#[test]
fn typed_local_info_core_rejects_a_wrong_logical_owner() {
    let target = unica_address::source_target::MetadataAddress::parse(
        unica_address::source_target::PLATFORM_XML_8_3_27_FORMAT_2_20,
        "Catalog.Orders",
    )
    .unwrap();

    let failure = unica_metadata_designer::info::parse_typed_meta_local_info(
        &typed_local_info_descriptor("Products"),
        &target,
        MetaSupportStatus::Supported,
    )
    .expect_err("descriptor identity must be bound to the requested owner");

    assert!(
        failure.diagnostics[0]
            .message
            .contains("does not match target Catalog.Orders"),
        "{failure:?}"
    );
}

#[test]
fn typed_local_info_core_rejects_malformed_descriptor_bytes() {
    let target = unica_address::source_target::MetadataAddress::parse(
        unica_address::source_target::PLATFORM_XML_8_3_27_FORMAT_2_20,
        "Catalog.Products",
    )
    .unwrap();

    let failure = unica_metadata_designer::info::parse_typed_meta_local_info(
        b"<not-metadata>",
        &target,
        MetaSupportStatus::Supported,
    )
    .expect_err("malformed bytes cannot become typed metadata");

    assert_eq!(
        failure.diagnostics[0].code,
        unica_metadata::MetaDiagnosticCode::ProviderUnavailable
    );
}

#[test]
fn typed_info_reads_simple_form_template_and_command_references() {
    let xml = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses"><Catalog><ChildObjects><Form>ItemForm</Form><Template>Print</Template><Command>Refresh</Command></ChildObjects></Catalog></MetaDataObject>"#;
    let document = roxmltree::Document::parse(xml).unwrap();
    let object = document.root_element().first_element_child().unwrap();
    let children = unica_xml_format::read::child_element(object, "ChildObjects");

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
    let children = unica_xml_format::read::child_element(object, "ChildObjects");

    let attributes = typed_elements(xml, children, "Attribute", false);
    let value = serde_json::to_value(&attributes[0]).unwrap();

    assert_eq!(value["name"], "LooksValidButIsNot");
    assert_eq!(value["incomplete"], true);
}

#[test]
fn recalculation_reference_is_complete_when_its_logical_name_is_proved() {
    let xml = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses"><CalculationRegister><ChildObjects><Recalculation>Main</Recalculation></ChildObjects></CalculationRegister></MetaDataObject>"#;
    let document = roxmltree::Document::parse(xml).unwrap();
    let object = document.root_element().first_element_child().unwrap();
    let children = unica_xml_format::read::child_element(object, "ChildObjects");

    let recalculations = typed_elements(xml, children, "Recalculation", false);
    let value = serde_json::to_value(&recalculations[0]).unwrap();

    assert_eq!(value["name"], "Main");
    assert!(value.get("incomplete").is_none(), "{value}");
}

#[test]
fn empty_recalculation_nulls_the_collection_with_a_diagnostic() {
    let xml = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses"><CalculationRegister><ChildObjects><Recalculation/></ChildObjects></CalculationRegister></MetaDataObject>"#;
    let document = roxmltree::Document::parse(xml).unwrap();
    let object = document.root_element().first_element_child().unwrap();
    let target = unica_address::source_target::MetadataAddress::parse(
        unica_address::source_target::PLATFORM_XML_8_3_27_FORMAT_2_20,
        "CalculationRegister.Payroll",
    )
    .unwrap();
    let mut diagnostics = Vec::new();

    let value = unica_metadata_designer::info::typed_optional_root_collection(
        xml,
        unica_xml_format::read::child_element(object, "ChildObjects"),
        unica_metadata_designer::info::TypedRootCollectionRoute::new(
            MetadataKind::CalculationRegister,
            "Recalculation",
            false,
            "collections.recalculations",
        ),
        &target,
        &mut diagnostics,
    );

    assert_eq!(value, Some(None));
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(
        diagnostics[0].field.as_deref(),
        Some("collections.recalculations[0].name")
    );
}

#[test]
fn optional_root_collections_distinguish_inapplicable_absent_and_proven_empty() {
    let xml = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses"><CalculationRegister><ChildObjects/></CalculationRegister></MetaDataObject>"#;
    let document = roxmltree::Document::parse(xml).unwrap();
    let object = document.root_element().first_element_child().unwrap();
    let target = unica_address::source_target::MetadataAddress::parse(
        unica_address::source_target::PLATFORM_XML_8_3_27_FORMAT_2_20,
        "CalculationRegister.Payroll",
    )
    .unwrap();
    let mut diagnostics = Vec::new();

    let absent = unica_metadata_designer::info::typed_optional_root_collection(
        xml,
        None,
        unica_metadata_designer::info::TypedRootCollectionRoute::new(
            MetadataKind::CalculationRegister,
            "Recalculation",
            false,
            "collections.recalculations",
        ),
        &target,
        &mut diagnostics,
    );
    let empty = unica_metadata_designer::info::typed_optional_root_collection(
        xml,
        unica_xml_format::read::child_element(object, "ChildObjects"),
        unica_metadata_designer::info::TypedRootCollectionRoute::new(
            MetadataKind::CalculationRegister,
            "Recalculation",
            false,
            "collections.recalculations",
        ),
        &target,
        &mut diagnostics,
    );
    let inapplicable = unica_metadata_designer::info::typed_optional_root_collection(
        xml,
        unica_xml_format::read::child_element(object, "ChildObjects"),
        unica_metadata_designer::info::TypedRootCollectionRoute::new(
            MetadataKind::Catalog,
            "Recalculation",
            false,
            "collections.recalculations",
        ),
        &target,
        &mut diagnostics,
    );

    assert_eq!(absent, Some(None));
    assert_eq!(empty, Some(Some(Vec::new())));
    assert_eq!(inapplicable, None);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
}

#[test]
fn optional_root_collection_keeps_an_incomplete_element_with_a_local_warning() {
    let xml = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core"><ChartOfAccounts><ChildObjects><AccountingFlag><Properties><Name>Currency</Name><Type><v8:Type>v8:FutureOpaque</v8:Type></Type></Properties></AccountingFlag></ChildObjects></ChartOfAccounts></MetaDataObject>"#;
    let document = roxmltree::Document::parse(xml).unwrap();
    let object = document.root_element().first_element_child().unwrap();
    let target = unica_address::source_target::MetadataAddress::parse(
        unica_address::source_target::PLATFORM_XML_8_3_27_FORMAT_2_20,
        "ChartOfAccounts.Accounting",
    )
    .unwrap();
    let mut diagnostics = Vec::new();

    let value = unica_metadata_designer::info::typed_optional_root_collection(
        xml,
        unica_xml_format::read::child_element(object, "ChildObjects"),
        unica_metadata_designer::info::TypedRootCollectionRoute::new(
            MetadataKind::ChartOfAccounts,
            "AccountingFlag",
            false,
            "collections.accountingFlags",
        ),
        &target,
        &mut diagnostics,
    );

    let Some(Some(values)) = value else {
        panic!("warning must not erase the proved collection");
    };
    assert_eq!(values.len(), 1);
    assert!(values[0].incomplete);
    assert!(values[0].r#type.is_none());
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(
        diagnostics[0].severity,
        unica_metadata::MetaDiagnosticSeverity::Warning
    );
}

#[test]
fn optional_root_collection_rejects_unknown_nested_semantics_and_missing_identity() {
    let cases = [
        (
            MetadataKind::ChartOfAccounts,
            "ChartOfAccounts.Accounting",
            "AccountingFlag",
            r#"<AccountingFlag><Properties><Name>Currency</Name><UnexpectedCompound/></Properties></AccountingFlag>"#,
        ),
        (
            MetadataKind::ChartOfAccounts,
            "ChartOfAccounts.Accounting",
            "AccountingFlag",
            r#"<AccountingFlag><Properties><Name/></Properties></AccountingFlag>"#,
        ),
        (
            MetadataKind::CalculationRegister,
            "CalculationRegister.Payroll",
            "Recalculation",
            r#"<Recalculation><Unexpected>Hidden</Unexpected></Recalculation>"#,
        ),
    ];

    for (kind, metadata_path, tag, item) in cases {
        let xml = format!(
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses"><Owner><ChildObjects>{item}</ChildObjects></Owner></MetaDataObject>"#
        );
        let document = roxmltree::Document::parse(&xml).unwrap();
        let object = document.root_element().first_element_child().unwrap();
        let target = unica_address::source_target::MetadataAddress::parse(
            unica_address::source_target::PLATFORM_XML_8_3_27_FORMAT_2_20,
            metadata_path,
        )
        .unwrap();
        let mut diagnostics = Vec::new();

        let value = unica_metadata_designer::info::typed_optional_root_collection(
            &xml,
            unica_xml_format::read::child_element(object, "ChildObjects"),
            unica_metadata_designer::info::TypedRootCollectionRoute::new(
                kind,
                tag,
                false,
                "collections.optional",
            ),
            &target,
            &mut diagnostics,
        );

        assert_eq!(value, Some(None), "{tag}: {value:?}");
        assert!(!diagnostics.is_empty(), "{tag} must explain null");
    }
}

#[test]
fn typed_info_preserves_structured_children_with_missing_or_empty_names_as_incomplete() {
    let xml = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses"><Catalog><ChildObjects><Attribute><Properties><Synonym/></Properties></Attribute><Attribute><Properties><Name> </Name></Properties></Attribute></ChildObjects></Catalog></MetaDataObject>"#;
    let document = roxmltree::Document::parse(xml).unwrap();
    let object = document.root_element().first_element_child().unwrap();
    let children = unica_xml_format::read::child_element(object, "ChildObjects");

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
        .and_then(|object| unica_xml_format::read::child_element(object, "Properties"));
    assert_eq!(
        unica_metadata_designer::info::predefined_code_type_for_info(
            properties,
            unica_metadata::MetadataKind::Catalog,
        )
        .as_deref(),
        Some("Number")
    );
}

#[test]
fn typed_info_preserves_uuid_as_an_observed_editable_type() {
    let xml = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core"><Catalog><ChildObjects><Attribute><Properties><Name>ExternalId</Name><Type><v8:Type>v8:UUID</v8:Type></Type></Properties></Attribute></ChildObjects></Catalog></MetaDataObject>"#;
    let document = roxmltree::Document::parse(xml).unwrap();
    let object = document.root_element().first_element_child().unwrap();
    let children = unica_xml_format::read::child_element(object, "ChildObjects");

    let attributes = typed_elements(xml, children, "Attribute", false);
    let value = serde_json::to_value(&attributes[0]).unwrap();

    assert_eq!(
        value["type"],
        serde_json::json!({
            "variants": [{"kind": "uuid"}],
            "mutationCapability": "editable"
        })
    );
    assert!(value.get("incomplete").is_none(), "{value}");
}

#[test]
fn typed_info_resolves_observed_type_qnames_by_namespace_not_prefix() {
    let xml = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:core="http://v8.1c.ru/8.1/data/core" xmlns:schema="http://www.w3.org/2001/XMLSchema"><Catalog><ChildObjects><Attribute><Properties><Name>ExternalId</Name><Type><core:Type>schema:string</core:Type><core:StringQualifiers><core:Length>36</core:Length><core:AllowedLength>Fixed</core:AllowedLength></core:StringQualifiers></Type></Properties></Attribute></ChildObjects></Catalog></MetaDataObject>"#;
    let document = roxmltree::Document::parse(xml).unwrap();
    let object = document.root_element().first_element_child().unwrap();
    let children = unica_xml_format::read::child_element(object, "ChildObjects");

    let attributes = typed_elements(xml, children, "Attribute", false);
    let value = serde_json::to_value(&attributes[0]).unwrap();

    assert_eq!(
        value["type"],
        serde_json::json!({
            "variants": [{
                "kind": "string",
                "length": 36,
                "allowedLength": "fixed"
            }],
            "mutationCapability": "editable"
        })
    );
    assert!(value.get("incomplete").is_none(), "{value}");
}

#[test]
fn typed_info_reads_a_proven_read_only_root_property_without_writer_permission() {
    let xml = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses"><WebService><Properties><DescriptorFileName>exchange.1cws</DescriptorFileName></Properties></WebService></MetaDataObject>"#;
    let document = roxmltree::Document::parse(xml).unwrap();
    let object = document.root_element().first_element_child().unwrap();
    let properties = unica_xml_format::read::child_element(object, "Properties");

    let values = serde_json::to_value(unica_metadata_designer::info::typed_properties(
        properties,
        MetadataKind::WebService,
    ))
    .unwrap();

    assert_eq!(
        values,
        serde_json::json!([{"key": "DescriptorFileName", "value": "exchange.1cws"}])
    );
    assert!(MetaPropertyChanges::convert(
        MetadataKind::WebService,
        vec![MetaPropertyInput::new(
            "DescriptorFileName",
            MetaPropertyValue::String("changed.1cws".to_string()),
        )],
    )
    .is_err());
}

#[test]
fn typed_info_keeps_localized_root_properties_as_text() {
    let xml = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core"><Catalog><Properties><Synonym><v8:item><v8:lang>ru</v8:lang><v8:content>Номенклатура</v8:content></v8:item></Synonym></Properties></Catalog></MetaDataObject>"#;
    let document = roxmltree::Document::parse(xml).unwrap();
    let object = document.root_element().first_element_child().unwrap();
    let properties = unica_xml_format::read::child_element(object, "Properties");

    let values = serde_json::to_value(unica_metadata_designer::info::typed_properties(
        properties,
        MetadataKind::Catalog,
    ))
    .unwrap();

    assert_eq!(
        values,
        serde_json::json!([{"key": "Synonym", "value": "Номенклатура"}])
    );
}

#[test]
fn typed_info_preserves_languages_for_new_read_only_localized_properties() {
    let xml = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core"><Catalog><Properties><ObjectPresentation><v8:item><v8:lang>ru</v8:lang><v8:content>Номенклатура</v8:content></v8:item><v8:item><v8:lang>en</v8:lang><v8:content>Item</v8:content></v8:item></ObjectPresentation></Properties></Catalog></MetaDataObject>"#;
    let document = roxmltree::Document::parse(xml).unwrap();
    let object = document.root_element().first_element_child().unwrap();
    let properties = unica_xml_format::read::child_element(object, "Properties");

    let values = serde_json::to_value(unica_metadata_designer::info::typed_properties(
        properties,
        MetadataKind::Catalog,
    ))
    .unwrap();

    assert_eq!(
        values,
        serde_json::json!([{
            "key": "ObjectPresentation",
            "value": {
                "kind": "localizedString",
                "values": [
                    {"language": "ru", "content": "Номенклатура"},
                    {"language": "en", "content": "Item"}
                ]
            }
        }])
    );
}

#[test]
fn typed_info_publishes_kind_specific_scalar_root_properties() {
    let cases = [
        (
            MetadataKind::Document,
            "Document",
            "Numerator",
            "DocumentNumerator.Sales",
        ),
        (
            MetadataKind::Task,
            "Task",
            "Addressing",
            "Task.Approval.Attribute.Assignee",
        ),
    ];

    for (kind, tag, property, expected) in cases {
        let xml = format!(
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses"><{tag}><Properties><{property}>{expected}</{property}></Properties></{tag}></MetaDataObject>"#
        );
        let document = roxmltree::Document::parse(&xml).unwrap();
        let object = document.root_element().first_element_child().unwrap();
        let properties = unica_xml_format::read::child_element(object, "Properties");

        let values = serde_json::to_value(unica_metadata_designer::info::typed_properties(
            properties, kind,
        ))
        .unwrap();

        assert_eq!(
            values,
            serde_json::json!([{"key": property, "value": expected}]),
            "{property}"
        );
    }
}

#[test]
fn read_only_boolean_properties_do_not_inherit_writer_applicability() {
    for (kind, tag, property) in [
        (MetadataKind::Enum, "Enum", "QuickChoice"),
        (
            MetadataKind::ChartOfCharacteristicTypes,
            "ChartOfCharacteristicTypes",
            "FoldersOnTop",
        ),
    ] {
        let xml = format!(
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses"><{tag}><Properties><{property}>false</{property}></Properties></{tag}></MetaDataObject>"#
        );
        let document = roxmltree::Document::parse(&xml).unwrap();
        let object = document.root_element().first_element_child().unwrap();

        let values = serde_json::to_value(unica_metadata_designer::info::typed_properties(
            unica_xml_format::read::child_element(object, "Properties"),
            kind,
        ))
        .unwrap();

        assert_eq!(
            values,
            serde_json::json!([{"key": property, "value": false}]),
            "{tag}.{property}"
        );
    }

    let xml = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses"><Constant><Properties><QuickChoice>Auto</QuickChoice></Properties></Constant></MetaDataObject>"#;
    let document = roxmltree::Document::parse(xml).unwrap();
    let object = document.root_element().first_element_child().unwrap();
    let values = serde_json::to_value(unica_metadata_designer::info::typed_properties(
        unica_xml_format::read::child_element(object, "Properties"),
        MetadataKind::Constant,
    ))
    .unwrap();
    assert_eq!(
        values,
        serde_json::json!([{"key": "QuickChoice", "value": "Auto"}])
    );
}

#[test]
fn typed_info_preserves_platform_specific_child_collections() {
    let cases = [
        (
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../tests/fixtures/platform_8_3_27/meta_info/edge/chart-of-accounts-child-kinds.xml"
            )),
            vec![("AccountingFlag", "Currency"), ("ExtDimensionAccountingFlag", "Amount")],
        ),
        (
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../tests/fixtures/platform_8_3_27/meta_info/edge/task-addressing.xml"
            )),
            vec![("AddressingAttribute", "Performer")],
        ),
    ];

    for (xml, collections) in cases {
        let document = roxmltree::Document::parse(xml).unwrap();
        let object = document.root_element().first_element_child().unwrap();
        let child_objects = unica_xml_format::read::child_element(object, "ChildObjects");
        for (tag, expected_name) in collections {
            let values = typed_elements(xml, child_objects, tag, false);
            assert_eq!(values.len(), 1, "{tag}");
            assert_eq!(values[0].name, expected_name, "{tag}");
            assert!(values[0].r#type.is_some(), "{tag}");
            if tag == "AddressingAttribute" {
                let value = serde_json::to_value(&values[0]).unwrap();
                assert_eq!(
                    value["addressingDimension"],
                    "InformationRegister.Performers.Dimension.Performer"
                );
            }
        }
    }
}

#[test]
fn typed_info_does_not_publish_a_collection_under_the_wrong_owner_kind() {
    let xml = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses"><Catalog><ChildObjects><AccountingFlag>Currency</AccountingFlag></ChildObjects></Catalog></MetaDataObject>"#;
    let document = roxmltree::Document::parse(xml).unwrap();
    let object = document.root_element().first_element_child().unwrap();
    let target = unica_address::source_target::MetadataAddress::parse(
        unica_address::source_target::PLATFORM_XML_8_3_27_FORMAT_2_20,
        "Catalog.Items",
    )
    .unwrap();
    let mut diagnostics = Vec::new();

    let values = unica_metadata_designer::info::typed_root_collection(
        xml,
        unica_xml_format::read::child_element(object, "ChildObjects"),
        unica_metadata_designer::info::TypedRootCollectionRoute::new(
            MetadataKind::Catalog,
            "AccountingFlag",
            false,
            "collections.accountingFlags",
        ),
        &target,
        &mut diagnostics,
    );

    assert!(values.is_empty());
    assert!(diagnostics.is_empty());
}

#[test]
fn typed_info_preserves_non_empty_data_lock_fields() {
    let xml = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/platform_8_3_27/meta_info/edge/task-addressing.xml"
    ));
    let document = roxmltree::Document::parse(xml).unwrap();
    let object = document.root_element().first_element_child().unwrap();
    let properties = unica_xml_format::read::child_element(object, "Properties");
    let target = unica_address::source_target::MetadataAddress::parse(
        unica_address::source_target::PLATFORM_XML_8_3_27_FORMAT_2_20,
        "Task.Approval",
    )
    .unwrap();
    let mut diagnostics = Vec::new();

    let relations = unica_metadata_designer::info::typed_relations(
        &document,
        properties,
        MetadataKind::Task,
        &target,
        &mut diagnostics,
    );
    let value = serde_json::to_value(relations).unwrap();

    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert_eq!(
        value["dataLockFields"],
        serde_json::json!([{
            "kind": "field",
            "value": "Task.Approval.Attribute.Subject"
        }])
    );
}

#[test]
fn malformed_data_lock_entry_nulls_the_collection_instead_of_publishing_a_subset() {
    let xml = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable"><Task><Properties><Name>Approval</Name><DataLockFields><xr:Field>Task.Approval.Attribute.Subject</xr:Field><xr:Field><xr:Unexpected/></xr:Field></DataLockFields></Properties></Task></MetaDataObject>"#;
    let document = roxmltree::Document::parse(xml).unwrap();
    let object = document.root_element().first_element_child().unwrap();
    let target = unica_address::source_target::MetadataAddress::parse(
        unica_address::source_target::PLATFORM_XML_8_3_27_FORMAT_2_20,
        "Task.Approval",
    )
    .unwrap();
    let mut diagnostics = Vec::new();

    let relations = unica_metadata_designer::info::typed_relations(
        &document,
        unica_xml_format::read::child_element(object, "Properties"),
        MetadataKind::Task,
        &target,
        &mut diagnostics,
    );
    let value = serde_json::to_value(relations).unwrap();

    assert!(value["dataLockFields"].is_null(), "{value}");
    assert_eq!(diagnostics.len(), 1);
}

#[test]
fn typed_relations_resolve_object_xsi_type_by_namespace_uri() {
    for (xsi_type, expected_len) in [("read:MDObjectRef", 1usize), ("xs:string", 0usize)] {
        let xml = format!(
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:read="http://v8.1c.ru/8.3/xcf/readable" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"><Catalog><Properties><Name>Items</Name><Owners><read:Item xsi:type="{xsi_type}">Catalog.Owner</read:Item></Owners></Properties></Catalog></MetaDataObject>"#
        );
        let document = roxmltree::Document::parse(&xml).unwrap();
        let object = document.root_element().first_element_child().unwrap();
        let target = unica_address::source_target::MetadataAddress::parse(
            unica_address::source_target::PLATFORM_XML_8_3_27_FORMAT_2_20,
            "Catalog.Items",
        )
        .unwrap();
        let mut diagnostics = Vec::new();

        let relations = unica_metadata_designer::info::typed_relations(
            &document,
            unica_xml_format::read::child_element(object, "Properties"),
            MetadataKind::Catalog,
            &target,
            &mut diagnostics,
        );

        let owners = relations.owners.expect("owners apply to Catalog");
        assert_eq!(
            owners.as_ref().map(Vec::len),
            (expected_len == 1).then_some(1),
            "{xsi_type}"
        );
        assert_eq!(diagnostics.is_empty(), expected_len == 1, "{xsi_type}");
    }
}

#[test]
fn typed_info_does_not_publish_a_foreign_or_malformed_property() {
    let xml = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:foreign="urn:foreign"><Catalog><Properties><foreign:Hierarchical>false</foreign:Hierarchical><CodeLength>9<foreign:Unexpected/></CodeLength></Properties></Catalog></MetaDataObject>"#;
    let document = roxmltree::Document::parse(xml).unwrap();
    let object = document.root_element().first_element_child().unwrap();
    let properties = unica_xml_format::read::child_element(object, "Properties");

    assert!(
        unica_metadata_designer::info::typed_properties(properties, MetadataKind::Catalog)
            .is_empty()
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

#[test]
fn platform_incomplete_borrowed_metadata_keeps_local_observations() {
    use unica_metadata::MetaInfoPropertyValue;
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/platform_8_3_27/incomplete_borrowed");
    for (kind, file, name) in [
        (MetadataKind::Catalog, "catalog", "CorpusCatalog"),
        (MetadataKind::Document, "document", "CorpusDocument"),
        (
            MetadataKind::InformationRegister,
            "information-register",
            "CorpusRegister",
        ),
        (MetadataKind::CommonModule, "common-module", "CorpusModule"),
        (
            MetadataKind::ChartOfCalculationTypes,
            "chart-of-calculation-types",
            "CorpusCalculationTypes",
        ),
    ] {
        let bytes = std::fs::read(root.join(format!("extension-{file}.xml"))).unwrap();
        let target = unica_address::source_target::MetadataAddress::parse(
            unica_address::source_target::PLATFORM_XML_8_3_27_FORMAT_2_20,
            &format!("{}.{name}", kind.as_str()),
        )
        .unwrap();
        let info = unica_metadata_designer::info::parse_typed_meta_local_info(
            &bytes,
            &target,
            MetaSupportStatus::Supported,
        )
        .unwrap();
        assert_eq!(info.name, name);
        assert!(
            info.diagnostics.is_empty(),
            "{file}: the platform export must read without diagnostics"
        );
        assert_eq!(
            info.predefined_code_type, None,
            "{file}: missing CodeType must not become String"
        );
        assert!(info.properties.iter().any(|prop| prop.key == "Comment"
            && prop.value == MetaInfoPropertyValue::String(String::new())));
        if kind == MetadataKind::Document {
            assert_eq!(info.relations.register_records, Some(Some(Vec::new())));
            assert_eq!(info.relations.based_on, Some(None));
        }
        if kind == MetadataKind::Catalog {
            assert_eq!(info.relations.owners, Some(None));
            assert!(info.properties.iter().any(|prop| prop.key == "Hierarchical"
                && prop.value == MetaInfoPropertyValue::Boolean(false)));
        }
        for missing in ["CodeType", "WriteMode", "Posting", "Server"] {
            assert!(
                !info.properties.iter().any(|prop| prop.key == missing),
                "{file}: {missing}"
            );
        }
    }
}

#[test]
fn missing_write_mode_does_not_claim_complete_registrar_evidence() {
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/platform_8_3_27/incomplete_borrowed/extension-information-register.xml");
    let raw = std::fs::read_to_string(fixture).unwrap();
    let root = tempfile::tempdir().unwrap();
    // The scan refuses a root reached through a link (macOS `/var`).
    let source_root = std::fs::canonicalize(root.path()).unwrap();
    let target = unica_address::source_target::MetadataAddress::parse(
        unica_address::source_target::PLATFORM_XML_8_3_27_FORMAT_2_20,
        "InformationRegister.CorpusRegister",
    )
    .unwrap();
    for value in [
        None,
        Some("Independent"),
        Some("RecorderSubordinate"),
        Some("broken"),
        Some(""),
    ] {
        // Explicit variants are synthetic. The unchanged omission is platform XML.
        let xml = value.map_or_else(
            || raw.clone(),
            |value| {
                raw.replace(
                    "</Properties>",
                    &format!("<WriteMode>{value}</WriteMode></Properties>"),
                )
            },
        );
        let doc = roxmltree::Document::parse(xml.trim_start_matches('\u{feff}')).unwrap();
        let props = doc
            .descendants()
            .find(|node| node.has_tag_name("Properties"));
        let (images, evidence) = super::info::typed_registrar_document_images(
            &source_root,
            MetadataKind::InformationRegister,
            props,
            &target,
            ProviderDeadline::new(Instant::now() + Duration::from_secs(10)),
            &CancellationToken::new(),
        );
        assert!(images.is_empty());
        match evidence {
            unica_metadata::validation_subject::MetadataEvidenceAvailability::Complete => {
                assert!(matches!(value, Some("Independent" | "RecorderSubordinate")))
            }
            unica_metadata::validation_subject::MetadataEvidenceAvailability::Unavailable(
                diagnostics,
            ) => {
                assert!(!matches!(
                    value,
                    Some("Independent" | "RecorderSubordinate")
                ));
                assert_eq!(diagnostics.len(), 1);
                assert_eq!(diagnostics[0].field.as_deref(), Some("registrarEvidence"));
                assert_eq!(
                    diagnostics[0].message.contains("is absent"),
                    value.is_none()
                );
            }
        }
    }
}

#[test]
fn typed_tabular_section_preserves_the_declared_standard_attributes() {
    let xml = include_bytes!("../../../../../../tests/fixtures/acceptance/workspace-tabular-standard/src/Catalogs/Items.xml");
    let target = unica_address::source_target::MetadataAddress::parse(
        unica_address::source_target::PLATFORM_XML_8_3_27_FORMAT_2_20,
        "Catalog.Items",
    )
    .unwrap();
    let local = unica_metadata_designer::info::parse_typed_meta_local_info(
        xml,
        &target,
        MetaSupportStatus::Locked,
    )
    .unwrap();
    let section = serde_json::to_value(&local.collections.tabular_sections[0]).unwrap();
    assert_eq!(section["attributes"].as_array().unwrap().len(), 10);
    assert_eq!(section["standardAttributes"][0]["name"], "LineNumber");
    assert_eq!(
        section["standardAttributes"][0]["properties"]
            .as_array()
            .unwrap()
            .len(),
        25
    );
}

#[test]
fn typed_member_comments_keep_empty_absent_and_damaged_observations() {
    for (owner, tag, field) in [
        ("Catalog", "Attribute", "attributes"),
        ("Catalog", "Command", "commands"),
        ("InformationRegister", "Dimension", "dimensions"),
        ("InformationRegister", "Resource", "resources"),
        ("DocumentJournal", "Column", "columns"),
        ("Enum", "EnumValue", "enumValues"),
    ] {
        for (comment, expected, damaged) in [
            ("<Comment/>", Some(""), false),
            ("", None, false),
            ("<Comment>Text</Comment>", Some("Text"), false),
            ("<Comment><Nested/></Comment>", None, true),
        ] {
            let xml = format!(
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><{owner} uuid="f1584000-0000-4000-8000-000000000001"><Properties><Name>Owner</Name></Properties><ChildObjects><{tag}><Properties><Name>Value</Name>{comment}</Properties></{tag}></ChildObjects></{owner}></MetaDataObject>"#
            );
            let target = unica_address::source_target::MetadataAddress::parse(
                unica_address::source_target::PLATFORM_XML_8_3_27_FORMAT_2_20,
                &format!("{owner}.Owner"),
            )
            .unwrap();
            let info = unica_metadata_designer::info::parse_typed_meta_local_info(
                xml.as_bytes(),
                &target,
                MetaSupportStatus::Supported,
            )
            .unwrap();
            let data = serde_json::to_value(&info.collections).unwrap();
            let item = &data[field][0];
            assert_eq!(item["name"], "Value");
            assert_eq!(
                item.get("comment").and_then(serde_json::Value::as_str),
                expected,
                "{tag}: {comment}"
            );
            assert_eq!(item["incomplete"].as_bool().unwrap_or(false), damaged);
            assert_eq!(
                info.diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.field.as_deref()
                        == Some(&format!("collections.{field}[0].comment"))),
                damaged
            );
        }
    }
}
