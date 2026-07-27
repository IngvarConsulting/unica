use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;
use serde_json::json;
use unica_format_core::{
    property::{
        deserialize_semantic_properties, property_definition, PropertyCapability,
        PropertyProvenance, PropertyValueState, SemanticProperty, SEMANTIC_PROPERTY_DEFINITIONS,
    },
    semantic_ids::SemanticPropertyId,
    value::{PrimitiveTypeKind, PropertyType, PropertyValue, TypeSetValue, TypeVariant},
};

#[derive(Deserialize)]
struct PropertyMapWire {
    #[serde(deserialize_with = "deserialize_semantic_properties")]
    properties: BTreeMap<SemanticPropertyId, SemanticProperty>,
}

#[test]
fn property_definition_registry_is_complete_unique_and_finite() {
    let ids = SEMANTIC_PROPERTY_DEFINITIONS
        .iter()
        .map(|definition| definition.id())
        .collect::<Vec<_>>();
    assert_eq!(ids, SemanticPropertyId::ALL);
    assert_eq!(
        ids.iter().copied().collect::<BTreeSet<_>>().len(),
        SemanticPropertyId::ALL.len()
    );
    for definition in SEMANTIC_PROPERTY_DEFINITIONS {
        assert!(!definition.allowed_types().is_empty());
        assert_eq!(
            definition
                .allowed_types()
                .iter()
                .copied()
                .collect::<BTreeSet<_>>()
                .len(),
            definition.allowed_types().len(),
            "{}",
            definition.id()
        );
        assert!(definition.accepts(definition.default_type()));
    }

    assert_eq!(
        property_definition(SemanticPropertyId::METADATA_NAME).allowed_types(),
        &[PropertyType::String]
    );
    assert_eq!(
        property_definition(SemanticPropertyId::FIELD_FILL_VALUE).allowed_types(),
        &[
            PropertyType::Boolean,
            PropertyType::Integer,
            PropertyType::Decimal,
            PropertyType::String,
            PropertyType::Uuid,
            PropertyType::Enum,
            PropertyType::Date,
            PropertyType::ObjectRef,
            PropertyType::List,
            PropertyType::Structure,
            PropertyType::Null,
            PropertyType::Unknown,
        ]
    );
}

#[test]
fn constructors_bind_id_type_value_state_provenance_and_capability() {
    let property = SemanticProperty::explicit(
        SemanticPropertyId::METADATA_NAME,
        PropertyValue::String("Items".to_string()),
    )
    .unwrap()
    .with_capability(PropertyCapability::ReadOnly)
    .unwrap();

    assert_eq!(property.id(), SemanticPropertyId::METADATA_NAME);
    assert_eq!(property.value_type(), PropertyType::String);
    assert_eq!(property.value_state(), PropertyValueState::Explicit);
    assert_eq!(property.provenance(), PropertyProvenance::Declared);
    assert_eq!(property.capability(), PropertyCapability::ReadOnly);
    assert_eq!(
        property.value(),
        Some(&PropertyValue::String("Items".to_string()))
    );
    assert!(property
        .validate_for(SemanticPropertyId::METADATA_NAME)
        .is_ok());
    assert!(property
        .validate_for(SemanticPropertyId::METADATA_COMMENT)
        .is_err());
    assert!(SemanticProperty::explicit(
        SemanticPropertyId::METADATA_NAME,
        PropertyValue::Integer(11),
    )
    .is_err());
    assert!(SemanticProperty::explicit(
        SemanticPropertyId::FIELD_FILL_VALUE,
        PropertyValue::TypeSet(
            TypeSetValue::new(vec![TypeVariant::primitive(
                PrimitiveTypeKind::Boolean,
                None
            )
            .unwrap()])
            .unwrap(),
        ),
    )
    .is_err());
}

#[test]
fn key_aware_deserialization_rejects_divergent_or_invalid_property_envelopes() {
    let valid = serde_json::from_value::<PropertyMapWire>(json!({
        "properties": {
            "metadata.name": {
                "type": "string",
                "valueState": "explicit",
                "value": {"type": "string", "value": "Items"},
                "provenance": "declared",
                "capability": "readOnly"
            }
        }
    }))
    .unwrap();
    assert_eq!(
        valid.properties[&SemanticPropertyId::METADATA_NAME].id(),
        SemanticPropertyId::METADATA_NAME
    );

    for invalid in [
        json!({
            "properties": {
                "metadata.name": {
                    "type": "string",
                    "valueState": "explicit",
                    "value": {"type": "integer", "value": 11},
                    "provenance": "declared",
                    "capability": "readOnly"
                }
            }
        }),
        json!({
            "properties": {
                "metadata.name": {
                    "type": "integer",
                    "valueState": "explicit",
                    "value": {"type": "integer", "value": 11},
                    "provenance": "declared",
                    "capability": "readOnly"
                }
            }
        }),
        json!({
            "properties": {
                "metadata.name": {
                    "type": "string",
                    "valueState": "explicit",
                    "provenance": "declared",
                    "capability": "readOnly"
                }
            }
        }),
        json!({
            "properties": {
                "metadata.name": {
                    "type": "string",
                    "valueState": "defaulted",
                    "value": {"type": "string", "value": "Items"},
                    "provenance": "declared",
                    "capability": "readOnly"
                }
            }
        }),
        json!({
            "properties": {
                "metadata.name": {
                    "type": "string",
                    "valueState": "absent",
                    "provenance": "declared",
                    "capability": "readOnly"
                }
            }
        }),
        json!({
            "properties": {
                "adapter.nativeName": {
                    "type": "string",
                    "valueState": "explicit",
                    "value": {"type": "string", "value": "Items"},
                    "provenance": "declared",
                    "capability": "readOnly"
                }
            }
        }),
    ] {
        assert!(
            serde_json::from_value::<PropertyMapWire>(invalid).is_err(),
            "invalid property envelope was accepted"
        );
    }
}

#[test]
fn property_map_round_trips_tagged_recursive_values_without_variant_loss() {
    let value = PropertyValue::Structure(BTreeMap::from([
        (
            "decimal".to_string(),
            PropertyValue::Decimal("10.250".to_string()),
        ),
        (
            "nested".to_string(),
            PropertyValue::List(vec![
                PropertyValue::Date("2026-07-27".to_string()),
                PropertyValue::Null,
            ]),
        ),
    ]));
    let property =
        SemanticProperty::explicit(SemanticPropertyId::FIELD_FILL_VALUE, value.clone()).unwrap();
    let encoded = serde_json::to_value(property).unwrap();
    let decoded = serde_json::from_value::<PropertyMapWire>(json!({
        "properties": {
            "field.fillValue": encoded
        }
    }))
    .unwrap();

    assert_eq!(
        decoded.properties[&SemanticPropertyId::FIELD_FILL_VALUE].value(),
        Some(&value)
    );
}
