use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use serde_json::json;
use unica_format_core::{
    facets::SemanticFacets,
    navigation::{
        Authorability, CapabilityState, FacetSelection, IdentityStrength, NavigationEnvelope,
        NavigationNode, NavigationSelection, NavigationStatus, ObjectKey, ObjectRef,
        PropertySelection, RelationSelection, ResolutionState,
    },
    property::{PropertyCapability, PropertyProvenance, SemanticProperty},
    semantic_ids::{SemanticObjectKind, SemanticPropertyId, SemanticRelationId},
    source::{SnapshotConsistency, SourceId, SourceRevision, SourceSnapshot},
    value::{
        PropertyType, PropertyValue, StringLength, StringQualifiers, TypeQualifiers, TypeSetValue,
        TypeVariant,
    },
};

fn source_id() -> SourceId {
    SourceId::new("workspace:main").unwrap()
}

fn object_ref() -> ObjectRef {
    ObjectRef::new(
        source_id(),
        ObjectKey::new("uuid:11111111-1111-1111-1111-111111111111").unwrap(),
        IdentityStrength::Persistent,
        SemanticObjectKind::Document,
        "Shipment",
    )
}

#[test]
fn navigation_json_has_exact_top_level_fields_and_stable_semantic_keys() {
    let reference = object_ref();
    let mut node = NavigationNode::new(
        reference.clone(),
        CapabilityState::new(ResolutionState::Resolved, Authorability::DerivedReadOnly),
    );
    node.properties.insert(
        SemanticPropertyId::METADATA_NAME,
        SemanticProperty::explicit(
            PropertyType::String,
            PropertyValue::String("Shipment".to_string()),
            PropertyProvenance::Declared,
        )
        .unwrap()
        .with_capability(PropertyCapability::ReadOnly),
    );
    node.properties.insert(
        SemanticPropertyId::METADATA_SYNONYM,
        SemanticProperty::explicit(
            PropertyType::LocalizedString,
            PropertyValue::LocalizedString(BTreeMap::from([
                ("en".to_string(), "Shipment".to_string()),
                ("ru".to_string(), "Отгрузка".to_string()),
            ])),
            PropertyProvenance::Declared,
        )
        .unwrap(),
    );
    node.properties.insert(
        SemanticPropertyId::FIELD_TYPE,
        SemanticProperty::explicit(
            PropertyType::TypeSet,
            PropertyValue::TypeSet(TypeSetValue {
                variants: vec![TypeVariant::Primitive {
                    kind: unica_format_core::value::PrimitiveTypeKind::String,
                    qualifiers: Some(TypeQualifiers::String(StringQualifiers {
                        length: Some(20),
                        allowed_length: Some(StringLength::Variable),
                    })),
                }],
            }),
            PropertyProvenance::Declared,
        )
        .unwrap(),
    );
    node.facets = SemanticFacets::for_available(
        node.properties.keys().copied(),
        [SemanticRelationId::ATTRIBUTES],
    );

    let envelope = NavigationEnvelope {
        schema_version: "1".to_string(),
        status: NavigationStatus::Available,
        snapshot: Some(SourceSnapshot {
            source_id: source_id(),
            revision: SourceRevision::new("sha256:fixture").unwrap(),
            consistency: SnapshotConsistency::Consistent,
            adapter_id: "fixture".to_string(),
        }),
        root: Some(reference),
        nodes: vec![node],
        relations: Vec::new(),
        diagnostics: Vec::new(),
        relation_index: Arc::new(Vec::new()),
    };

    let value = serde_json::to_value(envelope).unwrap();
    assert_eq!(
        value
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([
            "schemaVersion".to_string(),
            "status".to_string(),
            "snapshot".to_string(),
            "root".to_string(),
            "nodes".to_string(),
            "relations".to_string(),
            "diagnostics".to_string(),
        ])
    );
    assert_eq!(value["status"], "ready");
    assert_eq!(value["nodes"][0]["objectRef"]["kind"], "document");
    assert_eq!(
        value["nodes"][0]["properties"]["metadata.name"],
        json!({
            "type": "string",
            "valueState": "explicit",
            "value": "Shipment",
            "provenance": "declared",
            "capability": "readOnly",
        })
    );
    assert_eq!(
        value["nodes"][0]["properties"]["metadata.synonym"]["value"],
        json!({"en": "Shipment", "ru": "Отгрузка"})
    );
    assert_eq!(
        value["nodes"][0]["properties"]["field.type"]["value"]["variants"][0],
        json!({
            "primitive": {
                "kind": "string",
                "qualifiers": {
                    "string": {"length": 20, "allowedLength": "variable"}
                }
            }
        })
    );
    assert_eq!(
        value["nodes"][0]["facets"]["identity"],
        json!(["metadata.name", "metadata.synonym"])
    );
    assert!(
        value["nodes"][0]["facets"]["identity"][0]
            .as_str()
            .is_some(),
        "facets contain identifiers only, never duplicate property values"
    );
}

#[test]
fn selections_serialize_registered_ids_and_cannot_hold_arbitrary_strings() {
    let selection = NavigationSelection {
        properties: PropertySelection::Named(BTreeSet::from([
            SemanticPropertyId::METADATA_NAME,
            SemanticPropertyId::DOCUMENT_NUMBER_LENGTH,
        ])),
        facets: FacetSelection::Summary,
        relations: vec![RelationSelection::new(SemanticRelationId::ATTRIBUTES, Some(10)).unwrap()],
    };

    assert_eq!(
        serde_json::to_value(selection).unwrap(),
        json!({
            "properties": {"named": ["document.number.length", "metadata.name"]},
            "facets": "summary",
            "relations": [{
                "kind": "contains",
                "role": "attributes",
                "pageSize": 10
            }]
        })
    );
    assert!(SemanticPropertyId::parse("native.xmlTag").is_none());
    assert!(SemanticRelationId::parse("adapter.children").is_none());
}

#[test]
fn every_property_value_state_uses_the_neutral_envelope() {
    let value = PropertyValue::Integer(11);
    let properties = [
        SemanticProperty::explicit(
            PropertyType::Integer,
            value.clone(),
            PropertyProvenance::Declared,
        )
        .unwrap(),
        SemanticProperty::defaulted(PropertyType::Integer, value.clone()).unwrap(),
        SemanticProperty::inherited(PropertyType::Integer, value.clone()).unwrap(),
        SemanticProperty::computed(PropertyType::Integer, value).unwrap(),
        SemanticProperty::absent(PropertyType::Integer),
        SemanticProperty::unresolved(PropertyType::Integer),
    ];
    let states = properties
        .into_iter()
        .map(|property| {
            serde_json::to_value(property).unwrap()["valueState"]
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect::<Vec<_>>();

    assert_eq!(
        states,
        [
            "explicit",
            "defaulted",
            "inherited",
            "computed",
            "absent",
            "unresolved"
        ]
    );
}
