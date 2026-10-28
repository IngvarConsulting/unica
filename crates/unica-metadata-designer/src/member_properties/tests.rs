use super::super::info::{parse_typed_meta_local_info, parse_typed_meta_local_info_for_source};

use serde_json::json;
use unica_address::source_target::{MetadataAddress, PLATFORM_XML_8_3_27_FORMAT_2_20};
use unica_metadata::{MetaElementData, MetaSupportStatus};
use unica_source_contract::SourceSetKind;

mod catalog_attribute;

// These are retained native platform exports, not output of Unica's writer.
const CATALOG: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tests/fixtures/unica_mcp_script_parity/bsp/meta/Catalogs/Валюты.xml"
));
const DOCUMENT: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tests/fixtures/unica_mcp_script_parity/bsp/meta/Documents/АктОбУничтоженииПерсональныхДанных.xml"
));

fn read(
    xml: &str,
    owner: &str,
    source_kind: Option<SourceSetKind>,
) -> unica_metadata::validation_subject::MetaLocalInfo {
    let address = MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, owner).unwrap();
    match source_kind {
        Some(kind) => parse_typed_meta_local_info_for_source(
            xml.as_bytes(),
            &address,
            MetaSupportStatus::Supported,
            kind,
        ),
        None => parse_typed_meta_local_info(xml.as_bytes(), &address, MetaSupportStatus::Supported),
    }
    .unwrap_or_else(|failure| panic!("{owner}: {failure:?}"))
}

fn assert_no_section_properties(element: &MetaElementData) {
    for name in ["LineNumberLength", "Use"] {
        assert!(
            !element
                .properties
                .iter()
                .any(|property| property.name == name),
            "unproved {name} on {}: {:?}",
            element.name,
            element.properties
        );
    }
}

fn assert_no_pilot_properties(element: &MetaElementData) {
    assert_no_section_properties(element);
    assert!(
        !element
            .properties
            .iter()
            .any(|property| property.name == "ToolTip"),
        "unproved ToolTip on {}: {:?}",
        element.name,
        element.properties
    );
}

fn assert_profile_absent(info: &unica_metadata::validation_subject::MetaLocalInfo) {
    assert!(info.member_read_profile.is_none(), "{info:?}");
    assert!(!info.collections.tabular_sections.is_empty(), "{info:?}");
    for section in &info.collections.tabular_sections {
        assert_no_pilot_properties(section);
        for column in &section.attributes {
            assert_no_pilot_properties(column);
            assert!(
                !column.properties.iter().any(|property| {
                    matches!(
                        property.name.as_str(),
                        "Indexing" | "FullTextSearch" | "DataHistory"
                    )
                }),
                "unproved attribute properties must remain unpublished"
            );
        }
    }
}

#[test]
fn catalog_tabular_native_context_observes_exact_platform_export_facts() {
    let info = read(
        CATALOG,
        "Catalog.Валюты",
        Some(SourceSetKind::Configuration),
    );
    assert!(info.member_read_profile.is_some());
    let section = info
        .collections
        .tabular_sections
        .iter()
        .find(|section| section.name == "Представления")
        .expect("retained platform export has this named section");
    assert_eq!(section.properties.len(), 2);
    let value = |name| {
        let property = section
            .properties
            .iter()
            .find(|property| property.name == name)
            .unwrap_or_else(|| panic!("missing {name}: {:?}", section.properties));
        serde_json::to_value(&property.value).unwrap()
    };
    assert_eq!(
        value("LineNumberLength"),
        json!({"kind": "unsignedInteger", "value": 5})
    );
    assert_eq!(value("Use"), json!({"kind": "text", "value": "ForItem"}));
    assert!(!section.attributes.is_empty());
}

#[test]
fn catalog_tabular_native_context_requires_proved_configuration_source_kind() {
    for source_kind in [
        None,
        Some(SourceSetKind::Extension),
        Some(SourceSetKind::ExternalProcessor),
        Some(SourceSetKind::ExternalReport),
    ] {
        let info = read(CATALOG, "Catalog.Валюты", source_kind);
        assert_profile_absent(&info);
        assert_eq!(info.name, "Валюты");
    }
}

#[test]
fn catalog_tabular_native_context_requires_exact_literal_export_version() {
    for version in ["2.19", "2.21", "02.20", "2&#x2e;20", ""] {
        let xml = CATALOG.replacen("version=\"2.20\"", &format!("version=\"{version}\""), 1);
        assert_ne!(xml, CATALOG, "the native export version must be replaced");
        let info = read(&xml, "Catalog.Валюты", Some(SourceSetKind::Configuration));
        assert_profile_absent(&info);
        assert_eq!(info.name, "Валюты");
    }
    let xml = CATALOG.replacen("version=\"2.20\"", "", 1);
    assert_ne!(xml, CATALOG);
    let info = read(&xml, "Catalog.Валюты", Some(SourceSetKind::Configuration));
    assert_profile_absent(&info);
}

#[test]
fn catalog_tabular_native_context_does_not_admit_other_platform_owner_kinds() {
    assert!(DOCUMENT.contains("<LineNumberLength>5</LineNumberLength>"));
    let info = read(
        DOCUMENT,
        "Document.АктОбУничтоженииПерсональныхДанных",
        Some(SourceSetKind::Configuration),
    );
    assert_eq!(
        info.member_read_profile,
        Some(unica_metadata::MetaMemberReadProfile::DocumentCommand220)
    );
    for section in &info.collections.tabular_sections {
        assert_no_pilot_properties(section);
        for attribute in &section.attributes {
            assert!(attribute.properties.is_empty());
        }
    }
    assert_eq!(info.name, "АктОбУничтоженииПерсональныхДанных");
}

#[test]
fn catalog_tabular_native_context_does_not_propagate_to_header_or_column_attributes() {
    let info = read(
        CATALOG,
        "Catalog.Валюты",
        Some(SourceSetKind::Configuration),
    );
    assert!(info.member_read_profile.is_some());
    assert!(!info.collections.attributes.is_empty());
    for attribute in &info.collections.attributes {
        assert_no_pilot_properties(attribute);
    }
    let columns = info
        .collections
        .tabular_sections
        .iter()
        .flat_map(|section| &section.attributes)
        .collect::<Vec<_>>();
    assert!(!columns.is_empty());
    for column in columns {
        assert_no_section_properties(column);
    }
}

#[test]
fn nested_member_profiles_observe_nine_facts_without_losing_false() {
    for (owner, member, fields, expected) in [
        ("Document", "Command", "<Group>NavigationPanelImportant</Group><ParameterUseMode>Single</ParameterUseMode><ModifiesData>false</ModifiesData>", json!({"Group":{"kind":"text","value":"NavigationPanelImportant"},"ParameterUseMode":{"kind":"text","value":"Single"},"ModifiesData":{"kind":"boolean","value":false}})),
        ("InformationRegister", "Dimension", "<Master>true</Master><MainFilter>true</MainFilter><DenyIncompleteValues>true</DenyIncompleteValues><TypeReductionMode>TransformValues</TypeReductionMode>", json!({"Master":{"kind":"boolean","value":true},"MainFilter":{"kind":"boolean","value":true},"DenyIncompleteValues":{"kind":"boolean","value":true},"TypeReductionMode":{"kind":"text","value":"TransformValues"}})),
        ("AccumulationRegister", "Dimension", "<UseInTotals>false</UseInTotals>", json!({"UseInTotals":{"kind":"boolean","value":false}})),
        ("DocumentJournal", "Column", "<Indexing>DontIndex</Indexing>", json!({"Indexing":{"kind":"text","value":"DontIndex"}})),
    ] {
        let xml = format!("<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.20\"><{owner} uuid=\"a1583000-0000-4000-8000-000000000001\"><Properties><Name>Owner</Name></Properties><ChildObjects><{member} uuid=\"a1583000-0000-4000-8000-000000000002\"><Properties><Name>Member</Name><Synonym/><FillChecking>DontCheck</FillChecking>{fields}</Properties></{member}></ChildObjects></{owner}></MetaDataObject>");
        let info = read(&xml, &format!("{owner}.Owner"), Some(SourceSetKind::Configuration));
        let item = match member { "Command" => &info.collections.commands[0], "Dimension" => &info.collections.dimensions[0], _ => &info.collections.columns[0] };
        let actual = item.properties.iter().map(|property| (property.name.clone(), serde_json::to_value(&property.value).unwrap())).collect::<serde_json::Map<_,_>>();
        assert_eq!(serde_json::Value::Object(actual), expected, "{owner}.{member}");
        assert_eq!(item.name, "Member");
    }
}

#[test]
fn nested_optional_fact_damage_is_local_and_missing_never_infers_defaults() {
    for (owner, member, facts) in [
        (
            "Document",
            "Command",
            vec![
                ("Group", "NavigationPanelImportant"),
                ("ParameterUseMode", "Single"),
                ("ModifiesData", "false"),
            ],
        ),
        (
            "InformationRegister",
            "Dimension",
            vec![
                ("Master", "true"),
                ("MainFilter", "true"),
                ("DenyIncompleteValues", "true"),
                ("TypeReductionMode", "TransformValues"),
            ],
        ),
        (
            "AccumulationRegister",
            "Dimension",
            vec![("UseInTotals", "false")],
        ),
        ("DocumentJournal", "Column", vec![("Indexing", "DontIndex")]),
    ] {
        let xml = |fields: &str| {
            format!("<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.20\"><{owner} uuid=\"a1583000-0000-4000-8000-000000000001\"><Properties><Name>Owner</Name></Properties><ChildObjects><{member} uuid=\"a1583000-0000-4000-8000-000000000002\"><Properties><Name>Member</Name><Synonym/><FillChecking>DontCheck</FillChecking>{fields}</Properties></{member}></ChildObjects></{owner}></MetaDataObject>")
        };
        let observed = |fields: &str| {
            let info = read(
                &xml(fields),
                &format!("{owner}.Owner"),
                Some(SourceSetKind::Configuration),
            );
            match member {
                "Command" => info.collections.commands[0].clone(),
                "Dimension" => info.collections.dimensions[0].clone(),
                _ => info.collections.columns[0].clone(),
            }
        };
        let empty = observed("");
        assert!(empty.properties.is_empty(), "{owner}: {empty:?}");
        assert!(empty.property_limits.is_empty(), "{owner}: {empty:?}");
        let fields = facts
            .iter()
            .map(|(tag, value)| format!("<{tag}>{value}</{tag}>"))
            .collect::<String>();
        for (tag, value) in &facts {
            let original = format!("<{tag}>{value}</{tag}>");
            for replacement in [
                format!("{original}{original}"),
                format!("<{tag} xmlns=\"urn:foreign\">{value}</{tag}>"),
                format!("<{tag}><Nested>{value}</Nested></{tag}>"),
                format!("<{tag}/>"),
                format!("<{tag}>UnsupportedValue</{tag}>"),
            ] {
                let item = observed(&fields.replace(&original, &replacement));
                assert_eq!(item.name, "Member");
                assert_eq!(
                    item.properties.len(),
                    facts.len() - 1,
                    "{owner} {tag}: {item:?}"
                );
                assert!(!item.properties.iter().any(|property| property.name == *tag));
                assert_eq!(item.property_limits.len(), 1, "{owner} {tag}: {item:?}");
                assert!(item.property_limits[0].contains(&format!(".properties.{tag}")));
                assert!(item.incomplete);
            }
            let item = observed(&fields.replace(&original, ""));
            assert_eq!(item.properties.len(), facts.len() - 1);
            assert!(item.property_limits.is_empty());
            if [
                "Master",
                "MainFilter",
                "DenyIncompleteValues",
                "ModifiesData",
                "UseInTotals",
            ]
            .contains(tag)
            {
                for boolean in ["true", "false"] {
                    let item =
                        observed(&fields.replace(&original, &format!("<{tag}>{boolean}</{tag}>")));
                    let property = item
                        .properties
                        .iter()
                        .find(|property| property.name == *tag)
                        .unwrap();
                    assert_eq!(
                        serde_json::to_value(&property.value).unwrap(),
                        json!({"kind":"boolean","value":boolean == "true"})
                    );
                }
            }
        }
    }
}
