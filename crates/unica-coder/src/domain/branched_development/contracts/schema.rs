use schemars::Schema;
use serde_json::{Map, Number, Value};
use std::fmt;

pub(crate) fn string_schema(
    min_length: usize,
    max_length: usize,
    pattern: Option<&str>,
    format: Option<&str>,
) -> Schema {
    let mut schema = Map::new();
    schema.insert("type".to_owned(), Value::String("string".to_owned()));
    schema.insert(
        "minLength".to_owned(),
        Value::Number(Number::from(min_length)),
    );
    schema.insert(
        "maxLength".to_owned(),
        Value::Number(Number::from(max_length)),
    );
    if let Some(pattern) = pattern {
        schema.insert("pattern".to_owned(), Value::String(pattern.to_owned()));
    }
    if let Some(format) = format {
        schema.insert("format".to_owned(), Value::String(format.to_owned()));
    }
    Schema::from(schema)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SchemaAuditError {
    path: String,
    reason: &'static str,
}

impl fmt::Display for SchemaAuditError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "invalid schema at {}: {}",
            self.path, self.reason
        )
    }
}

impl std::error::Error for SchemaAuditError {}

pub(crate) fn audit_json_schema(schema: &Value) -> Result<(), SchemaAuditError> {
    audit_schema(schema, "$")
}

fn audit_schema(schema: &Value, path: &str) -> Result<(), SchemaAuditError> {
    let object = schema.as_object().ok_or_else(|| SchemaAuditError {
        path: path.to_owned(),
        reason: "schema nodes must be objects",
    })?;
    if !object.keys().any(|keyword| {
        matches!(
            keyword.as_str(),
            "$ref" | "type" | "enum" | "const" | "allOf" | "anyOf" | "oneOf" | "not"
        )
    }) {
        return Err(SchemaAuditError {
            path: path.to_owned(),
            reason: "schema nodes must constrain their value type",
        });
    }
    if object.get("type").and_then(Value::as_str) == Some("object")
        || object.contains_key("properties")
    {
        if object.get("additionalProperties") != Some(&Value::Bool(false)) {
            return Err(SchemaAuditError {
                path: path.to_owned(),
                reason: "object schemas must set additionalProperties to false",
            });
        }
        if let Some(properties) = object.get("properties") {
            let properties = properties.as_object().ok_or_else(|| SchemaAuditError {
                path: path.to_owned(),
                reason: "properties must be an object",
            })?;
            for (name, property) in properties {
                audit_schema(property, &format!("{path}.properties.{name}"))?;
            }
        }
    }
    if object.get("type").and_then(Value::as_str) == Some("array") {
        let items = object.get("items").ok_or_else(|| SchemaAuditError {
            path: path.to_owned(),
            reason: "array schemas must define typed items",
        })?;
        audit_schema(items, &format!("{path}.items"))?;
    }
    for keyword in ["$defs", "definitions"] {
        if let Some(definitions) = object.get(keyword) {
            let definitions = definitions.as_object().ok_or_else(|| SchemaAuditError {
                path: format!("{path}.{keyword}"),
                reason: "schema definitions must be an object",
            })?;
            for (name, definition) in definitions {
                audit_schema(definition, &format!("{path}.{keyword}.{name}"))?;
            }
        }
    }
    for keyword in ["allOf", "anyOf", "oneOf", "prefixItems"] {
        if let Some(schemas) = object.get(keyword) {
            let schemas = schemas.as_array().ok_or_else(|| SchemaAuditError {
                path: format!("{path}.{keyword}"),
                reason: "schema composition entries must be arrays",
            })?;
            for (index, nested) in schemas.iter().enumerate() {
                audit_schema(nested, &format!("{path}.{keyword}[{index}]"))?;
            }
        }
    }
    if let Some(not) = object.get("not") {
        audit_schema(not, &format!("{path}.not"))?;
    }
    Ok(())
}
