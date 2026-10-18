use super::super::info::{parse_typed_meta_local_info, parse_typed_meta_local_info_for_source};
use crate::application::ports::MetaLocalInfo;
use crate::domain::metadata::{MetaElementData, MetaSupportStatus};
use crate::domain::project_sources::SourceSetKind;
use crate::domain::source_target::{MetadataAddress, PLATFORM_XML_8_3_27_FORMAT_2_20};
use serde_json::json;

// These are retained native platform exports, not output of Unica's writer.
const CATALOG: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tests/fixtures/unica_mcp_script_parity/bsp/meta/Catalogs/Валюты.xml"
));
const DOCUMENT: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tests/fixtures/unica_mcp_script_parity/bsp/meta/Documents/АктОбУничтоженииПерсональныхДанных.xml"
));

fn read(xml: &str, owner: &str, source_kind: Option<SourceSetKind>) -> MetaLocalInfo {
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

fn assert_no_pilot_properties(element: &MetaElementData) {
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

fn assert_profile_absent(info: &MetaLocalInfo) {
    assert!(info.member_read_profile.is_none(), "{info:?}");
    assert!(!info.collections.tabular_sections.is_empty(), "{info:?}");
    for section in &info.collections.tabular_sections {
        assert_no_pilot_properties(section);
        for column in &section.attributes {
            assert_no_pilot_properties(column);
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
    assert_profile_absent(&info);
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
        assert_no_pilot_properties(column);
    }
}
