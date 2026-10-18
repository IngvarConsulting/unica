use super::{read, CATALOG};
use crate::application::ports::MetaLocalInfo;
use crate::domain::metadata::MetaElementData;
use crate::domain::project_sources::SourceSetKind;
use serde_json::{json, Value};

const ENUMS: [(&str, &str, &[&str]); 3] = [
    (
        "Indexing",
        "Index",
        &["DontIndex", "Index", "IndexWithAdditionalOrder"],
    ),
    ("FullTextSearch", "Use", &["Use", "DontUse"]),
    ("DataHistory", "Use", &["Use", "DontUse"]),
];

fn column(info: &MetaLocalInfo) -> &MetaElementData {
    info.collections
        .tabular_sections
        .iter()
        .find(|section| section.name == "Представления")
        .unwrap()
        .attributes
        .iter()
        .find(|attribute| attribute.name == "КодЯзыка")
        .unwrap()
}

fn observed(column: &MetaElementData, name: &str) -> Option<Value> {
    column
        .properties
        .iter()
        .find(|property| property.name == name)
        .map(|property| serde_json::to_value(&property.value).unwrap())
}

fn replace_column_property(name: &str, replacement: &str) -> String {
    let doc = roxmltree::Document::parse(CATALOG).unwrap();
    let attribute = doc
        .descendants()
        .find(|node| {
            node.has_tag_name(("http://v8.1c.ru/8.3/MDClasses", "Attribute"))
                && node
                    .children()
                    .filter(|child| child.is_element())
                    .any(|properties| {
                        properties.tag_name().name() == "Properties"
                            && properties.children().any(|child| {
                                child.is_element()
                                    && child.tag_name().name() == "Name"
                                    && child.text() == Some("КодЯзыка")
                            })
                    })
        })
        .unwrap();
    let property = attribute
        .descendants()
        .find(|node| node.is_element() && node.tag_name().name() == name)
        .unwrap();
    let mut xml = CATALOG.to_string();
    xml.replace_range(property.range(), replacement);
    xml
}

fn read_catalog(xml: &str) -> MetaLocalInfo {
    read(xml, "Catalog.Валюты", Some(SourceSetKind::Configuration))
}

#[test]
fn catalog_attribute_native_profile_reads_each_closed_enum_value() {
    for (name, _, values) in ENUMS {
        for value in values {
            let xml = replace_column_property(name, &format!("<{name}>{value}</{name}>"));
            let info = read_catalog(&xml);
            let attribute = column(&info);
            assert_eq!(attribute.properties.len(), ENUMS.len());
            assert_eq!(
                observed(attribute, name),
                Some(json!({"kind": "text", "value": value}))
            );
            assert!(attribute.property_limits.is_empty());
            assert_eq!(attribute.required, Some(false));
            assert!(attribute.r#type.is_some());
        }
    }
}

#[test]
fn catalog_attribute_native_absence_is_optional_and_never_inherited() {
    for (name, _, _) in ENUMS {
        let info = read_catalog(&replace_column_property(name, ""));
        let attribute = column(&info);
        assert!(observed(attribute, name).is_none());
        assert!(attribute.property_limits.is_empty());
        for (independent, expected, _) in ENUMS {
            if independent != name {
                assert_eq!(
                    observed(attribute, independent),
                    Some(json!({"kind": "text", "value": expected}))
                );
            }
        }
    }
}

#[test]
fn catalog_attribute_native_damage_is_local_and_names_its_exact_field() {
    for (name, expected, _) in ENUMS {
        for replacement in [
            format!("<{name}/>"),
            format!("<{name} xsi:nil=\"true\"/>"),
            format!("<{name}>Unknown</{name}>"),
            format!("<{name}>{expected} </{name}>"),
            format!("<{name}>{expected}</{name}><{name}>{expected}</{name}>"),
            format!("<{name} unexpected=\"true\">{expected}</{name}>"),
            format!("<{name}><v8:Type>{expected}</v8:Type></{name}>"),
            format!("<{name} xmlns=\"urn:foreign\">{expected}</{name}>"),
            format!("<{name}>{expected}</{name}><{name} xmlns=\"urn:foreign\">{expected}</{name}>"),
        ] {
            let info = read_catalog(&replace_column_property(name, &replacement));
            let attribute = column(&info);
            assert!(observed(attribute, name).is_none(), "{replacement}");
            for (independent, expected, _) in ENUMS {
                if independent != name {
                    assert_eq!(
                        observed(attribute, independent),
                        Some(json!({"kind": "text", "value": expected})),
                        "{replacement}"
                    );
                }
            }
            assert!(attribute.incomplete, "{replacement}");
            assert_eq!(attribute.required, Some(false));
            assert!(attribute.r#type.is_some());
            let expected_field =
                format!("collections.tabularSections[0].attributes[0].properties.{name}");
            assert!(
                info.diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.field.as_deref() == Some(expected_field.as_str())),
                "{replacement}: the malformed fact must retain its diagnostic field"
            );
            assert!(
                attribute
                    .property_limits
                    .iter()
                    .any(|limit| limit.contains(&expected_field)),
                "{replacement}"
            );
        }
    }
}

#[test]
fn catalog_attribute_native_properties_stay_out_of_section_and_header_contexts() {
    let info = read_catalog(CATALOG);
    for section in &info.collections.tabular_sections {
        assert_eq!(section.properties.len(), 2);
        for (name, _, _) in ENUMS {
            assert!(observed(section, name).is_none());
        }
    }
    for attribute in &info.collections.attributes {
        for (name, _, _) in ENUMS {
            assert!(observed(attribute, name).is_none());
        }
    }
    for name in ["LineNumberLength", "Use", "ToolTip"] {
        assert!(observed(column(&info), name).is_none());
    }
}
