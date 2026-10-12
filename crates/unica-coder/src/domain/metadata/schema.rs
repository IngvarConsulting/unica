//! JSON Schema of typed metadata values shared by the published operation
//! contracts of `apply` and the internal metadata edit request: one owner
//! for the type, fill-value and position shapes the writer accepts.

use super::{metadata_reference_type_kinds, MetadataKind, METADATA_XS_DATETIME_PATTERN};
use crate::domain::source_target::metadata_address_kind_spellings;
use serde_json::{json, Value};

pub(crate) fn metadata_kinds_object_pattern(kinds: &[MetadataKind]) -> String {
    format!(
        r"^({})\.[^.]+$",
        kinds
            .iter()
            .flat_map(|kind| {
                metadata_address_kind_spellings(kind.as_str())
                    .expect("relation target kind must have registered spellings")
            })
            .collect::<Vec<_>>()
            .join("|")
    )
}

pub(crate) fn position_schema() -> Value {
    json!({
        "oneOf": [
            {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "before": {
                        "type": "string",
                        "minLength": 1,
                        "description": "Name of the existing element that follows this element.",
                    },
                },
                "required": ["before"],
            },
            {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "after": {
                        "type": "string",
                        "minLength": 1,
                        "description": "Name of the existing element that precedes this element.",
                    },
                },
                "required": ["after"],
            },
        ],
        "description": "Exact insertion or movement anchor.",
    })
}

pub(crate) fn metadata_type_schema() -> Value {
    let at_most_one = [
        "string",
        "number",
        "boolean",
        "uuid",
        "date",
        "binaryData",
        "valueStorage",
    ]
    .into_iter()
    .map(|kind| {
        json!({
            "contains": {
                "type": "object",
                "properties": {"kind": {"enum": [kind]}},
                "required": ["kind"],
            },
            "minContains": 0,
            "maxContains": 1,
        })
    })
    .collect::<Vec<_>>();
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "variants": {
                "type": "array",
                "minItems": 1,
                "uniqueItems": true,
                "allOf": at_most_one,
                "if": {
                    "contains": {
                        "type": "object",
                        "properties": {"kind": {"enum": ["valueStorage"]}},
                        "required": ["kind"],
                    },
                },
                "then": {"maxItems": 1},
                "description": "One or more platform type variants.",
                "items": type_variant_schema(),
            },
        },
        "required": ["variants"],
        "description": "Structured 1C metadata type.",
    })
}

fn type_variant_schema() -> Value {
    let mut string = tagged_type_variant(
        "string",
        json!({
            "length": {"type": "integer", "minimum": 0, "maximum": 1024, "description": "Maximum string length."},
            "allowedLength": {"type": "string", "enum": ["variable", "fixed"], "description": "Whether the string length is variable or fixed."},
        }),
        &["kind", "length", "allowedLength"],
    );
    require_positive_fixed_length(&mut string);
    let mut binary_data = tagged_type_variant(
        "binaryData",
        json!({
            "length": {"type": "integer", "minimum": 0, "maximum": u32::MAX, "description": "Maximum binary data length."},
            "allowedLength": {"type": "string", "enum": ["variable", "fixed"], "description": "Whether the binary data length is variable or fixed."},
        }),
        &["kind", "length", "allowedLength"],
    );
    require_positive_fixed_length(&mut binary_data);
    let mut number = tagged_type_variant(
        "number",
        json!({
            "digits": {"type": "integer", "minimum": 0, "maximum": 38, "description": "Total number of decimal digits."},
            "fraction": {"type": "integer", "minimum": 0, "maximum": 38, "description": "Number of fractional decimal digits."},
            "sign": {"type": "string", "enum": ["any", "nonNegative"], "description": "Whether negative values are allowed."},
        }),
        &["kind", "digits", "fraction", "sign"],
    );
    number
        .as_object_mut()
        .expect("number type variant schema is always an object")
        .insert(
            "anyOf".into(),
            Value::Array(
                (0..=38)
                    .map(|digits| {
                        json!({
                            "properties": {
                                "digits": {"const": digits},
                                "fraction": {"maximum": digits},
                            },
                        })
                    })
                    .collect(),
            ),
        );
    json!({
        "oneOf": [
            string,
            number,
            tagged_type_variant("boolean", json!({}), &["kind"]),
            tagged_type_variant("uuid", json!({}), &["kind"]),
            tagged_type_variant(
                "date",
                json!({
                    "fractions": {"type": "string", "enum": ["date", "time", "dateTime"], "description": "Date and time fractions stored by the value."},
                }),
                &["kind", "fractions"],
            ),
            binary_data,
            tagged_type_variant("valueStorage", json!({}), &["kind"]),
            tagged_type_variant(
                "reference",
                json!({
                    "metadataPath": {
                        "type": "string",
                        "pattern": metadata_kinds_object_pattern(metadata_reference_type_kinds()),
                        "description": "Logical metadata path of an object kind that exposes a platform reference type."
                    },
                }),
                &["kind", "metadataPath"],
            ),
            tagged_type_variant(
                "definedType",
                json!({
                    "metadataPath": {
                        "type": "string",
                        "pattern": metadata_kinds_object_pattern(&[MetadataKind::DefinedType]),
                        "description": "Logical metadata path of the defined type."
                    },
                }),
                &["kind", "metadataPath"],
            ),
        ],
        "description": "One closed platform type variant.",
    })
}

fn require_positive_fixed_length(schema: &mut Value) {
    schema
        .as_object_mut()
        .expect("type variant schema is always an object")
        .insert(
            "allOf".into(),
            json!([{
                "if": {
                    "properties": {"allowedLength": {"enum": ["fixed"]}},
                    "required": ["allowedLength"],
                },
                "then": {"properties": {"length": {"minimum": 1}}},
            }]),
        );
}

fn tagged_type_variant(kind: &str, fields: Value, required: &[&str]) -> Value {
    let mut properties = fields
        .as_object()
        .expect("type variant fields are always an object")
        .clone();
    properties.insert(
        "kind".into(),
        json!({
            "type": "string",
            "enum": [kind],
        }),
    );
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": properties,
        "required": required,
    })
}

pub(crate) fn fill_value_schema() -> Value {
    json!({
        "oneOf": [
            tagged_fill_value_variant(
                "string",
                json!({"value": {"type": "string", "description": "String fill value."}}),
                &["kind", "value"],
            ),
            tagged_fill_value_variant(
                "number",
                json!({"value": {
                    "type": "string",
                    "pattern": r"^[+-]?[0-9]+(\.[0-9]+)?$",
                    "description": "Platform-formatted numeric fill value."
                }}),
                &["kind", "value"],
            ),
            tagged_fill_value_variant(
                "boolean",
                json!({"value": {"type": "boolean", "description": "Boolean fill value."}}),
                &["kind", "value"],
            ),
            tagged_fill_value_variant(
                "dateTime",
                json!({"value": {
                    "type": "string",
                    "pattern": METADATA_XS_DATETIME_PATTERN,
                    "description": "Platform-formatted date-time fill value."
                }}),
                &["kind", "value"],
            ),
            tagged_fill_value_variant(
                "reference",
                json!({"metadataPath": {"type": "string", "minLength": 1, "description": "Logical metadata path of the reference fill value."}}),
                &["kind", "metadataPath"],
            ),
        ],
        "description": "One closed fill-value variant.",
    })
}

fn tagged_fill_value_variant(kind: &str, fields: Value, required: &[&str]) -> Value {
    let mut properties = fields
        .as_object()
        .expect("fill-value variant fields are always an object")
        .clone();
    properties.insert(
        "kind".into(),
        json!({
            "type": "string",
            "enum": [kind],
            "description": "Discriminator for this fill-value variant.",
        }),
    );
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": properties,
        "required": required,
    })
}
