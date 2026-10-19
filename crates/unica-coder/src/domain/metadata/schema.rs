//! JSON Schema of typed metadata values shared by the published operation
//! contracts of `apply` and the internal metadata edit request: one owner
//! for the type, fill-value, position and relation shapes the writer accepts.

use super::{
    metadata_reference_type_kinds, metadata_relation_specs, EventSourceClass, MetaEditOperationTag,
    MetaPropertyValueKind, MetaRelation, MetaRelationTargetPolicy, MetadataKind, RelationEditMode,
    METADATA_PROPERTY_SPECS, METADATA_XS_DATETIME_PATTERN,
};
use crate::domain::source_target::{metadata_address_kind_spellings, RECALCULATION_KIND_SPELLINGS};
use serde_json::{json, Map, Value};
use std::sync::OnceLock;

/// Project the existing property vocabulary, narrowed before merging any
/// owner-specific enum. The legacy host consumes the kind-agnostic union.
pub(crate) fn metadata_property_schemas(kind: Option<MetadataKind>) -> Map<String, Value> {
    let mut root_properties: Map<String, Value> = Map::new();
    for spec in METADATA_PROPERTY_SPECS
        .iter()
        .filter(|spec| kind.is_none_or(|kind| spec.allowed_kinds.contains(&kind)))
    {
        let entry = root_properties
            .entry(spec.public_name.to_string())
            .or_insert_with(|| match spec.value_kind {
                MetaPropertyValueKind::Type => metadata_type_schema(),
                MetaPropertyValueKind::String => json!({"type": "string"}),
                MetaPropertyValueKind::Boolean => json!({"type": "boolean"}),
                MetaPropertyValueKind::UnsignedInteger => {
                    json!({"type": "integer", "minimum": 0, "maximum": u32::MAX})
                }
            })
            .as_object_mut()
            .expect("root property schema is always an object");
        if let Some(pattern) = spec.string_pattern {
            entry.insert("pattern".into(), Value::String(pattern.to_string()));
        }
        if spec.enum_values.is_empty() {
            // A kind that accepts a free value widens the published name.
            entry.remove("enum");
            entry.insert("unconstrained".into(), Value::Bool(true));
            continue;
        }
        if entry.contains_key("unconstrained") {
            continue;
        }
        let merged = entry
            .remove("enum")
            .and_then(|value| match value {
                Value::Array(values) => Some(values),
                _ => None,
            })
            .unwrap_or_default();
        let mut values = merged;
        for candidate in spec.enum_values {
            let candidate = Value::String((*candidate).to_string());
            if !values.contains(&candidate) {
                values.push(candidate);
            }
        }
        entry.insert("enum".into(), Value::Array(values));
    }
    for value in root_properties.values_mut() {
        value
            .as_object_mut()
            .expect("root property schema is always an object")
            .remove("unconstrained");
    }
    root_properties
}

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

fn relation_fields() -> Value {
    json!({
        "relation": {
            "type": "string",
            "enum": MetaRelation::ALL.iter().copied().map(MetaRelation::as_str).collect::<Vec<_>>(),
        },
        "mode": {
            "type": "string",
            "enum": RelationEditMode::ALL.iter().copied().map(RelationEditMode::as_str).collect::<Vec<_>>(),
        },
        "targets": {"type": "array", "uniqueItems": true},
    })
}

pub(crate) fn metadata_relation_operation_input_schema() -> &'static Value {
    static SCHEMA: OnceLock<Value> = OnceLock::new();
    SCHEMA.get_or_init(|| {
        json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["relation", "mode", "targets"],
            "properties": relation_fields(),
        })
    })
}

/// Structural union read before owner/relation correlation by the typed decoder.
pub(crate) fn metadata_relation_values_input_schema() -> &'static Value {
    static SCHEMA: OnceLock<Value> = OnceLock::new();
    SCHEMA.get_or_init(|| {
        let mut fields = metadata_relation_operation_input_schema()["properties"].clone();
        fields.as_object_mut().unwrap().remove("mode");
        fields["targets"]["items"] = json!({
            "oneOf": [
                metadata_relation_object_target_input_schema(),
                metadata_relation_field_target_input_schema(),
                metadata_event_source_input_schema(),
            ],
        });
        json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["relation", "targets"],
            "properties": fields,
        })
    })
}

/// The historical host envelope, with the same correlated relation branches.
pub(crate) fn metadata_relation_operation_schema() -> Value {
    let mut fields = json!({
        "op": {
            "type": "string",
            "enum": [MetaEditOperationTag::EditRelations.as_str()],
        },
    });
    fields.as_object_mut().unwrap().extend(
        metadata_relation_operation_input_schema()["properties"]
            .as_object()
            .unwrap()
            .clone(),
    );
    json!({
        "type": "object",
        "additionalProperties": false,
        "description": "Edit one relation. Each relation selects its own mode and target contract; EventSubscription source supports non-empty add/remove/replace requests; removing the last logical source is refused.",
        "required": ["op", "relation", "mode", "targets"],
        "properties": fields,
        "oneOf": MetaRelation::ALL.iter().copied().map(metadata_relation_schema_branch).collect::<Vec<_>>(),
    })
}

/// Canonical relation values narrowed by the metadata owner's capability profile.
pub(crate) fn metadata_relation_values_schema(owner: Option<MetadataKind>) -> Value {
    let relations = owner.map_or_else(
        || MetaRelation::ALL.to_vec(),
        |owner| {
            metadata_relation_specs(owner)
                .iter()
                .map(|spec| spec.relation)
                .collect()
        },
    );
    let mut schema = metadata_relation_operation_schema();
    let properties = schema["properties"].as_object_mut().unwrap();
    properties.remove("op");
    properties.remove("mode");
    schema["required"] = json!(["relation", "targets"]);
    schema.as_object_mut().unwrap().remove("description");
    if relations.is_empty() {
        schema["properties"]["relation"] = json!({"type": "string", "not": {}});
        schema.as_object_mut().unwrap().remove("oneOf");
    } else {
        schema["properties"]["relation"]["enum"] = json!(relations
            .iter()
            .copied()
            .map(MetaRelation::as_str)
            .collect::<Vec<_>>());
        schema["oneOf"] = Value::Array(
            relations
                .into_iter()
                .map(|relation| {
                    let mut branch = metadata_relation_schema_branch(relation);
                    branch["properties"].as_object_mut().unwrap().remove("mode");
                    branch
                })
                .collect(),
        );
    }
    schema
}

fn metadata_relation_schema_branch(relation: MetaRelation) -> Value {
    let mut fields = relation_fields();
    fields["relation"] = json!({"const": relation.as_str()});
    fields["targets"] = if relation == MetaRelation::Source {
        json!({
            "type": "array",
            "minItems": 1,
            "uniqueItems": true,
            "items": metadata_event_source_target_schema(),
            "description": "Replace the source with one or more logical event source targets.",
        })
    } else {
        json!({
            "type": "array",
            "minItems": 1,
            "uniqueItems": true,
            "items": metadata_relation_target_schema(relation),
        })
    };
    json!({"properties": fields})
}

fn path_target_input_schema(field: &str) -> Value {
    let mut fields = serde_json::Map::new();
    fields.insert(field.into(), json!({"type": "string"}));
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": [field],
        "properties": fields,
    })
}

pub(crate) fn metadata_relation_object_target_input_schema() -> &'static Value {
    static SCHEMA: OnceLock<Value> = OnceLock::new();
    SCHEMA.get_or_init(|| path_target_input_schema("metadataPath"))
}

pub(crate) fn metadata_relation_field_target_input_schema() -> &'static Value {
    static SCHEMA: OnceLock<Value> = OnceLock::new();
    SCHEMA.get_or_init(|| path_target_input_schema("fieldPath"))
}

fn metadata_relation_target_schema(relation: MetaRelation) -> Value {
    let policies = MetadataKind::ALL
        .iter()
        .copied()
        .flat_map(metadata_relation_specs)
        .filter(|spec| spec.relation == relation)
        .map(|spec| spec.target_policy)
        .collect::<Vec<_>>();
    let policy = *policies
        .first()
        .expect("every public relation has a capability policy");
    assert!(
        policies.iter().all(|candidate| *candidate == policy),
        "one public relation must not publish conflicting target policies"
    );
    match policy {
        MetaRelationTargetPolicy::MetadataKinds(kinds) => {
            let mut schema = metadata_relation_object_target_input_schema().clone();
            schema["properties"]["metadataPath"]["pattern"] =
                json!(metadata_kinds_object_pattern(kinds));
            schema
        }
        MetaRelationTargetPolicy::SameOwnerField => {
            let mut schema = metadata_relation_field_target_input_schema().clone();
            schema["properties"]["fieldPath"]["pattern"] =
                json!(r"^[^.]+\.[^.]+\.(Attribute|StandardAttribute)\.[^.]+$");
            schema
        }
        MetaRelationTargetPolicy::EventSources => metadata_event_source_target_schema(),
    }
}

fn event_source_class_schema() -> Value {
    json!({
        "type": "string",
        "enum": EventSourceClass::ALL.iter().copied().map(EventSourceClass::as_str).collect::<Vec<_>>(),
    })
}

/// Raw manager sourceClass stays optional; its path/class correlation is a domain check.
pub(crate) fn metadata_event_source_input_schema() -> &'static Value {
    static SCHEMA: OnceLock<Value> = OnceLock::new();
    SCHEMA.get_or_init(|| {
        let mut variants = ["object", "manager", "recordSet", "definedType"]
            .into_iter()
            .map(|kind| {
                let mut schema = metadata_relation_object_target_input_schema().clone();
                schema["required"] = json!(["kind", "metadataPath"]);
                schema["properties"]["kind"] = json!({"type": "string", "enum": [kind]});
                if kind == "manager" {
                    schema["properties"]["sourceClass"] = event_source_class_schema();
                }
                schema
            })
            .collect::<Vec<_>>();
        variants.push(json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["kind", "sourceClass"],
            "properties": {
                "kind": {"type": "string", "enum": ["family"]},
                "sourceClass": event_source_class_schema(),
            },
        }));
        json!({"oneOf": variants})
    })
}

fn event_source_schema_variant(kind: &str) -> Value {
    metadata_event_source_input_schema()["oneOf"]
        .as_array()
        .unwrap()
        .iter()
        .find(|variant| variant["properties"]["kind"]["enum"] == json!([kind]))
        .expect("the raw event source union owns each host variant")
        .clone()
}

fn metadata_event_source_target_schema() -> Value {
    let metadata_object = |kind: &str, pattern: String| {
        let mut schema = event_source_schema_variant(kind);
        let mut path = schema["properties"]["metadataPath"].clone();
        path["pattern"] = json!(pattern);
        schema["properties"] = json!({
            "kind": {"const": kind},
            "metadataPath": path,
        });
        schema
    };
    let named = r"[_A-Za-zА-Яа-яЁё][_A-Za-zА-Яа-яЁё0-9]*";
    let spelling_group = |values: &[&str]| {
        values
            .iter()
            .flat_map(|value| {
                metadata_address_kind_spellings(value)
                    .unwrap_or_else(|| panic!("event source root {value} must be registered"))
            })
            .collect::<Vec<_>>()
            .join("|")
    };
    let roots = |values: &[&str]| format!(r"^({})\.{named}$", spelling_group(values));
    let mut constant = metadata_object("manager", roots(&["Constant"]));
    constant["required"] = json!(["kind", "metadataPath", "sourceClass"]);
    constant["properties"]["sourceClass"] =
        event_source_schema_variant("manager")["properties"]["sourceClass"].clone();
    constant["properties"]["sourceClass"]["enum"] =
        json!(["constantManager", "constantValueManager"]);
    let mut family = event_source_schema_variant("family");
    family["properties"]["kind"] = json!({"const": "family"});
    json!({
        "oneOf": [
            metadata_object("object", roots(&[
                "Catalog", "Document", "ChartOfAccounts", "ChartOfCharacteristicTypes",
                "ChartOfCalculationTypes", "ExchangePlan", "BusinessProcess", "Task", "Report", "DataProcessor",
            ])),
            metadata_object("manager", roots(&[
                "Catalog", "Document", "Enum", "InformationRegister", "AccumulationRegister",
                "AccountingRegister", "CalculationRegister", "ChartOfAccounts", "ChartOfCharacteristicTypes",
                "ChartOfCalculationTypes", "BusinessProcess", "Task", "ExchangePlan", "DocumentJournal",
                "Report", "DataProcessor", "FilterCriterion", "SettingsStorage",
            ])),
            constant,
            metadata_object("recordSet", format!(
                r"^(({})\.{named}|({})\.{named}\.({})\.{named})$",
                spelling_group(&["InformationRegister", "AccumulationRegister", "AccountingRegister", "CalculationRegister", "Sequence"]),
                spelling_group(&["CalculationRegister"]), RECALCULATION_KIND_SPELLINGS.join("|"),
            )),
            metadata_object("definedType", roots(&["DefinedType"])),
            family,
        ],
        "description": "One member of the closed typed EventSubscription source algebra.",
    })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relation_values_schema_follows_each_owner_without_empty_alternatives() {
        for owner in MetadataKind::ALL.iter().copied() {
            let schema = metadata_relation_values_schema(Some(owner));
            let validator = jsonschema::validator_for(&schema).expect("contextual relation schema");
            let relations = metadata_relation_specs(owner)
                .iter()
                .map(|spec| spec.relation.as_str())
                .collect::<Vec<_>>();
            assert_eq!(schema["required"], json!(["relation", "targets"]));
            assert_eq!(schema["additionalProperties"], false);
            assert!(schema["properties"].get("mode").is_none());
            if relations.is_empty() {
                assert_eq!(
                    schema["properties"]["relation"],
                    json!({"type": "string", "not": {}})
                );
                assert!(schema.get("oneOf").is_none());
                assert!(
                    !validator.is_valid(&json!({
                        "relation": "owners", "targets": [{"metadataPath": "Catalog.Parent"}],
                    })),
                    "{owner:?}"
                );
            } else {
                assert_eq!(schema["properties"]["relation"]["enum"], json!(relations));
                assert_eq!(schema["oneOf"].as_array().unwrap().len(), relations.len());
                assert!(schema["oneOf"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|branch| { branch["properties"].get("mode").is_none() }));
            }
        }
    }

    #[test]
    fn raw_relation_union_does_not_preempt_owner_or_event_source_correlation() {
        let raw = jsonschema::validator_for(metadata_relation_values_input_schema()).unwrap();
        let catalog = jsonschema::validator_for(&metadata_relation_values_schema(Some(
            MetadataKind::Catalog,
        )))
        .unwrap();
        let source = jsonschema::validator_for(&metadata_relation_values_schema(Some(
            MetadataKind::EventSubscription,
        )))
        .unwrap();
        for values in [
            json!({"relation": "owners", "targets": [{"fieldPath": "Catalog.Items.Attribute.Code"}]}),
            json!({"relation": "source", "targets": [{"kind": "manager", "metadataPath": "Constant.Flag"}]}),
            json!({"relation": "source", "targets": [{"kind": "manager", "metadataPath": "Catalog.Items", "sourceClass": "constantManager"}]}),
        ] {
            assert!(
                raw.is_valid(&values),
                "raw typed input must reach domain correlation: {values}"
            );
            assert!(
                !catalog.is_valid(&values),
                "Catalog Can must remain narrowed: {values}"
            );
            assert!(
                !source.is_valid(&values),
                "source Can must remain narrowed: {values}"
            );
        }
        for values in [
            json!({"relation": false, "targets": []}),
            json!({"relation": "owners", "targets": [{"metadataPath": false}]}),
            json!({"relation": "owners", "targets": [{"metadataPath": "Catalog.Parent", "extra": true}]}),
            json!({"relation": "source", "targets": [{"kind": "manager"}]}),
            json!({"relation": "source", "targets": [{"kind": "family", "sourceClass": false}]}),
        ] {
            assert!(
                !raw.is_valid(&values),
                "raw union lost a structural field: {values}"
            );
        }
    }

    #[test]
    fn host_relation_schema_keeps_six_strict_event_source_variants() {
        let operation = metadata_relation_operation_schema();
        assert_eq!(
            operation["required"],
            json!(["op", "relation", "mode", "targets"])
        );
        let source = operation["oneOf"]
            .as_array()
            .unwrap()
            .iter()
            .find(|branch| branch["properties"]["relation"]["const"] == "source")
            .unwrap();
        let variants = source["properties"]["targets"]["items"]["oneOf"]
            .as_array()
            .unwrap();
        assert_eq!(variants.len(), 6);
        assert_eq!(
            metadata_event_source_input_schema()["oneOf"]
                .as_array()
                .unwrap()
                .len(),
            5
        );
        let managers = variants
            .iter()
            .filter(|branch| branch["properties"]["kind"]["const"] == "manager")
            .collect::<Vec<_>>();
        assert_eq!(managers.len(), 2);
        assert!(managers.iter().any(|schema| schema["required"]
            == json!(["kind", "metadataPath"])
            && schema["properties"].get("sourceClass").is_none()));
        assert!(managers.iter().any(|schema| schema["required"]
            == json!(["kind", "metadataPath", "sourceClass"])
            && schema["properties"]["sourceClass"]["enum"]
                == json!(["constantManager", "constantValueManager"])));
    }
}
