use std::collections::BTreeMap;

use serde_json::json;
use unica_format_core::{
    navigation::{IdentityStrength, ObjectKey, ObjectRef},
    semantic_ids::{SemanticEnumValue, SemanticObjectKind},
    source::SourceId,
    value::{
        DateFractions, DateQualifiers, NumberQualifiers, NumberSign, PrimitiveTypeKind,
        PropertyValue, StringLength, StringQualifiers, TypeQualifiers, TypeSetValue, TypeVariant,
    },
};
use uuid::Uuid;

fn reference() -> ObjectRef {
    ObjectRef::new(
        SourceId::new("workspace:main").unwrap(),
        ObjectKey::new("uuid:11111111-1111-1111-1111-111111111111").unwrap(),
        IdentityStrength::Persistent,
        SemanticObjectKind::Catalog,
        "Products",
    )
}

#[test]
fn type_sets_accept_only_registered_targets_and_valid_qualifiers() {
    let reference = TypeVariant::reference(SemanticObjectKind::Catalog, "Products").unwrap();
    assert_eq!(
        reference.target().unwrap().kind(),
        SemanticObjectKind::Catalog
    );
    assert_eq!(reference.target().unwrap().name(), "Products");
    assert!(TypeVariant::reference(SemanticObjectKind::Enumeration, "Statuses").is_err());
    assert!(
        TypeVariant::reference(SemanticObjectKind::Catalog, "cfg:CatalogRef.Products").is_err()
    );
    assert!(TypeVariant::reference(SemanticObjectKind::Catalog, "Catalog.Products").is_err());

    let string = TypeVariant::primitive(
        PrimitiveTypeKind::String,
        Some(TypeQualifiers::String(
            StringQualifiers::new(Some(20), Some(StringLength::Variable)).unwrap(),
        )),
    )
    .unwrap();
    assert!(TypeVariant::primitive(
        PrimitiveTypeKind::Boolean,
        Some(TypeQualifiers::String(
            StringQualifiers::new(Some(20), None).unwrap(),
        )),
    )
    .is_err());
    assert!(StringQualifiers::new(Some(0), None).is_err());
    assert!(StringQualifiers::new(None, Some(StringLength::Fixed)).is_err());
    assert!(StringQualifiers::new(Some(0), Some(StringLength::Variable)).is_ok());
    assert!(NumberQualifiers::new(Some(0), None, None).is_err());
    assert!(NumberQualifiers::new(Some(3), Some(4), Some(NumberSign::Any)).is_err());
    assert!(DateQualifiers::new(None).is_err());
    assert!(TypeSetValue::new(Vec::new()).is_err());
    assert!(TypeSetValue::new(vec![string.clone(), string]).is_err());

    let type_set = TypeSetValue::new(vec![
        reference,
        TypeVariant::enumeration("Statuses").unwrap(),
        TypeVariant::defined_type("CustomerCode").unwrap(),
        TypeVariant::primitive(
            PrimitiveTypeKind::Number,
            Some(TypeQualifiers::Number(
                NumberQualifiers::new(Some(10), Some(2), Some(NumberSign::Nonnegative)).unwrap(),
            )),
        )
        .unwrap(),
        TypeVariant::primitive(
            PrimitiveTypeKind::Date,
            Some(TypeQualifiers::Date(
                DateQualifiers::new(Some(DateFractions::DateTime)).unwrap(),
            )),
        )
        .unwrap(),
    ])
    .unwrap();
    assert_eq!(
        serde_json::from_value::<TypeSetValue>(serde_json::to_value(&type_set).unwrap()).unwrap(),
        type_set
    );

    for malformed in [
        json!({"variants": [{"kind": "reference", "target": "cfg:CatalogRef.Products"}]}),
        json!({"variants": [{"kind": "reference", "target": {"kind": "catalog", "name": "cfg:CatalogRef.Products"}}]}),
        json!({"variants": [{"kind": "reference", "target": {"kind": "adapterCatalog", "name": "Products"}}]}),
        json!({"variants": [{"kind": "reference", "target": {"kind": "enumeration", "name": "Statuses"}}]}),
        json!({"variants": [{"kind": "enumeration", "target": {"kind": "catalog", "name": "Products"}}]}),
        json!({"variants": [{"kind": "primitive", "primitive": "boolean", "qualifiers": {"string": {"length": 10}}}]}),
        json!({"variants": [{"kind": "primitive", "primitive": "number", "qualifiers": {"number": {"digits": 3, "fractionDigits": 4}}}]}),
        json!({"variants": [{"kind": "primitive", "primitive": "date", "qualifiers": {"date": {}}}]}),
        json!({"variants": []}),
    ] {
        assert!(
            serde_json::from_value::<TypeSetValue>(malformed).is_err(),
            "malformed type set was accepted"
        );
    }
}

#[test]
fn recursive_property_values_round_trip_without_variant_loss() {
    let type_set = TypeSetValue::new(vec![
        TypeVariant::reference(SemanticObjectKind::Document, "Orders").unwrap(),
        TypeVariant::primitive(PrimitiveTypeKind::Boolean, None).unwrap(),
    ])
    .unwrap();
    let value = PropertyValue::Structure(BTreeMap::from([
        ("boolean".to_string(), PropertyValue::Boolean(true)),
        ("integer".to_string(), PropertyValue::Integer(42)),
        (
            "decimal".to_string(),
            PropertyValue::Decimal("-12.340".to_string()),
        ),
        (
            "string".to_string(),
            PropertyValue::String("text".to_string()),
        ),
        (
            "localized".to_string(),
            PropertyValue::LocalizedString(BTreeMap::from([
                ("en".to_string(), "Products".to_string()),
                ("ru".to_string(), "Товары".to_string()),
            ])),
        ),
        (
            "uuid".to_string(),
            PropertyValue::Uuid(Uuid::parse_str("22222222-2222-2222-2222-222222222222").unwrap()),
        ),
        (
            "enum".to_string(),
            PropertyValue::EnumSymbol(SemanticEnumValue::parse("number").unwrap()),
        ),
        (
            "date".to_string(),
            PropertyValue::Date("2026-07-27T12:34:56Z".to_string()),
        ),
        ("typeSet".to_string(), PropertyValue::TypeSet(type_set)),
        (
            "objectRef".to_string(),
            PropertyValue::ObjectRef(reference()),
        ),
        (
            "list".to_string(),
            PropertyValue::List(vec![
                PropertyValue::Decimal("1.25".to_string()),
                PropertyValue::Date("2026-07-27".to_string()),
                PropertyValue::List(vec![PropertyValue::Uuid(
                    Uuid::parse_str("33333333-3333-3333-3333-333333333333").unwrap(),
                )]),
                PropertyValue::Structure(BTreeMap::from([(
                    "state".to_string(),
                    PropertyValue::EnumSymbol(SemanticEnumValue::parse("string").unwrap()),
                )])),
            ]),
        ),
        ("null".to_string(), PropertyValue::Null),
        (
            "unknown".to_string(),
            PropertyValue::Unknown {
                summary: "not projected".to_string(),
            },
        ),
    ]));

    let encoded = serde_json::to_value(&value).unwrap();
    assert_eq!(encoded["type"], "structure");
    assert_eq!(encoded["value"]["decimal"]["type"], "decimal");
    assert_eq!(encoded["value"]["list"]["value"][0]["type"], "decimal");
    assert_eq!(
        serde_json::from_value::<PropertyValue>(encoded).unwrap(),
        value
    );
}

#[test]
fn recursive_property_value_deserialization_rejects_ambiguous_or_invalid_json() {
    for malformed in [
        json!("plain string"),
        json!({"type": "adapterValue", "value": "x"}),
        json!({"type": "decimal", "value": 1.25}),
        json!({"type": "decimal", "value": "NaN"}),
        json!({"type": "date", "value": "not-a-date"}),
        json!({"type": "uuid", "value": "not-a-uuid"}),
        json!({"type": "enum", "value": "adapter.native"}),
        json!({"type": "typeSet", "value": {"variants": []}}),
        json!({"type": "list", "value": ["untagged"]}),
        json!({"type": "structure", "value": {"nested": {"value": "missing-tag"}}}),
        json!({"type": "unknown", "value": {"summary": ""}}),
        json!({"type": "null", "value": null}),
    ] {
        let description = malformed.to_string();
        assert!(
            serde_json::from_value::<PropertyValue>(malformed).is_err(),
            "malformed recursive value was accepted: {description}"
        );
    }
}
