use crate::domain::metadata::{MetaDiagnosticCode, MetadataKind};
use crate::domain::source_target::{MetadataAddress, PLATFORM_XML_8_3_27_FORMAT_2_20};

use super::info::{typed_properties, typed_root_collection, TypedRootCollectionRoute};
use super::info_projection::{
    declared_meta_info_collection_routes, declared_meta_info_semantic_routes,
    meta_info_profile_errors, observed_meta_info_semantic_routes,
    observed_type_is_strict_but_unmodelled, parse_observed_metadata_type_node,
    project_meta_info_declarations, project_meta_info_details, validate_meta_info_profile,
};
use super::template_catalog::minimal_metadata_xml_for_tests;
use super::xml_model::meta_info_child;

fn project(xml: &str, kind: MetadataKind, metadata_path: &str) -> serde_json::Value {
    let document = roxmltree::Document::parse(xml).unwrap();
    let object = document.root_element().first_element_child().unwrap();
    let properties = meta_info_child(object, "Properties");
    let child_objects = meta_info_child(object, "ChildObjects");
    let target = MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, metadata_path).unwrap();
    let mut diagnostics = Vec::new();
    let details =
        project_meta_info_details(kind, properties, child_objects, &target, &mut diagnostics);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    serde_json::to_value(details).unwrap()["details"].clone()
}

#[test]
fn report_profile_observes_the_tracked_auxiliary_variant_form() {
    let xml = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/unica_mcp_script_parity/bsp/meta/Reports/",
        "АнализВерсийОбъектов.xml"
    ));
    let document = roxmltree::Document::parse(xml.trim_start_matches('\u{feff}')).unwrap();
    let object = document.root_element().first_element_child().unwrap();
    let properties = meta_info_child(object, "Properties");

    validate_meta_info_profile(MetadataKind::Report, properties, None).unwrap();
    let projected = serde_json::to_value(typed_properties(properties, MetadataKind::Report))
        .unwrap()
        .as_array()
        .unwrap()
        .clone();
    assert!(projected
        .iter()
        .any(|property| { property["key"] == "AuxiliaryVariantForm" && property["value"] == "" }));
}

fn project_declarations(xml: &str, kind: MetadataKind, metadata_path: &str) -> serde_json::Value {
    let document = roxmltree::Document::parse(xml).unwrap();
    let object = document.root_element().first_element_child().unwrap();
    let target = MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, metadata_path).unwrap();
    let mut diagnostics = Vec::new();
    let declarations = project_meta_info_declarations(
        kind,
        meta_info_child(object, "Properties"),
        &target,
        &mut diagnostics,
    );
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    serde_json::to_value(declarations).unwrap()
}

#[test]
fn scheduled_job_details_split_the_logical_common_module_method() {
    let xml = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/platform_8_3_27/meta_info/edge/scheduled-job.xml"
    ));

    let details = project(xml, MetadataKind::ScheduledJob, "ScheduledJob.MonthClose");

    assert_eq!(
        details["method"],
        serde_json::json!({
            "metadataPath": "CommonModule.MonthClose",
            "method": "RunScheduled"
        })
    );
}

#[test]
fn scheduled_job_rejects_a_two_part_value_that_is_not_a_common_module_method() {
    let xml = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses"><ScheduledJob><Properties><Name>Invalid</Name><MethodName>Catalog.Items</MethodName></Properties></ScheduledJob></MetaDataObject>"#;
    let document = roxmltree::Document::parse(xml).unwrap();
    let object = document.root_element().first_element_child().unwrap();
    let target =
        MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "ScheduledJob.Invalid").unwrap();
    let mut diagnostics = Vec::new();

    let value = serde_json::to_value(project_meta_info_details(
        MetadataKind::ScheduledJob,
        meta_info_child(object, "Properties"),
        None,
        &target,
        &mut diagnostics,
    ))
    .unwrap();

    assert!(value["details"]["method"].is_null());
    assert_eq!(diagnostics[0].field.as_deref(), Some("details.method"));
}

#[test]
fn calculation_register_details_keep_the_schedule_triple_at_its_real_owner() {
    let xml = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/platform_8_3_27/meta_info/edge/calculation-register.xml"
    ));

    let details = project(
        xml,
        MetadataKind::CalculationRegister,
        "CalculationRegister.Payroll",
    );

    assert_eq!(
        details["schedule"],
        serde_json::json!({
            "register": "InformationRegister.WorkSchedules",
            "valueField": "InformationRegister.WorkSchedules.Resource.DayValue",
            "dateField": "InformationRegister.WorkSchedules.Dimension.Date"
        })
    );

    let document = roxmltree::Document::parse(xml).unwrap();
    let object = document.root_element().first_element_child().unwrap();
    assert!(validate_meta_info_profile(
        MetadataKind::CalculationRegister,
        meta_info_child(object, "Properties"),
        meta_info_child(object, "ChildObjects"),
    )
    .is_ok());
}

#[test]
fn http_service_details_preserve_templates_and_methods() {
    let xml = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/platform_8_3_27/meta_info/edge/http-service.xml"
    ));

    let details = project(xml, MetadataKind::HTTPService, "HTTPService.ExternalAPI");

    assert_eq!(
        details["urlTemplates"],
        serde_json::json!([{
            "name": "Metrics",
            "template": "/v1/kpi/",
            "methods": [{
                "name": "Get",
                "httpMethod": "GET",
                "handler": "MetricsGet"
            }]
        }])
    );
}

#[test]
fn web_service_details_preserve_packages_operations_and_expanded_qnames() {
    let xml = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/platform_8_3_27/meta_info/edge/web-service.xml"
    ));

    let details = project(xml, MetadataKind::WebService, "WebService.Exchange");

    assert_eq!(
        details["xdtoPackages"],
        serde_json::json!([
            {"kind": "package", "metadataPath": "XDTOPackage.Exchange"},
            {"kind": "namespace", "namespace": "http://v8.1c.ru/8.1/data/core"}
        ])
    );
    assert_eq!(
        details["operations"],
        serde_json::json!([{
            "name": "Send",
            "returnType": {
                "namespace": "http://www.w3.org/2001/XMLSchema",
                "localName": "boolean"
            },
            "nillable": false,
            "transactioned": true,
            "procedure": "SendData",
            "parameters": [{
                "name": "Payload",
                "type": {"namespace": "urn:example:service", "localName": "Payload"},
                "nillable": true,
                "direction": "in"
            }]
        }])
    );
}

#[test]
fn manifest_edge_fixtures_keep_the_canonical_platform_wrapper() {
    let manifest: serde_json::Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/platform_8_3_27/meta_info/manifest.json"
    )))
    .unwrap();
    let fixture_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/platform_8_3_27/meta_info");

    for entry in manifest["kinds"].as_array().unwrap() {
        for relative in entry
            .get("edgeFixtures")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
        {
            let path = fixture_root.join(relative.as_str().unwrap());
            let xml = std::fs::read_to_string(&path).unwrap();
            assert_eq!(
                xml.lines().next(),
                Some("<?xml version=\"1.0\" encoding=\"UTF-8\"?>"),
                "{} must preserve the canonical XML declaration",
                path.display()
            );
            let document = roxmltree::Document::parse(&xml).unwrap();
            let object = document.root_element().first_element_child().unwrap();
            assert!(
                object
                    .attribute("uuid")
                    .is_some_and(|uuid| !uuid.is_empty()),
                "{} must preserve the canonical metadata-object UUID",
                path.display()
            );
        }
    }
}

#[test]
fn manifest_and_profile_cover_every_platform_gated_metadata_kind() {
    let manifest: serde_json::Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/platform_8_3_27/meta_info/manifest.json"
    )))
    .unwrap();
    let manifest_kinds = manifest["kinds"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["kind"].as_str().unwrap())
        .collect::<Vec<_>>();
    let expected_kinds = MetadataKind::ALL
        .iter()
        .map(|kind| kind.as_str())
        .collect::<Vec<_>>();
    assert_eq!(manifest_kinds, expected_kinds);

    let platform_cases = manifest["kinds"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| {
            assert_eq!(entry["mainFixture"], "canonical-template");
            entry["platformCase"].as_str().unwrap()
        })
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(platform_cases.len(), MetadataKind::ALL.len());

    let platform_corpus = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/format_8_3_27_xml_corpus.rs"
    ));
    for platform_case in platform_cases {
        assert!(
            platform_corpus.contains(&format!("\"{platform_case}\"")),
            "platform case {platform_case} is not tracked by the exact-platform corpus"
        );
    }

    let mut profile_errors = Vec::new();
    let mut observed_routes = std::collections::BTreeSet::new();
    for kind in MetadataKind::ALL {
        let (xml, _) = minimal_metadata_xml_for_tests(*kind, "Evidence").unwrap();
        let document = roxmltree::Document::parse(&xml).unwrap();
        let object = document.root_element().first_element_child().unwrap();
        let properties = meta_info_child(object, "Properties");
        let child_objects = meta_info_child(object, "ChildObjects");
        let errors = meta_info_profile_errors(*kind, properties, child_objects);
        if !errors.is_empty() {
            profile_errors.extend(
                errors
                    .into_iter()
                    .map(|error| format!("{}: {error:?}", kind.as_str())),
            );
            continue;
        }
        observed_routes.extend(observed_meta_info_semantic_routes(
            *kind,
            properties,
            child_objects,
        ));
        let target = MetadataAddress::parse(
            PLATFORM_XML_8_3_27_FORMAT_2_20,
            &format!("{}.Evidence", kind.as_str()),
        )
        .unwrap();
        let mut diagnostics = Vec::new();
        let value = serde_json::to_value(project_meta_info_details(
            *kind,
            properties,
            child_objects,
            &target,
            &mut diagnostics,
        ))
        .unwrap();
        assert!(diagnostics.is_empty(), "{}: {diagnostics:?}", kind.as_str());
        assert_eq!(value["kind"], kind.as_str());
        assert!(value["details"].is_object(), "{}: {value}", kind.as_str());
    }
    assert!(
        profile_errors.is_empty(),
        "unclassified canonical profile nodes:\n{}",
        profile_errors.join("\n")
    );

    let fixture_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/platform_8_3_27/meta_info");
    for entry in manifest["kinds"].as_array().unwrap() {
        let kind = MetadataKind::parse(entry["kind"].as_str().unwrap()).unwrap();
        for relative in entry
            .get("edgeFixtures")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
        {
            let path = fixture_root.join(relative.as_str().unwrap());
            let xml = std::fs::read_to_string(&path).unwrap();
            let document = roxmltree::Document::parse(&xml).unwrap();
            let object = document.root_element().first_element_child().unwrap();
            assert_eq!(
                object.tag_name().name(),
                kind.as_str(),
                "{}",
                path.display()
            );
            let properties = meta_info_child(object, "Properties");
            let child_objects = meta_info_child(object, "ChildObjects");
            validate_meta_info_profile(kind, properties, child_objects).unwrap();
            observed_routes.extend(observed_meta_info_semantic_routes(
                kind,
                properties,
                child_objects,
            ));
            let name = properties
                .and_then(|node| super::xml_model::meta_info_child_text(node, "Name"))
                .unwrap();
            let target = MetadataAddress::parse(
                PLATFORM_XML_8_3_27_FORMAT_2_20,
                &format!("{}.{name}", kind.as_str()),
            )
            .unwrap();
            let mut diagnostics = Vec::new();
            let details = serde_json::to_value(project_meta_info_details(
                kind,
                properties,
                child_objects,
                &target,
                &mut diagnostics,
            ))
            .unwrap();
            assert!(
                diagnostics.is_empty(),
                "{}: {diagnostics:?}",
                path.display()
            );
            assert_eq!(details["kind"], kind.as_str());
        }
    }

    let mut manifest_collection_routes = std::collections::BTreeSet::new();
    for (kind_name, tags) in manifest["collectionRouteMatrix"].as_object().unwrap() {
        let kind = MetadataKind::parse(kind_name).unwrap();
        for tag in tags
            .as_array()
            .unwrap()
            .iter()
            .map(|tag| tag.as_str().unwrap())
        {
            let route = format!("{}.childObjects.{tag}", kind.as_str());
            assert!(manifest_collection_routes.insert(route.clone()), "{route}");
            let item = match tag {
                "AccountingFlag" | "ExtDimensionAccountingFlag" => format!(
                    "<{tag}><Properties><Name>Evidence</Name><Type><v8:Type>xs:boolean</v8:Type></Type></Properties></{tag}>"
                ),
                "AddressingAttribute" => format!(
                    "<{tag}><Properties><Name>Evidence</Name><Type><v8:Type>xs:string</v8:Type><v8:StringQualifiers><v8:Length>10</v8:Length><v8:AllowedLength>Variable</v8:AllowedLength></v8:StringQualifiers></Type><AddressingDimension>InformationRegister.Evidence.Dimension.Evidence</AddressingDimension></Properties></{tag}>"
                ),
                _ => format!("<{tag}>Evidence</{tag}>"),
            };
            let xml = format!(
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:xs="http://www.w3.org/2001/XMLSchema"><{kind}><ChildObjects>{item}</ChildObjects></{kind}></MetaDataObject>"#,
                kind = kind.as_str(),
            );
            let document = roxmltree::Document::parse(&xml).unwrap();
            let object = document.root_element().first_element_child().unwrap();
            let target = MetadataAddress::parse(
                PLATFORM_XML_8_3_27_FORMAT_2_20,
                &format!("{}.Evidence", kind.as_str()),
            )
            .unwrap();
            let mut diagnostics = Vec::new();
            let values = typed_root_collection(
                &xml,
                meta_info_child(object, "ChildObjects"),
                TypedRootCollectionRoute::new(
                    kind,
                    tag,
                    tag == "TabularSection",
                    "collections.routeEvidence",
                ),
                &target,
                &mut diagnostics,
            );
            assert_eq!(values.len(), 1, "{route}");
            assert!(diagnostics.is_empty(), "{diagnostics:?}");
            observed_routes.insert(route);
        }
    }
    let declared_collection_routes = declared_meta_info_collection_routes()
        .into_iter()
        .map(|(kind, tag)| format!("{}.childObjects.{tag}", kind.as_str()))
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(manifest_collection_routes, declared_collection_routes);

    for (kind_name, names) in manifest["propertyRouteMatrix"].as_object().unwrap() {
        let kind = MetadataKind::parse(kind_name).unwrap();
        for name in names
            .as_array()
            .unwrap()
            .iter()
            .map(|name| name.as_str().unwrap())
        {
            let xml = format!(
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses"><{kind}><Properties><Name>Evidence</Name><{name}>false</{name}></Properties></{kind}></MetaDataObject>"#,
                kind = kind.as_str(),
            );
            let document = roxmltree::Document::parse(&xml).unwrap();
            let object = document.root_element().first_element_child().unwrap();
            let properties = meta_info_child(object, "Properties");
            assert!(
                meta_info_profile_errors(kind, properties, None).is_empty(),
                "{}.properties.{name}",
                kind.as_str()
            );
            assert_eq!(typed_properties(properties, kind).len(), 1);
            observed_routes.extend(observed_meta_info_semantic_routes(kind, properties, None));
        }
    }

    let declared_routes = declared_meta_info_semantic_routes();
    let missing_routes = declared_routes
        .difference(&observed_routes)
        .cloned()
        .collect::<Vec<_>>();
    assert!(
        missing_routes.is_empty(),
        "declared semantic routes without canonical, edge, or explicit route-matrix evidence:\n{}",
        missing_routes.join("\n")
    );
}

#[test]
fn defined_type_details_preserve_type_sets_and_qualifiers() {
    let xml = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/platform_8_3_27/meta_info/edge/defined-type.xml"
    ));

    let details = project(xml, MetadataKind::DefinedType, "DefinedType.Identifier");

    assert_eq!(
        details["type"],
        serde_json::json!({
            "variants": [
                {"kind": "string", "length": 13, "allowedLength": "fixed"},
                {"kind": "definedType", "metadataPath": "DefinedType.GLN"}
            ],
            "mutationCapability": "editable"
        })
    );
}

#[test]
fn characteristic_type_details_preserve_the_observed_value_type() {
    let xml = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:xs="http://www.w3.org/2001/XMLSchema"><ChartOfCharacteristicTypes><Properties><Name>Characteristics</Name><Type><v8:Type>xs:boolean</v8:Type></Type></Properties></ChartOfCharacteristicTypes></MetaDataObject>"#;

    let details = project(
        xml,
        MetadataKind::ChartOfCharacteristicTypes,
        "ChartOfCharacteristicTypes.Characteristics",
    );

    assert_eq!(
        details["type"],
        serde_json::json!({
            "variants": [{"kind": "boolean"}],
            "mutationCapability": "editable"
        })
    );
}

#[test]
fn canonical_standard_attributes_and_tabular_sections_have_typed_owners() {
    let (xml, _) =
        minimal_metadata_xml_for_tests(MetadataKind::ChartOfAccounts, "Accounts").unwrap();

    let declarations = project_declarations(
        &xml,
        MetadataKind::ChartOfAccounts,
        "ChartOfAccounts.Accounts",
    );

    let attributes = declarations["standardAttributes"].as_array().unwrap();
    assert!(!attributes.is_empty());
    let reference = attributes
        .iter()
        .find(|attribute| attribute["name"] == "Ref")
        .unwrap();
    assert!(reference["properties"]
        .as_array()
        .unwrap()
        .iter()
        .any(|property| {
            property["name"] == "FillChecking" && property["value"]["kind"] == "text"
        }));
    let sections = declarations["standardTabularSections"].as_array().unwrap();
    assert_eq!(sections[0]["name"], "ExtDimensionTypes");
    assert_eq!(
        sections[0]["standardAttributes"].as_array().unwrap().len(),
        4
    );
}

#[test]
fn declaration_localized_strings_preserve_language_and_content_pairs() {
    let xml = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" xmlns:v8="http://v8.1c.ru/8.1/data/core"><Catalog><Properties><Name>Items</Name><StandardAttributes><xr:StandardAttribute name="Ref"><xr:ToolTip><v8:item><v8:lang>ru</v8:lang><v8:content>Ссылка</v8:content></v8:item><v8:item><v8:lang>en</v8:lang><v8:content>Reference</v8:content></v8:item></xr:ToolTip></xr:StandardAttribute></StandardAttributes></Properties></Catalog></MetaDataObject>"#;

    let declarations = project_declarations(xml, MetadataKind::Catalog, "Catalog.Items");
    let tooltip = declarations["standardAttributes"][0]["properties"][0]["value"].clone();

    assert_eq!(
        tooltip,
        serde_json::json!({
            "kind": "localizedString",
            "values": [
                {"language": "ru", "content": "Ссылка"},
                {"language": "en", "content": "Reference"}
            ]
        })
    );
}

#[test]
fn applicable_declaration_collections_distinguish_absent_from_inapplicable() {
    let chart = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses"><ChartOfAccounts><Properties><Name>Accounts</Name></Properties></ChartOfAccounts></MetaDataObject>"#;
    let declarations = project_declarations(
        chart,
        MetadataKind::ChartOfAccounts,
        "ChartOfAccounts.Accounts",
    );

    assert!(declarations.get("standardAttributes").unwrap().is_null());
    assert!(declarations.get("characteristics").unwrap().is_null());
    assert!(declarations
        .get("standardTabularSections")
        .unwrap()
        .is_null());

    let service = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses"><WebService><Properties><Name>Exchange</Name></Properties></WebService></MetaDataObject>"#;
    let declarations =
        project_declarations(service, MetadataKind::WebService, "WebService.Exchange");
    assert_eq!(declarations, serde_json::json!({}));
}

#[test]
fn declaration_localized_leaves_reject_unmodelled_attributes() {
    let xml = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" xmlns:foreign="urn:foreign"><ChartOfAccounts><Properties><Name>Accounts</Name><StandardTabularSections><xr:StandardTabularSection name="ExtDimensionTypes"><xr:Synonym foreign:source="poison"/><xr:Comment/><xr:ToolTip/><xr:FillChecking>DontCheck</xr:FillChecking><xr:StandardAttributes/></xr:StandardTabularSection></StandardTabularSections></Properties></ChartOfAccounts></MetaDataObject>"#;
    let document = roxmltree::Document::parse(xml).unwrap();
    let object = document.root_element().first_element_child().unwrap();
    let target =
        MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "ChartOfAccounts.Accounts")
            .unwrap();
    let mut diagnostics = Vec::new();

    let declarations = serde_json::to_value(project_meta_info_declarations(
        MetadataKind::ChartOfAccounts,
        meta_info_child(object, "Properties"),
        &target,
        &mut diagnostics,
    ))
    .unwrap();

    assert!(declarations["standardTabularSections"].is_null());
    assert_eq!(diagnostics[0].code, MetaDiagnosticCode::ProviderUnavailable);
}

#[test]
fn non_empty_characteristics_are_preserved_in_a_closed_typed_shape() {
    let xml = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"><Catalog><Properties><Name>Items</Name><Characteristics><xr:Characteristic><xr:CharacteristicTypes from="Catalog.Sets.TabularSection.Fields.Attribute.Property"><xr:KeyField>InformationRegister.Values.Dimension.Property</xr:KeyField><xr:TypesFilterField>Catalog.Sets.TabularSection.Fields.Attribute.Property</xr:TypesFilterField><xr:TypesFilterValue xsi:type="xs:string">Catalog_Items</xr:TypesFilterValue><xr:DataPathField>-1</xr:DataPathField><xr:MultipleValuesUseField>-1</xr:MultipleValuesUseField></xr:CharacteristicTypes><xr:CharacteristicValues from="InformationRegister.Values"><xr:ObjectField>InformationRegister.Values.Dimension.Object</xr:ObjectField><xr:TypeField>InformationRegister.Values.Dimension.Property</xr:TypeField><xr:ValueField>InformationRegister.Values.Resource.Value</xr:ValueField><xr:MultipleValuesKeyField>-1</xr:MultipleValuesKeyField><xr:MultipleValuesOrderField>-1</xr:MultipleValuesOrderField></xr:CharacteristicValues></xr:Characteristic></Characteristics></Properties></Catalog></MetaDataObject>"#;

    let declarations = project_declarations(xml, MetadataKind::Catalog, "Catalog.Items");

    assert_eq!(
        declarations["characteristics"][0]["types"]["typesFilterValue"],
        serde_json::json!({
            "kind": "typed",
            "type": {"namespace": "http://www.w3.org/2001/XMLSchema", "localName": "string"},
            "value": "Catalog_Items"
        })
    );
    assert_eq!(
        declarations["characteristics"][0]["values"]["valueField"],
        "InformationRegister.Values.Resource.Value"
    );
}

#[test]
fn observed_type_rejects_a_defined_type_qname_with_the_wrong_metadata_kind() {
    let xml = r#"<Properties xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:cfg="http://v8.1c.ru/8.1/data/enterprise/current-config"><Type><v8:TypeSet>cfg:Catalog.Items</v8:TypeSet></Type></Properties>"#;
    let document = roxmltree::Document::parse(xml).unwrap();

    assert!(parse_observed_metadata_type_node(document.root_element()).is_err());
}

#[test]
fn root_details_localize_unknown_platform_qnames_as_warnings() {
    for type_value in ["v8:FutureOpaque", "cfg:FutureRef.Item"] {
        let xml = format!(
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:cfg="http://v8.1c.ru/8.1/data/enterprise/current-config"><Constant><Properties><Name>Value</Name><Type><v8:Type>{type_value}</v8:Type></Type></Properties></Constant></MetaDataObject>"#
        );
        let document = roxmltree::Document::parse(&xml).unwrap();
        let object = document.root_element().first_element_child().unwrap();
        let target =
            MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "Constant.Value").unwrap();
        let mut diagnostics = Vec::new();
        let details = serde_json::to_value(project_meta_info_details(
            MetadataKind::Constant,
            meta_info_child(object, "Properties"),
            None,
            &target,
            &mut diagnostics,
        ))
        .unwrap();

        assert!(details["details"]["type"].is_null(), "{type_value}");
        assert_eq!(
            serde_json::to_value(&diagnostics[0]).unwrap()["severity"],
            "warning",
            "{type_value}"
        );
        assert_eq!(diagnostics[0].field.as_deref(), Some("details.type"));
    }
}

#[test]
fn unknown_platform_type_with_a_malformed_qualifier_remains_an_error() {
    let xml = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core"><Constant><Properties><Name>Value</Name><Type><v8:Type>v8:FutureOpaque</v8:Type><v8:StringQualifiers><v8:Length>garbage</v8:Length></v8:StringQualifiers></Type></Properties></Constant></MetaDataObject>"#;
    let document = roxmltree::Document::parse(xml).unwrap();
    let object = document.root_element().first_element_child().unwrap();
    let target = MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "Constant.Value").unwrap();
    let mut diagnostics = Vec::new();

    let _ = project_meta_info_details(
        MetadataKind::Constant,
        meta_info_child(object, "Properties"),
        None,
        &target,
        &mut diagnostics,
    );

    assert_eq!(diagnostics.len(), 1);
    assert_eq!(
        diagnostics[0].severity,
        crate::domain::metadata::MetaDiagnosticSeverity::Error
    );
}

#[test]
fn observed_type_rejects_a_foreign_namespace_qualifier_decoy() {
    let xml = r#"<Properties xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:foreign="urn:foreign"><Type><v8:Type>xs:string</v8:Type><v8:StringQualifiers><foreign:Length>36</foreign:Length><v8:AllowedLength>Fixed</v8:AllowedLength></v8:StringQualifiers></Type></Properties>"#;
    let document = roxmltree::Document::parse(xml).unwrap();

    assert!(parse_observed_metadata_type_node(document.root_element()).is_err());
}

#[test]
fn observed_type_requires_present_qualifiers_to_be_complete_and_match_a_primitive() {
    for xml in [
        r#"<Properties xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:xs="http://www.w3.org/2001/XMLSchema"><Type><v8:Type>xs:string</v8:Type><v8:StringQualifiers><v8:Length>10</v8:Length></v8:StringQualifiers></Type></Properties>"#,
        r#"<Properties xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:xs="http://www.w3.org/2001/XMLSchema"><Type><v8:Type>xs:boolean</v8:Type><v8:StringQualifiers><v8:Length>10</v8:Length><v8:AllowedLength>Fixed</v8:AllowedLength></v8:StringQualifiers></Type></Properties>"#,
    ] {
        let document = roxmltree::Document::parse(xml).unwrap();
        assert!(
            parse_observed_metadata_type_node(document.root_element()).is_err(),
            "qualifiers must be complete and owned by a matching primitive: {xml}"
        );
    }

    let canonical_default = r#"<Properties xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:xs="http://www.w3.org/2001/XMLSchema"><Type><v8:Type>xs:string</v8:Type></Type></Properties>"#;
    let document = roxmltree::Document::parse(canonical_default).unwrap();
    assert!(
        parse_observed_metadata_type_node(document.root_element()).is_ok(),
        "the platform emitter uses omitted qualifiers for the canonical default string"
    );
}

#[test]
fn observed_type_rejects_nested_markup_in_type_and_qualifier_leaves() {
    for xml in [
        r#"<Properties xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:xs="http://www.w3.org/2001/XMLSchema"><Type><v8:Type>xs:boolean<v8:Unexpected/></v8:Type></Type></Properties>"#,
        r#"<Properties xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:xs="http://www.w3.org/2001/XMLSchema"><Type><v8:Type>xs:string</v8:Type><v8:StringQualifiers><v8:Length>10<v8:Unexpected/></v8:Length></v8:StringQualifiers></Type></Properties>"#,
    ] {
        let document = roxmltree::Document::parse(xml).unwrap();
        assert!(
            parse_observed_metadata_type_node(document.root_element()).is_err(),
            "nested markup must not be hidden by node.text(): {xml}"
        );
    }
}

#[test]
fn observed_type_rejects_attributes_and_mixed_text_on_type_containers() {
    for xml in [
        r#"<Properties xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:foreign="urn:foreign"><Type foreign:source="poison">ignored<v8:Type>xs:boolean</v8:Type></Type></Properties>"#,
        r#"<Properties xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:foreign="urn:foreign"><Type><v8:Type>xs:string</v8:Type><v8:StringQualifiers foreign:source="poison"><v8:Length>10</v8:Length><v8:AllowedLength>Fixed</v8:AllowedLength></v8:StringQualifiers></Type></Properties>"#,
    ] {
        let document = roxmltree::Document::parse(xml).unwrap();
        assert!(
            parse_observed_metadata_type_node(document.root_element()).is_err(),
            "container attributes or mixed text must fail closed: {xml}"
        );
    }
}

#[test]
fn unknown_platform_qnames_are_classified_by_uri_and_unbound_prefixes_are_rejected() {
    let unknown = r#"<Properties xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:core="http://v8.1c.ru/8.1/data/core"><Type><core:Type>core:FutureOpaque</core:Type></Type></Properties>"#;
    let document = roxmltree::Document::parse(unknown).unwrap();
    assert!(observed_type_is_strict_but_unmodelled(
        document.root_element()
    ));

    let unbound = r#"<Properties xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:core="http://v8.1c.ru/8.1/data/core"><Type><core:Type>v8:UUID</core:Type></Type></Properties>"#;
    let document = roxmltree::Document::parse(unbound).unwrap();
    assert!(parse_observed_metadata_type_node(document.root_element()).is_err());
}

#[test]
fn malformed_known_variants_are_not_downgraded_by_an_unknown_qname() {
    for xml in [
        r#"<Properties xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:cfg="http://v8.1c.ru/8.1/data/enterprise/current-config"><Type><v8:TypeSet>cfg:DefinedType.GLN.Form.Main</v8:TypeSet><v8:Type>v8:FutureOpaque</v8:Type></Type></Properties>"#,
        r#"<Properties xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core"><Type><v8:Type>v8:ValueStorage</v8:Type><v8:Type>v8:FutureOpaque</v8:Type></Type></Properties>"#,
    ] {
        let document = roxmltree::Document::parse(xml).unwrap();
        assert!(parse_observed_metadata_type_node(document.root_element()).is_err());
        assert!(
            !observed_type_is_strict_but_unmodelled(document.root_element()),
            "malformed known semantics must stay in the error branch: {xml}"
        );
    }
}

#[test]
fn details_reject_nested_markup_in_reference_and_qname_leaves() {
    let journal = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"><DocumentJournal><Properties><Name>Journal</Name><RegisteredDocuments><xr:Item xsi:type="xr:MDObjectRef">Document.Orders<xr:Unexpected/></xr:Item></RegisteredDocuments></Properties></DocumentJournal></MetaDataObject>"#;
    let document = roxmltree::Document::parse(journal).unwrap();
    let object = document.root_element().first_element_child().unwrap();
    let target =
        MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "DocumentJournal.Journal").unwrap();
    let mut diagnostics = Vec::new();
    let details = serde_json::to_value(project_meta_info_details(
        MetadataKind::DocumentJournal,
        meta_info_child(object, "Properties"),
        None,
        &target,
        &mut diagnostics,
    ))
    .unwrap();
    assert!(details["details"]["registeredDocuments"].is_null());
    assert_eq!(
        diagnostics[0].field.as_deref(),
        Some("details.registeredDocuments[0]")
    );

    let web = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core"><WebService><Properties><Name>Service</Name></Properties><ChildObjects><Operation><Properties><Name>Run</Name><XDTOReturningValueType>v8:Value<v8:Unexpected/></XDTOReturningValueType><Nillable>false</Nillable><Transactioned>false</Transactioned><ProcedureName>Run</ProcedureName></Properties><ChildObjects/></Operation></ChildObjects></WebService></MetaDataObject>"#;
    let document = roxmltree::Document::parse(web).unwrap();
    let object = document.root_element().first_element_child().unwrap();
    let target =
        MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "WebService.Service").unwrap();
    let mut diagnostics = Vec::new();
    let details = serde_json::to_value(project_meta_info_details(
        MetadataKind::WebService,
        meta_info_child(object, "Properties"),
        meta_info_child(object, "ChildObjects"),
        &target,
        &mut diagnostics,
    ))
    .unwrap();
    assert!(details["details"]["operations"].is_null());
    assert_eq!(
        diagnostics[0].field.as_deref(),
        Some("details.operations[0].returnType")
    );
}

#[test]
fn reference_and_xdto_entries_reject_unmodelled_attributes() {
    let journal = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xmlns:foreign="urn:foreign"><DocumentJournal><Properties><Name>Journal</Name><RegisteredDocuments><xr:Item xsi:type="xr:MDObjectRef" foreign:source="poison">Document.Orders</xr:Item></RegisteredDocuments></Properties></DocumentJournal></MetaDataObject>"#;
    let document = roxmltree::Document::parse(journal).unwrap();
    let object = document.root_element().first_element_child().unwrap();
    let target =
        MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "DocumentJournal.Journal").unwrap();
    let mut diagnostics = Vec::new();

    let details = serde_json::to_value(project_meta_info_details(
        MetadataKind::DocumentJournal,
        meta_info_child(object, "Properties"),
        None,
        &target,
        &mut diagnostics,
    ))
    .unwrap();

    assert!(details["details"]["registeredDocuments"].is_null());
    assert_eq!(diagnostics[0].code, MetaDiagnosticCode::ProviderUnavailable);
}

#[test]
fn root_reference_lists_reject_nested_metadata_addresses() {
    let journal = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"><DocumentJournal><Properties><Name>Journal</Name><RegisteredDocuments><xr:Item xsi:type="xr:MDObjectRef">Document.Orders.Form.Item</xr:Item></RegisteredDocuments></Properties></DocumentJournal></MetaDataObject>"#;
    let document = roxmltree::Document::parse(journal).unwrap();
    let object = document.root_element().first_element_child().unwrap();
    let target =
        MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "DocumentJournal.Journal").unwrap();
    let mut diagnostics = Vec::new();
    let details = serde_json::to_value(project_meta_info_details(
        MetadataKind::DocumentJournal,
        meta_info_child(object, "Properties"),
        None,
        &target,
        &mut diagnostics,
    ))
    .unwrap();
    assert!(details["details"]["registeredDocuments"].is_null());

    let web = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/platform_8_3_27/meta_info/edge/web-service.xml"
    ))
    .replace(
        "XDTOPackage.Exchange</xr:Value>",
        "XDTOPackage.Exchange.Form.Item</xr:Value>",
    );
    let document = roxmltree::Document::parse(&web).unwrap();
    let object = document.root_element().first_element_child().unwrap();
    let target =
        MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "WebService.Exchange").unwrap();
    let mut diagnostics = Vec::new();
    let details = serde_json::to_value(project_meta_info_details(
        MetadataKind::WebService,
        meta_info_child(object, "Properties"),
        meta_info_child(object, "ChildObjects"),
        &target,
        &mut diagnostics,
    ))
    .unwrap();
    assert!(details["details"]["xdtoPackages"].is_null());
}

#[test]
fn observed_type_does_not_scan_type_nodes_outside_the_direct_type_property() {
    let xml = r#"<Properties xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:xs="http://www.w3.org/2001/XMLSchema"><StandardAttributes><v8:Type>xs:boolean</v8:Type></StandardAttributes></Properties>"#;
    let document = roxmltree::Document::parse(xml).unwrap();

    assert!(parse_observed_metadata_type_node(document.root_element()).is_err());
}

#[test]
fn non_empty_type_without_a_known_variant_is_not_silently_treated_as_absent() {
    let xml = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:foreign="urn:foreign"><Constant><Properties><Name>Value</Name><Type><foreign:Type>foreign:Opaque</foreign:Type></Type></Properties></Constant></MetaDataObject>"#;
    let document = roxmltree::Document::parse(xml).unwrap();
    let object = document.root_element().first_element_child().unwrap();
    let target = MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "Constant.Value").unwrap();
    let mut diagnostics = Vec::new();

    let details = serde_json::to_value(project_meta_info_details(
        MetadataKind::Constant,
        meta_info_child(object, "Properties"),
        None,
        &target,
        &mut diagnostics,
    ))
    .unwrap();

    assert!(details["details"]["type"].is_null());
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].code, MetaDiagnosticCode::ProviderUnavailable);
}

#[test]
fn document_journal_details_preserve_registered_documents() {
    let xml = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"><DocumentJournal><Properties><Name>Journal</Name><RegisteredDocuments><xr:Item xsi:type="xr:MDObjectRef">Document.Sale</xr:Item></RegisteredDocuments></Properties></DocumentJournal></MetaDataObject>"#;

    let details = project(
        xml,
        MetadataKind::DocumentJournal,
        "DocumentJournal.Journal",
    );

    assert_eq!(
        details["registeredDocuments"],
        serde_json::json!(["Document.Sale"])
    );
}

#[test]
fn an_incomplete_http_collection_is_null_and_names_the_public_field() {
    let xml = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/platform_8_3_27/meta_info/edge/http-service.xml"
    ))
    .replace("<Handler>MetricsGet</Handler>", "<Handler/>");
    let document = roxmltree::Document::parse(&xml).unwrap();
    let object = document.root_element().first_element_child().unwrap();
    let target =
        MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "HTTPService.ExternalAPI").unwrap();
    let mut diagnostics = Vec::new();

    let value = serde_json::to_value(project_meta_info_details(
        MetadataKind::HTTPService,
        meta_info_child(object, "Properties"),
        meta_info_child(object, "ChildObjects"),
        &target,
        &mut diagnostics,
    ))
    .unwrap();

    assert!(value["details"]["urlTemplates"].is_null());
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(
        diagnostics[0].field.as_deref(),
        Some("details.urlTemplates[0].methods[0].handler")
    );
    assert_eq!(diagnostics[0].metadata_path.as_ref(), Some(&target));
}

#[test]
fn a_missing_http_methods_container_is_not_reported_as_an_empty_collection() {
    let xml = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses"><HTTPService><Properties><Name>ExternalAPI</Name></Properties><ChildObjects><URLTemplate><Properties><Name>Metrics</Name><Template>/metrics</Template></Properties></URLTemplate></ChildObjects></HTTPService></MetaDataObject>"#;
    let document = roxmltree::Document::parse(xml).unwrap();
    let object = document.root_element().first_element_child().unwrap();
    let target =
        MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "HTTPService.ExternalAPI").unwrap();
    let mut diagnostics = Vec::new();

    let value = serde_json::to_value(project_meta_info_details(
        MetadataKind::HTTPService,
        meta_info_child(object, "Properties"),
        meta_info_child(object, "ChildObjects"),
        &target,
        &mut diagnostics,
    ))
    .unwrap();

    assert!(value["details"]["urlTemplates"].is_null());
    assert_eq!(
        diagnostics[0].field.as_deref(),
        Some("details.urlTemplates[0].methods")
    );
}

#[test]
fn an_unexpected_nested_http_child_is_not_reported_as_an_empty_collection() {
    let xml = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses"><HTTPService><Properties><Name>ExternalAPI</Name></Properties><ChildObjects><URLTemplate><Properties><Name>Metrics</Name><Template>/metrics</Template></Properties><ChildObjects><Unexpected/></ChildObjects></URLTemplate></ChildObjects></HTTPService></MetaDataObject>"#;
    let document = roxmltree::Document::parse(xml).unwrap();
    let object = document.root_element().first_element_child().unwrap();
    let target =
        MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "HTTPService.ExternalAPI").unwrap();
    let mut diagnostics = Vec::new();

    let value = serde_json::to_value(project_meta_info_details(
        MetadataKind::HTTPService,
        meta_info_child(object, "Properties"),
        meta_info_child(object, "ChildObjects"),
        &target,
        &mut diagnostics,
    ))
    .unwrap();

    assert!(value["details"]["urlTemplates"].is_null());
    assert_eq!(
        diagnostics[0].field.as_deref(),
        Some("details.urlTemplates[0].methods")
    );
    assert_eq!(diagnostics[0].code, MetaDiagnosticCode::ProviderUnavailable);
}

#[test]
fn duplicate_nested_structure_has_a_stable_provider_unavailable_code() {
    let xml = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses"><HTTPService><Properties><Name>ExternalAPI</Name></Properties><ChildObjects><URLTemplate><Properties><Name>Metrics</Name><Template>/metrics</Template></Properties><ChildObjects/><ChildObjects/></URLTemplate></ChildObjects></HTTPService></MetaDataObject>"#;
    let document = roxmltree::Document::parse(xml).unwrap();
    let object = document.root_element().first_element_child().unwrap();
    let target =
        MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "HTTPService.ExternalAPI").unwrap();
    let mut diagnostics = Vec::new();

    let value = serde_json::to_value(project_meta_info_details(
        MetadataKind::HTTPService,
        meta_info_child(object, "Properties"),
        meta_info_child(object, "ChildObjects"),
        &target,
        &mut diagnostics,
    ))
    .unwrap();

    assert!(value["details"]["urlTemplates"].is_null());
    assert_eq!(diagnostics[0].code, MetaDiagnosticCode::ProviderUnavailable);
}

#[test]
fn nested_http_and_web_leaves_reject_unmodelled_attributes() {
    let xml = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:foreign="urn:foreign"><HTTPService><Properties><Name>ExternalAPI</Name></Properties><ChildObjects><URLTemplate><Properties><Name>Metrics</Name><Template>/metrics</Template></Properties><ChildObjects><Method><Properties><Name>Get</Name><HTTPMethod>GET</HTTPMethod><Handler foreign:source="poison">MetricsGet</Handler></Properties></Method></ChildObjects></URLTemplate></ChildObjects></HTTPService></MetaDataObject>"#;
    let document = roxmltree::Document::parse(xml).unwrap();
    let object = document.root_element().first_element_child().unwrap();
    let target =
        MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "HTTPService.ExternalAPI").unwrap();
    let mut diagnostics = Vec::new();

    let value = serde_json::to_value(project_meta_info_details(
        MetadataKind::HTTPService,
        meta_info_child(object, "Properties"),
        meta_info_child(object, "ChildObjects"),
        &target,
        &mut diagnostics,
    ))
    .unwrap();

    assert!(value["details"]["urlTemplates"].is_null());
    assert_eq!(diagnostics[0].code, MetaDiagnosticCode::ValidationFailed);
    assert_eq!(
        diagnostics[0].field.as_deref(),
        Some("details.urlTemplates[0].methods[0].handler")
    );
}

#[test]
fn a_missing_web_parameters_container_is_not_reported_as_an_empty_collection() {
    let xml = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:xs="http://www.w3.org/2001/XMLSchema"><WebService><Properties><Name>Exchange</Name></Properties><ChildObjects><Operation><Properties><Name>Ping</Name><XDTOReturningValueType>xs:boolean</XDTOReturningValueType><Nillable>false</Nillable><Transactioned>false</Transactioned><ProcedureName>Ping</ProcedureName></Properties></Operation></ChildObjects></WebService></MetaDataObject>"#;
    let document = roxmltree::Document::parse(xml).unwrap();
    let object = document.root_element().first_element_child().unwrap();
    let target =
        MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "WebService.Exchange").unwrap();
    let mut diagnostics = Vec::new();

    let value = serde_json::to_value(project_meta_info_details(
        MetadataKind::WebService,
        meta_info_child(object, "Properties"),
        meta_info_child(object, "ChildObjects"),
        &target,
        &mut diagnostics,
    ))
    .unwrap();

    assert!(value["details"]["operations"].is_null());
    assert_eq!(
        diagnostics[0].field.as_deref(),
        Some("details.operations[0].parameters")
    );
}

#[test]
fn an_unexpected_nested_web_child_is_not_reported_as_an_empty_collection() {
    let xml = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:xs="http://www.w3.org/2001/XMLSchema"><WebService><Properties><Name>Exchange</Name></Properties><ChildObjects><Operation><Properties><Name>Ping</Name><XDTOReturningValueType>xs:boolean</XDTOReturningValueType><Nillable>false</Nillable><Transactioned>false</Transactioned><ProcedureName>Ping</ProcedureName></Properties><ChildObjects><Unexpected/></ChildObjects></Operation></ChildObjects></WebService></MetaDataObject>"#;
    let document = roxmltree::Document::parse(xml).unwrap();
    let object = document.root_element().first_element_child().unwrap();
    let target =
        MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "WebService.Exchange").unwrap();
    let mut diagnostics = Vec::new();

    let value = serde_json::to_value(project_meta_info_details(
        MetadataKind::WebService,
        meta_info_child(object, "Properties"),
        meta_info_child(object, "ChildObjects"),
        &target,
        &mut diagnostics,
    ))
    .unwrap();

    assert!(value["details"]["operations"].is_null());
    assert_eq!(
        diagnostics[0].field.as_deref(),
        Some("details.operations[0].parameters")
    );
    assert_eq!(diagnostics[0].code, MetaDiagnosticCode::ProviderUnavailable);
}

#[test]
fn web_service_rejects_a_qname_with_an_invalid_local_name() {
    let xml = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/platform_8_3_27/meta_info/edge/web-service.xml"
    ))
    .replace("xs:boolean", "xs:1invalid");
    let document = roxmltree::Document::parse(&xml).unwrap();
    let object = document.root_element().first_element_child().unwrap();
    let target =
        MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "WebService.Exchange").unwrap();
    let mut diagnostics = Vec::new();

    let value = serde_json::to_value(project_meta_info_details(
        MetadataKind::WebService,
        meta_info_child(object, "Properties"),
        meta_info_child(object, "ChildObjects"),
        &target,
        &mut diagnostics,
    ))
    .unwrap();

    assert!(value["details"]["operations"].is_null());
    assert_eq!(
        diagnostics[0].field.as_deref(),
        Some("details.operations[0].returnType")
    );
    assert_eq!(diagnostics[0].code, MetaDiagnosticCode::ValidationFailed);
}

#[test]
fn expanded_xdto_names_do_not_depend_on_the_xml_prefix() {
    let original = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/platform_8_3_27/meta_info/edge/web-service.xml"
    ));
    let aliased = original
        .replace("xmlns:svc=", "xmlns:alternate=")
        .replace("svc:Payload", "alternate:Payload");

    let original_details = project(original, MetadataKind::WebService, "WebService.Exchange");
    let aliased_details = project(&aliased, MetadataKind::WebService, "WebService.Exchange");

    assert_eq!(aliased_details, original_details);
}

#[test]
fn profile_rejects_a_foreign_namespace_decoy_with_a_known_local_name() {
    let xml = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:foreign="urn:foreign"><Catalog><Properties><Name>Items</Name><foreign:Hierarchical>false</foreign:Hierarchical></Properties></Catalog></MetaDataObject>"#;
    let document = roxmltree::Document::parse(xml).unwrap();
    let object = document.root_element().first_element_child().unwrap();

    let error = validate_meta_info_profile(
        MetadataKind::Catalog,
        meta_info_child(object, "Properties"),
        None,
    )
    .unwrap_err();

    assert_eq!(error.field, "properties.Hierarchical");
}

#[test]
fn profile_rejects_a_malformed_known_scalar_instead_of_dropping_it() {
    let xml = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses"><Catalog><Properties><Name>Items</Name><Hierarchical>unknown</Hierarchical></Properties></Catalog></MetaDataObject>"#;
    let document = roxmltree::Document::parse(xml).unwrap();
    let object = document.root_element().first_element_child().unwrap();

    let error = validate_meta_info_profile(
        MetadataKind::Catalog,
        meta_info_child(object, "Properties"),
        None,
    )
    .unwrap_err();

    assert_eq!(error.field, "properties.Hierarchical");
}

#[test]
fn profile_rejects_a_boolean_suffix_hidden_after_an_xml_comment() {
    let xml = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses"><Catalog><Properties><Name>Items</Name><Hierarchical>true<!--split-->garbage</Hierarchical></Properties></Catalog></MetaDataObject>"#;
    let document = roxmltree::Document::parse(xml).unwrap();
    let object = document.root_element().first_element_child().unwrap();

    let error = validate_meta_info_profile(
        MetadataKind::Catalog,
        meta_info_child(object, "Properties"),
        None,
    )
    .unwrap_err();

    assert_eq!(error.field, "properties.Hierarchical");
}

#[test]
fn nested_http_names_use_the_complete_direct_text_value() {
    let xml = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/platform_8_3_27/meta_info/edge/http-service.xml"
    ))
    .replace("<Name>Metrics</Name>", "<Name>Met<!--split-->rics</Name>");

    let details = project(&xml, MetadataKind::HTTPService, "HTTPService.ExternalAPI");

    assert_eq!(details["urlTemplates"][0]["name"], "Metrics");
}

#[test]
fn http_url_template_name_must_be_a_platform_identifier() {
    let xml = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/platform_8_3_27/meta_info/edge/http-service.xml"
    ))
    .replace("<Name>Metrics</Name>", "<Name>bad name</Name>");
    let document = roxmltree::Document::parse(&xml).unwrap();
    let object = document.root_element().first_element_child().unwrap();
    let target =
        MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "HTTPService.ExternalAPI").unwrap();
    let mut diagnostics = Vec::new();

    let value = serde_json::to_value(project_meta_info_details(
        MetadataKind::HTTPService,
        meta_info_child(object, "Properties"),
        meta_info_child(object, "ChildObjects"),
        &target,
        &mut diagnostics,
    ))
    .unwrap();

    assert!(value["details"]["urlTemplates"].is_null());
    assert_eq!(
        diagnostics[0].field.as_deref(),
        Some("details.urlTemplates[0].name")
    );
}

#[test]
fn profile_rejects_a_known_string_property_with_nested_markup() {
    let xml = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses"><Catalog><Properties><Name>Items</Name><Comment><Unexpected>hidden</Unexpected></Comment></Properties></Catalog></MetaDataObject>"#;
    let document = roxmltree::Document::parse(xml).unwrap();
    let object = document.root_element().first_element_child().unwrap();

    let error = validate_meta_info_profile(
        MetadataKind::Catalog,
        meta_info_child(object, "Properties"),
        None,
    )
    .unwrap_err();

    assert_eq!(error.field, "properties.Comment");
}

#[test]
fn profile_rejects_nested_markup_in_every_scalar_property_kind() {
    let cases = [
        (
            "<Hierarchical>true<Unexpected/></Hierarchical>",
            "Hierarchical",
        ),
        ("<CodeLength>9<Unexpected/></CodeLength>", "CodeLength"),
        (
            "<Synonym><Unexpected><content>hidden</content></Unexpected></Synonym>",
            "Synonym",
        ),
    ];

    for (property, name) in cases {
        let xml = format!(
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses"><Catalog><Properties><Name>Items</Name>{property}</Properties></Catalog></MetaDataObject>"#
        );
        let document = roxmltree::Document::parse(&xml).unwrap();
        let object = document.root_element().first_element_child().unwrap();

        let error = validate_meta_info_profile(
            MetadataKind::Catalog,
            meta_info_child(object, "Properties"),
            None,
        )
        .unwrap_err();

        assert_eq!(error.field, format!("properties.{name}"), "{name}");
    }
}

#[test]
fn profile_rejects_duplicate_details_and_nested_text_leaves() {
    for (kind, xml) in [
        (
            MetadataKind::ScheduledJob,
            r#"<Properties xmlns="http://v8.1c.ru/8.3/MDClasses"><Name>Job</Name><MethodName>CommonModule.A.Run</MethodName><MethodName>CommonModule.B.Run</MethodName></Properties>"#,
        ),
        (
            MetadataKind::CalculationRegister,
            r#"<Properties xmlns="http://v8.1c.ru/8.3/MDClasses"><Name>Calc</Name><Schedule>InformationRegister.Schedule<Unexpected/></Schedule><ScheduleValue>InformationRegister.Schedule.Resource.Value</ScheduleValue><ScheduleDate>InformationRegister.Schedule.Dimension.Date</ScheduleDate></Properties>"#,
        ),
    ] {
        let document = roxmltree::Document::parse(xml).unwrap();
        assert!(
            validate_meta_info_profile(kind, Some(document.root_element()), None).is_err(),
            "{} must fail closed",
            kind.as_str()
        );
    }
}

#[test]
fn profile_rejects_localized_leaf_markup_duplicate_languages_and_scalar_attributes() {
    for xml in [
        r#"<Properties xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core"><Name>Items</Name><Synonym><v8:item><v8:lang><v8:x/></v8:lang><v8:content>Items</v8:content></v8:item></Synonym></Properties>"#,
        r#"<Properties xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core"><Name>Items</Name><Synonym><v8:item><v8:lang>ru</v8:lang><v8:content>One</v8:content></v8:item><v8:item><v8:lang>ru</v8:lang><v8:content>Two</v8:content></v8:item></Synonym></Properties>"#,
        r#"<Properties xmlns="http://v8.1c.ru/8.3/MDClasses"><Name>Items</Name><CodeLength source="foreign">9</CodeLength></Properties>"#,
    ] {
        let document = roxmltree::Document::parse(xml).unwrap();
        assert!(
            validate_meta_info_profile(MetadataKind::Catalog, Some(document.root_element()), None,)
                .is_err(),
            "{xml}"
        );
    }
}

#[test]
fn profile_rejects_a_known_child_object_under_the_wrong_kind() {
    let xml = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses"><Catalog><Properties><Name>Items</Name></Properties><ChildObjects><Operation><Properties><Name>Foreign</Name></Properties></Operation></ChildObjects></Catalog></MetaDataObject>"#;
    let document = roxmltree::Document::parse(xml).unwrap();
    let object = document.root_element().first_element_child().unwrap();

    let error = validate_meta_info_profile(
        MetadataKind::Catalog,
        meta_info_child(object, "Properties"),
        meta_info_child(object, "ChildObjects"),
    )
    .unwrap_err();

    assert_eq!(error.field, "collections.Operation");
}

#[test]
fn profile_rejects_child_objects_for_platform_childless_kinds() {
    for kind in [
        MetadataKind::CommonModule,
        MetadataKind::Constant,
        MetadataKind::DefinedType,
        MetadataKind::EventSubscription,
        MetadataKind::ScheduledJob,
    ] {
        let xml = format!(
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses"><{kind}><Properties><Name>Evidence</Name></Properties><ChildObjects/></{kind}></MetaDataObject>"#,
            kind = kind.as_str(),
        );
        let document = roxmltree::Document::parse(&xml).unwrap();
        let object = document.root_element().first_element_child().unwrap();

        let error = validate_meta_info_profile(
            kind,
            meta_info_child(object, "Properties"),
            meta_info_child(object, "ChildObjects"),
        )
        .unwrap_err();

        assert_eq!(error.field, "collections", "{}", kind.as_str());
        assert_eq!(error.code, MetaDiagnosticCode::ProviderUnavailable);
    }
}

#[test]
fn profile_rejects_a_relation_property_under_the_wrong_kind() {
    let xml = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses"><WebService><Properties><Name>Exchange</Name><Owners/></Properties></WebService></MetaDataObject>"#;
    let document = roxmltree::Document::parse(xml).unwrap();
    let object = document.root_element().first_element_child().unwrap();

    let error = validate_meta_info_profile(
        MetadataKind::WebService,
        meta_info_child(object, "Properties"),
        None,
    )
    .unwrap_err();

    assert_eq!(error.field, "properties.Owners");
}
