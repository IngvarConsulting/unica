//! Closed XML 2.20 observations for DocumentNumerator, independent of writes.

use crate::info_projection::{
    direct_meta_info_property, direct_text_content, meta_info_property_value_is_valid,
    ProjectionError,
};
use roxmltree::{Document, Node};
use unica_metadata::{MetaInfoPropertyValueKind, MetaObservedProperty, MetaObservedPropertyValue};

const MD: &str = "http://v8.1c.ru/8.3/MDClasses";
// 1c-configuration-spec.md §6.12: the numerator's own format profile.
#[derive(Clone, Copy)]
enum ValueKind {
    Enumeration(&'static [&'static str]),
    Boolean,
}

const PROFILE: &[(&str, ValueKind)] = &[
    ("NumberType", ValueKind::Enumeration(&["String", "Number"])),
    (
        "NumberAllowedLength",
        ValueKind::Enumeration(&["Variable", "Fixed"]),
    ),
    (
        "NumberPeriodicity",
        ValueKind::Enumeration(&["Nonperiodical", "Year", "Quarter", "Month", "Day"]),
    ),
    ("CheckUnique", ValueKind::Boolean),
];

#[derive(Default)]
pub struct NumeratorReadFacts {
    pub properties: Vec<MetaObservedProperty>,
    pub unreadable: Vec<ProjectionError>,
}

fn observe(
    properties: Node<'_, '_>,
    name: &str,
    value_kind: ValueKind,
) -> Result<Option<MetaObservedPropertyValue>, ProjectionError> {
    let Some(node) = direct_meta_info_property(properties, "DocumentNumerator", name)? else {
        return Ok(None);
    };
    let kind = match value_kind {
        ValueKind::Enumeration(_) => MetaInfoPropertyValueKind::String,
        ValueKind::Boolean => MetaInfoPropertyValueKind::Boolean,
    };
    let value = direct_text_content(node);
    if !meta_info_property_value_is_valid(node, kind)
        || matches!(value_kind, ValueKind::Enumeration(values) if !values.contains(&value.as_str()))
    {
        return Err((
            format!("properties.{name}"),
            format!("DocumentNumerator contains a malformed {name}"),
        )
            .into());
    }
    Ok(Some(match value_kind {
        ValueKind::Enumeration(_) => MetaObservedPropertyValue::Text { value },
        ValueKind::Boolean => MetaObservedPropertyValue::Boolean {
            value: value == "true",
        },
    }))
}

pub fn read(bytes: &[u8]) -> Result<NumeratorReadFacts, String> {
    let text = std::str::from_utf8(bytes).map_err(|error| error.to_string())?;
    let document =
        Document::parse(text.trim_start_matches('\u{feff}')).map_err(|error| error.to_string())?;
    let root = document.root_element();
    if !root.has_tag_name((MD, "MetaDataObject")) {
        return Err("DocumentNumerator descriptor has the wrong root".to_string());
    }
    let owners = root.children().filter(Node::is_element).collect::<Vec<_>>();
    let [owner] = owners.as_slice() else {
        return Err("DocumentNumerator descriptor must contain one owner".to_string());
    };
    if !owner.has_tag_name((MD, "DocumentNumerator")) {
        return Err("DocumentNumerator owner is absent".to_string());
    }
    let containers = owner
        .children()
        .filter(|node| node.is_element() && node.tag_name().name() == "Properties")
        .collect::<Vec<_>>();
    let properties = match containers.as_slice() {
        [node] if node.tag_name().namespace() == Some(MD) => *node,
        _ => return Err("DocumentNumerator must contain one direct Properties container in the metadata namespace".to_string()),
    };
    let mut facts = NumeratorReadFacts::default();
    for &(name, value_kind) in PROFILE {
        match observe(properties, name, value_kind) {
            Ok(Some(value)) => facts.properties.push(MetaObservedProperty {
                name: name.to_string(),
                value,
            }),
            Ok(None) => {}
            Err(error) => facts.unreadable.push(error),
        }
    }
    Ok(facts)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn descriptor(properties: &str) -> String {
        format!("<MetaDataObject xmlns=\"{MD}\" version=\"2.20\"><DocumentNumerator><Properties><Name>Probe</Name>{properties}</Properties></DocumentNumerator></MetaDataObject>")
    }

    const VALID: &str = "<NumberType>String</NumberType><NumberAllowedLength>Fixed</NumberAllowedLength><NumberPeriodicity>Year</NumberPeriodicity><CheckUnique>false</CheckUnique>";

    #[test]
    fn numerator_read_profile_keeps_all_explicit_values_including_false() {
        let facts = read(descriptor(VALID).as_bytes()).unwrap();
        assert_eq!(facts.properties.len(), 4);
        assert!(facts.unreadable.is_empty());
        assert_eq!(
            facts.properties[0].value,
            MetaObservedPropertyValue::Text {
                value: "String".to_string()
            }
        );
        assert_eq!(
            facts.properties[1].value,
            MetaObservedPropertyValue::Text {
                value: "Fixed".to_string()
            }
        );
        assert_eq!(
            facts.properties[2].value,
            MetaObservedPropertyValue::Text {
                value: "Year".to_string()
            }
        );
        assert_eq!(
            facts.properties[3].value,
            MetaObservedPropertyValue::Boolean { value: false }
        );
        let alternatives = "<NumberType>Number</NumberType><NumberAllowedLength>Variable</NumberAllowedLength><NumberPeriodicity>Month</NumberPeriodicity><CheckUnique>true</CheckUnique>";
        let facts = read(descriptor(alternatives).as_bytes()).unwrap();
        assert!(facts.unreadable.is_empty());
        assert_eq!(
            facts.properties[0].value,
            MetaObservedPropertyValue::Text {
                value: "Number".to_string()
            }
        );
        assert_eq!(
            facts.properties[1].value,
            MetaObservedPropertyValue::Text {
                value: "Variable".to_string()
            }
        );
        assert_eq!(
            facts.properties[2].value,
            MetaObservedPropertyValue::Text {
                value: "Month".to_string()
            }
        );
        assert_eq!(
            facts.properties[3].value,
            MetaObservedPropertyValue::Boolean { value: true }
        );
    }

    #[test]
    fn numerator_read_profile_distinguishes_missing_and_damaged_fields_independently() {
        for (name, value) in [
            ("NumberType", "String"),
            ("NumberAllowedLength", "Fixed"),
            ("NumberPeriodicity", "Year"),
            ("CheckUnique", "false"),
        ] {
            let original = format!("<{name}>{value}</{name}>");
            for raw in [
                format!("<{name}/>"),
                format!("<{name}>Unknown</{name}>"),
                format!("<{name}><Nested>{value}</Nested></{name}>"),
                format!("<{name} extra=\"bad\">{value}</{name}>"),
                format!("<{name} xmlns=\"urn:foreign\">{value}</{name}>"),
                format!("{original}{original}"),
                format!("<{name}> {value} </{name}>"),
            ] {
                let facts = read(descriptor(&VALID.replace(&original, &raw)).as_bytes()).unwrap();
                assert_eq!(facts.properties.len(), 3);
                assert!(!facts.properties.iter().any(|fact| fact.name == name));
                assert_eq!(facts.unreadable.len(), 1);
                assert_eq!(facts.unreadable[0].field, format!("properties.{name}"));
            }
            let facts = read(descriptor(&VALID.replace(&original, "")).as_bytes()).unwrap();
            assert_eq!(facts.properties.len(), 3);
            assert!(facts.unreadable.is_empty());
        }
    }

    #[test]
    fn numerator_read_profile_rejects_ambiguous_properties_and_observes_other_periods() {
        let valid = descriptor(VALID);
        for extra in ["<Properties/>", "<Properties xmlns=\"urn:foreign\"/>"] {
            let raw = valid.replace(
                "</DocumentNumerator>",
                &format!("{extra}</DocumentNumerator>"),
            );
            assert!(read(raw.as_bytes()).is_err());
        }
        for period in ["Nonperiodical", "Quarter", "Day"] {
            let raw = VALID.replace(
                "<NumberPeriodicity>Year</NumberPeriodicity>",
                &format!("<NumberPeriodicity>{period}</NumberPeriodicity>"),
            );
            let facts = read(descriptor(&raw).as_bytes()).unwrap();
            assert!(facts.unreadable.is_empty());
            assert_eq!(
                facts.properties[2].value,
                MetaObservedPropertyValue::Text {
                    value: period.to_string()
                }
            );
        }
    }
}
