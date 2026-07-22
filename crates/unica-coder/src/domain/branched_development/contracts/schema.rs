use crate::domain::i_json;
use schemars::Schema;
use serde_json::{Map, Number, Value};
use std::collections::HashSet;
use std::fmt;

// Draft 2020-12 treats `format` as an annotation unless a validator opts in.
// Contract validators must register these exact predicates and assert formats;
// the runtime scalar deserializers call the same predicates below.
pub(crate) const I_JSON_SINGLE_LINE_TEXT_FORMAT: &str = "unica-i-json-single-line-text";
pub(crate) const I_JSON_LF_TEXT_FORMAT: &str = "unica-i-json-lf-text";
pub(crate) const NORMALIZED_UTC_INSTANT_FORMAT: &str = "unica-normalized-utc-instant";

pub(crate) fn is_i_json_single_line_text(value: &str) -> bool {
    is_i_json_text(value, false)
}

pub(crate) fn is_i_json_lf_text(value: &str) -> bool {
    is_i_json_text(value, true)
}

fn is_i_json_text(value: &str, allow_line_feed: bool) -> bool {
    i_json::validate_i_json_string(value).is_ok()
        && value
            .chars()
            .all(|character| !character.is_control() || allow_line_feed && character == '\n')
}

pub(crate) fn is_normalized_utc_instant(value: &str) -> bool {
    let bytes = value.as_bytes();
    let base_is_well_formed = bytes.len() >= 20
        && bytes.len() <= 30
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes[10] == b'T'
        && bytes[13] == b':'
        && bytes[16] == b':'
        && bytes[19] != b'+'
        && bytes[19] != b'-';
    if !base_is_well_formed || bytes[19] != b'Z' && bytes[19] != b'.' {
        return false;
    }
    if !bytes[..19]
        .iter()
        .enumerate()
        .all(|(index, byte)| matches!(index, 4 | 7 | 10 | 13 | 16) || byte.is_ascii_digit())
    {
        return false;
    }
    match bytes[19] {
        b'Z' if bytes.len() == 20 => {}
        b'.' if bytes.last() == Some(&b'Z') => {
            let fraction = &bytes[20..bytes.len() - 1];
            if fraction.is_empty()
                || fraction.len() > 9
                || !fraction.iter().all(u8::is_ascii_digit)
                || fraction.last() == Some(&b'0')
            {
                return false;
            }
        }
        _ => return false,
    }
    let format = time::format_description::parse_borrowed::<2>(
        "[year]-[month]-[day]T[hour]:[minute]:[second]",
    )
    .expect("the fixed RFC 3339 UTC format must parse");
    time::PrimitiveDateTime::parse(&value[..19], &format).is_ok()
}

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

pub(crate) fn one_of_schema(variants: Vec<Schema>) -> Schema {
    assert!(
        !variants.is_empty(),
        "a closed contract union must contain at least one branch"
    );
    schemars::json_schema!({ "oneOf": variants })
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
    SchemaAuditor {
        root: schema,
        active_references: HashSet::new(),
    }
    .audit(schema, "$")
}

struct SchemaAuditor<'a> {
    root: &'a Value,
    active_references: HashSet<String>,
}

impl SchemaAuditor<'_> {
    fn audit(&mut self, schema: &Value, path: &str) -> Result<(), SchemaAuditError> {
        let object = schema
            .as_object()
            .ok_or_else(|| error(path, "schema nodes must be objects"))?;
        for keyword in object.keys() {
            if !is_supported_keyword(keyword) {
                return Err(error(path, "schema contains an unsupported keyword"));
            }
        }

        let has_positive_constraint = object.keys().any(|keyword| {
            matches!(
                keyword.as_str(),
                "$ref" | "type" | "enum" | "const" | "allOf" | "anyOf" | "oneOf"
            )
        });
        if !has_positive_constraint {
            return Err(error(
                path,
                "schema nodes must contain a positive value constraint",
            ));
        }

        self.audit_annotations(object, path)?;
        let types = audit_types(object.get("type"), path)?;
        let object_typed = types.contains("object");
        let array_typed = types.contains("array");
        let string_typed = types.contains("string");
        let numeric_typed = types.contains("number") || types.contains("integer");

        if object.keys().any(|keyword| {
            matches!(
                keyword.as_str(),
                "properties"
                    | "required"
                    | "additionalProperties"
                    | "minProperties"
                    | "maxProperties"
            )
        }) && !object_typed
        {
            return Err(error(
                path,
                "object keywords require an explicit object type",
            ));
        }
        if object_typed {
            self.audit_object(object, path)?;
        }

        if object.keys().any(|keyword| {
            matches!(
                keyword.as_str(),
                "items" | "prefixItems" | "minItems" | "maxItems" | "uniqueItems"
            )
        }) && !array_typed
        {
            return Err(error(path, "array keywords require an explicit array type"));
        }
        if array_typed {
            match object.get("prefixItems") {
                Some(prefix_items) => {
                    let prefix_items = prefix_items.as_array().ok_or_else(|| {
                        error(path, "prefixItems must be a non-empty schema array")
                    })?;
                    if prefix_items.is_empty() {
                        return Err(error(path, "prefixItems must not be empty"));
                    }
                    for (index, item) in prefix_items.iter().enumerate() {
                        self.audit(item, &format!("{path}.prefixItems[{index}]"))?;
                    }
                    match object.get("items") {
                        Some(Value::Bool(false)) => {
                            let exact_length = Value::Number(Number::from(prefix_items.len()));
                            if object.get("minItems") != Some(&exact_length)
                                || object.get("maxItems") != Some(&exact_length)
                            {
                                return Err(error(
                                    path,
                                    "closed positional arrays must fix minItems and maxItems to prefix length",
                                ));
                            }
                        }
                        Some(items @ Value::Object(_)) => {
                            self.audit(items, &format!("{path}.items"))?;
                            let Some(min_items) = object.get("minItems").and_then(Value::as_u64)
                            else {
                                return Err(error(
                                    path,
                                    "typed positional tails must require their complete prefix",
                                ));
                            };
                            if min_items < prefix_items.len() as u64 {
                                return Err(error(
                                    path,
                                    "typed positional tails must require their complete prefix",
                                ));
                            }
                            if object
                                .get("maxItems")
                                .and_then(Value::as_u64)
                                .is_some_and(|max_items| max_items < min_items)
                            {
                                return Err(error(
                                    path,
                                    "array maxItems must not be smaller than minItems",
                                ));
                            }
                        }
                        _ => {
                            return Err(error(
                                path,
                                "positional arrays must close or strictly type their tail",
                            ));
                        }
                    }
                }
                None => {
                    let items = object
                        .get("items")
                        .ok_or_else(|| error(path, "array schemas must define typed items"))?;
                    if !items.is_object() {
                        return Err(error(path, "non-positional array items must be a schema"));
                    }
                    self.audit(items, &format!("{path}.items"))?;
                }
            }
            audit_non_negative_integer(object.get("minItems"), path, "minItems")?;
            audit_non_negative_integer(object.get("maxItems"), path, "maxItems")?;
            audit_boolean(object.get("uniqueItems"), path, "uniqueItems")?;
        }

        if object.keys().any(|keyword| {
            matches!(
                keyword.as_str(),
                "minLength" | "maxLength" | "pattern" | "format"
            )
        }) && !string_typed
        {
            return Err(error(
                path,
                "string keywords require an explicit string type",
            ));
        }
        if string_typed {
            audit_non_negative_integer(object.get("minLength"), path, "minLength")?;
            audit_non_negative_integer(object.get("maxLength"), path, "maxLength")?;
            audit_string(object.get("pattern"), path, "pattern")?;
            audit_format(object.get("format"), path)?;
        }

        if object.keys().any(|keyword| {
            matches!(
                keyword.as_str(),
                "minimum" | "maximum" | "exclusiveMinimum" | "exclusiveMaximum" | "multipleOf"
            )
        }) && !numeric_typed
        {
            return Err(error(
                path,
                "numeric keywords require an explicit numeric type",
            ));
        }
        if numeric_typed {
            for keyword in [
                "minimum",
                "maximum",
                "exclusiveMinimum",
                "exclusiveMaximum",
                "multipleOf",
            ] {
                audit_number(object.get(keyword), path, keyword)?;
            }
        }

        audit_scalar_enum_or_const(object.get("enum"), path, "enum")?;
        audit_scalar_enum_or_const(object.get("const"), path, "const")?;

        for keyword in ["$defs", "definitions"] {
            if let Some(definitions) = object.get(keyword) {
                let definitions = definitions.as_object().ok_or_else(|| {
                    error(
                        &format!("{path}.{keyword}"),
                        "schema definitions must be an object",
                    )
                })?;
                for (name, definition) in definitions {
                    self.audit(definition, &format!("{path}.{keyword}.{name}"))?;
                }
            }
        }
        for keyword in ["allOf", "anyOf", "oneOf"] {
            if let Some(schemas) = object.get(keyword) {
                let schemas = schemas.as_array().ok_or_else(|| {
                    error(
                        &format!("{path}.{keyword}"),
                        "schema composition entries must be arrays",
                    )
                })?;
                if schemas.is_empty() {
                    return Err(error(
                        &format!("{path}.{keyword}"),
                        "schema composition arrays must not be empty",
                    ));
                }
                for (index, nested) in schemas.iter().enumerate() {
                    self.audit(nested, &format!("{path}.{keyword}[{index}]"))?;
                }
            }
        }
        if let Some(not) = object.get("not") {
            self.audit(not, &format!("{path}.not"))?;
        }
        if let Some(reference) = object.get("$ref") {
            self.audit_reference(reference, path)?;
        }
        Ok(())
    }

    fn audit_object(
        &mut self,
        object: &Map<String, Value>,
        path: &str,
    ) -> Result<(), SchemaAuditError> {
        if object.get("additionalProperties") != Some(&Value::Bool(false)) {
            return Err(error(
                path,
                "object schemas must set additionalProperties to false",
            ));
        }
        let empty_properties = Map::new();
        let properties = match object.get("properties") {
            Some(properties) => properties
                .as_object()
                .ok_or_else(|| error(path, "properties must be an object"))?,
            None => &empty_properties,
        };
        for (name, property) in properties {
            self.audit(property, &format!("{path}.properties.{name}"))?;
        }
        if let Some(required) = object.get("required") {
            let required = required
                .as_array()
                .ok_or_else(|| error(path, "required must be an array"))?;
            let mut names = HashSet::new();
            for name in required {
                let name = name
                    .as_str()
                    .ok_or_else(|| error(path, "required entries must be strings"))?;
                if !names.insert(name) || !properties.contains_key(name) {
                    return Err(error(
                        path,
                        "required entries must uniquely name declared properties",
                    ));
                }
            }
        }
        audit_non_negative_integer(object.get("minProperties"), path, "minProperties")?;
        audit_non_negative_integer(object.get("maxProperties"), path, "maxProperties")?;
        Ok(())
    }

    fn audit_annotations(
        &self,
        object: &Map<String, Value>,
        path: &str,
    ) -> Result<(), SchemaAuditError> {
        if let Some(dialect) = object.get("$schema") {
            if path != "$"
                || dialect.as_str() != Some("https://json-schema.org/draft/2020-12/schema")
            {
                return Err(error(
                    path,
                    "schema dialect must be Draft 2020-12 at the root",
                ));
            }
        }
        for keyword in ["title", "description"] {
            audit_string(object.get(keyword), path, keyword)?;
        }
        for keyword in ["deprecated", "readOnly", "writeOnly"] {
            audit_boolean(object.get(keyword), path, keyword)?;
        }
        if let Some(examples) = object.get("examples") {
            if !examples.is_array() {
                return Err(error(path, "examples must be an array"));
            }
        }
        Ok(())
    }

    fn audit_reference(&mut self, reference: &Value, path: &str) -> Result<(), SchemaAuditError> {
        let reference = reference
            .as_str()
            .ok_or_else(|| error(path, "$ref must be a string"))?;
        let pointer = reference
            .strip_prefix('#')
            .filter(|pointer| pointer.starts_with('/'))
            .ok_or_else(|| error(path, "$ref must be a non-root local JSON pointer"))?;
        let target = self
            .root
            .pointer(pointer)
            .ok_or_else(|| error(path, "$ref must resolve to a local schema"))?;
        if !self.active_references.insert(reference.to_owned()) {
            return Err(error(path, "recursive $ref graphs are not supported"));
        }
        let result = self.audit(target, &format!("{path}.$ref({reference})"));
        self.active_references.remove(reference);
        result
    }
}

fn error(path: &str, reason: &'static str) -> SchemaAuditError {
    SchemaAuditError {
        path: path.to_owned(),
        reason,
    }
}

fn is_supported_keyword(keyword: &str) -> bool {
    matches!(
        keyword,
        "$schema"
            | "$ref"
            | "$defs"
            | "definitions"
            | "title"
            | "description"
            | "default"
            | "examples"
            | "deprecated"
            | "readOnly"
            | "writeOnly"
            | "type"
            | "enum"
            | "const"
            | "allOf"
            | "anyOf"
            | "oneOf"
            | "not"
            | "properties"
            | "required"
            | "additionalProperties"
            | "minProperties"
            | "maxProperties"
            | "items"
            | "prefixItems"
            | "minItems"
            | "maxItems"
            | "uniqueItems"
            | "minLength"
            | "maxLength"
            | "pattern"
            | "format"
            | "minimum"
            | "maximum"
            | "exclusiveMinimum"
            | "exclusiveMaximum"
            | "multipleOf"
    )
}

fn audit_types<'a>(
    value: Option<&'a Value>,
    path: &str,
) -> Result<HashSet<&'a str>, SchemaAuditError> {
    let Some(value) = value else {
        return Ok(HashSet::new());
    };
    let mut types = HashSet::new();
    match value {
        Value::String(kind) => {
            insert_type(&mut types, kind, path)?;
        }
        Value::Array(kinds) if !kinds.is_empty() => {
            for kind in kinds {
                let kind = kind
                    .as_str()
                    .ok_or_else(|| error(path, "type array entries must be strings"))?;
                insert_type(&mut types, kind, path)?;
            }
        }
        _ => {
            return Err(error(
                path,
                "type must be a string or non-empty string array",
            ))
        }
    }
    Ok(types)
}

fn insert_type<'a>(
    types: &mut HashSet<&'a str>,
    kind: &'a str,
    path: &str,
) -> Result<(), SchemaAuditError> {
    if !matches!(
        kind,
        "null" | "boolean" | "object" | "array" | "number" | "integer" | "string"
    ) || !types.insert(kind)
    {
        return Err(error(path, "type names must be supported and unique"));
    }
    Ok(())
}

fn audit_scalar_enum_or_const(
    value: Option<&Value>,
    path: &str,
    keyword: &'static str,
) -> Result<(), SchemaAuditError> {
    let Some(value) = value else {
        return Ok(());
    };
    let values: &[Value] = if keyword == "enum" {
        let values = value
            .as_array()
            .ok_or_else(|| error(path, "enum must be an array"))?;
        if values.is_empty() {
            return Err(error(path, "enum must not be empty"));
        }
        values
    } else {
        std::slice::from_ref(value)
    };
    if values
        .iter()
        .any(|value| value.is_array() || value.is_object())
    {
        return Err(error(
            path,
            "enum and const values must not hide structured instances",
        ));
    }
    Ok(())
}

fn audit_format(value: Option<&Value>, path: &str) -> Result<(), SchemaAuditError> {
    let Some(value) = value else {
        return Ok(());
    };
    match value.as_str() {
        Some(
            "uuid"
            | I_JSON_SINGLE_LINE_TEXT_FORMAT
            | I_JSON_LF_TEXT_FORMAT
            | NORMALIZED_UTC_INSTANT_FORMAT,
        ) => Ok(()),
        _ => Err(error(path, "string format is unsupported")),
    }
}

fn audit_string(
    value: Option<&Value>,
    path: &str,
    _keyword: &'static str,
) -> Result<(), SchemaAuditError> {
    match value {
        Some(Value::String(_)) | None => Ok(()),
        Some(_) => Err(error(path, "keyword must contain a string")),
    }
}

fn audit_boolean(
    value: Option<&Value>,
    path: &str,
    _keyword: &'static str,
) -> Result<(), SchemaAuditError> {
    match value {
        Some(Value::Bool(_)) | None => Ok(()),
        Some(_) => Err(error(path, "keyword must contain a boolean")),
    }
}

fn audit_non_negative_integer(
    value: Option<&Value>,
    path: &str,
    _keyword: &'static str,
) -> Result<(), SchemaAuditError> {
    match value {
        Some(Value::Number(number)) if number.as_u64().is_some() => Ok(()),
        None => Ok(()),
        Some(_) => Err(error(path, "keyword must contain a non-negative integer")),
    }
}

fn audit_number(
    value: Option<&Value>,
    path: &str,
    _keyword: &'static str,
) -> Result<(), SchemaAuditError> {
    match value {
        Some(Value::Number(_)) | None => Ok(()),
        Some(_) => Err(error(path, "keyword must contain a number")),
    }
}
