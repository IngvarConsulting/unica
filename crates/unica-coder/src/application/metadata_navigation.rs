use std::{collections::BTreeSet, io::Write};

use serde_json::Value;
use unica_format_core::{
    limits::{
        MAX_NAVIGATION_NESTING_DEPTH, MAX_NAVIGATION_PROPERTY_SELECTORS,
        MAX_NAVIGATION_RELATION_SELECTORS, MAX_NAVIGATION_SELECTOR_STRING_BYTES,
        MAX_NAVIGATION_SELECT_JSON_BYTES,
    },
    navigation::{
        normalize_navigation_selection, FacetSelection, NavigationSelection, PropertySelection,
        RelationKind, RelationSelection,
    },
    semantic_ids::{SemanticPropertyId, SemanticRelationId},
    source::{SourceAdapterError, SourceAdapterErrorKind},
};

pub(crate) fn parse_navigation_selection(
    value: Option<&Value>,
) -> Result<NavigationSelection, SourceAdapterError> {
    let Some(value) = value else {
        return Ok(NavigationSelection {
            properties: PropertySelection::All,
            facets: FacetSelection::Summary,
            relations: vec![RelationSelection::new(SemanticRelationId::CHILDREN, None)?],
        });
    };
    preflight_navigation_selection(value)?;
    let object = value.as_object().ok_or_else(|| {
        SourceAdapterError::new(
            SourceAdapterErrorKind::DecodeCorrupted,
            "select must be an object",
        )
    })?;
    if object
        .keys()
        .any(|key| !matches!(key.as_str(), "properties" | "facets" | "relations"))
    {
        return Err(SourceAdapterError::new(
            SourceAdapterErrorKind::DecodeCorrupted,
            "select has unknown fields",
        ));
    }
    let properties = match object.get("properties") {
        None => PropertySelection::All,
        Some(Value::String(value)) if value == "all" => PropertySelection::All,
        Some(Value::Array(values)) => parse_named_properties(values, false)?,
        Some(Value::Object(value)) => {
            if value.len() != 1 || !value.contains_key("named") {
                return Err(SourceAdapterError::new(
                    SourceAdapterErrorKind::DecodeCorrupted,
                    "select.properties has unknown fields",
                ));
            }
            let names = value
                .get("named")
                .and_then(Value::as_array)
                .ok_or_else(|| {
                    SourceAdapterError::new(
                        SourceAdapterErrorKind::DecodeCorrupted,
                        "select.properties.named must be an array",
                    )
                })?;
            parse_named_properties(names, true)?
        }
        _ => {
            return Err(SourceAdapterError::new(
                SourceAdapterErrorKind::DecodeCorrupted,
                "select.properties is invalid",
            ))
        }
    };
    let facets = match object.get("facets") {
        None => FacetSelection::Summary,
        Some(Value::String(value)) => match value.as_str() {
            "none" => FacetSelection::None,
            "summary" => FacetSelection::Summary,
            "full" => FacetSelection::Full,
            _ => return Err(decode_error("select.facets is invalid")),
        },
        Some(_) => return Err(decode_error("select.facets must be a string")),
    };
    let relations = match object.get("relations") {
        None => vec![RelationSelection::new(SemanticRelationId::CHILDREN, None)?],
        Some(Value::Array(relations)) => {
            let mut parsed = Vec::with_capacity(relations.len());
            for relation in relations {
                let relation = relation
                    .as_object()
                    .ok_or_else(|| decode_error("select.relations items must be objects"))?;
                if relation
                    .keys()
                    .any(|key| !matches!(key.as_str(), "role" | "kind" | "pageSize"))
                {
                    return Err(decode_error("select.relations item has unknown fields"));
                }
                let role = relation
                    .get("role")
                    .and_then(Value::as_str)
                    .ok_or_else(|| decode_error("select.relations item has no role"))?;
                let page_size =
                    match relation.get("pageSize") {
                        None => None,
                        Some(value) => Some(
                            u16::try_from(value.as_u64().ok_or_else(|| {
                                decode_error("select.relations pageSize is invalid")
                            })?)
                            .map_err(|_| decode_error("select.relations pageSize is invalid"))?,
                        ),
                    };
                let kind = match relation.get("kind") {
                    None => RelationKind::Contains,
                    Some(Value::String(value)) => match value.as_str() {
                        "contains" => RelationKind::Contains,
                        "references" => RelationKind::References,
                        _ => return Err(decode_error("select.relations kind is invalid")),
                    },
                    Some(_) => return Err(decode_error("select.relations kind is invalid")),
                };
                let role = SemanticRelationId::parse(role)
                    .ok_or_else(|| decode_error("select.relations role is invalid"))?;
                let mut selection = RelationSelection::new(role, page_size)
                    .map_err(|_| decode_error("select.relations role is invalid"))?;
                selection.kind = kind;
                parsed.push(selection);
            }
            parsed
        }
        _ => return Err(decode_error("select.relations is invalid")),
    };
    normalize_navigation_selection(NavigationSelection {
        properties,
        facets,
        relations,
    })
    .map_err(|_| decode_error("select is invalid"))
}

fn parse_named_properties(
    values: &[Value],
    require_nonempty: bool,
) -> Result<PropertySelection, SourceAdapterError> {
    if require_nonempty && values.is_empty() {
        return Err(decode_error("select.properties.named must not be empty"));
    }
    let mut names = BTreeSet::new();
    for value in values {
        let name = value
            .as_str()
            .filter(|name| !name.is_empty())
            .ok_or_else(|| decode_error("select.properties values must be non-empty strings"))?;
        let id = SemanticPropertyId::parse(name)
            .ok_or_else(|| decode_error("select.properties value is not registered"))?;
        if !names.insert(id) {
            return Err(decode_error("select.properties values must be unique"));
        }
    }
    Ok(PropertySelection::Named(names))
}

pub(crate) fn preflight_navigation_selection(value: &Value) -> Result<(), SourceAdapterError> {
    preflight_navigation_json(
        value,
        MAX_NAVIGATION_SELECT_JSON_BYTES,
        MAX_NAVIGATION_SELECTOR_STRING_BYTES,
        "select",
    )?;
    let Some(object) = value.as_object() else {
        return Ok(());
    };
    let property_values = object.get("properties").and_then(|value| match value {
        Value::Array(values) => Some(values),
        Value::Object(values) => values.get("named").and_then(Value::as_array),
        _ => None,
    });
    if property_values.is_some_and(|values| values.len() > MAX_NAVIGATION_PROPERTY_SELECTORS) {
        return Err(resource_limit("select has too many property selectors"));
    }
    if let Some(values) = property_values {
        for value in values {
            if value
                .as_str()
                .is_some_and(|value| value.len() > MAX_NAVIGATION_SELECTOR_STRING_BYTES)
            {
                return Err(resource_limit(
                    "select property selector exceeds its byte limit",
                ));
            }
        }
    }
    if object
        .get("relations")
        .and_then(Value::as_array)
        .is_some_and(|values| values.len() > MAX_NAVIGATION_RELATION_SELECTORS)
    {
        return Err(resource_limit("select has too many relation selectors"));
    }
    Ok(())
}

fn preflight_navigation_json(
    value: &Value,
    max_bytes: usize,
    max_string_bytes: usize,
    label: &str,
) -> Result<(), SourceAdapterError> {
    validate_navigation_json_structure(value, 1, max_string_bytes, label)?;
    let mut writer = BoundedCountingWriter::new(max_bytes);
    let serialized = serde_json::to_writer(&mut writer, value);
    if writer.exceeded {
        return Err(resource_limit(&format!(
            "{label} exceeds its JSON byte limit"
        )));
    }
    serialized.map_err(|_| decode_error(&format!("{label} cannot be serialized as JSON")))?;
    Ok(())
}

fn validate_navigation_json_structure(
    value: &Value,
    depth: usize,
    max_string_bytes: usize,
    label: &str,
) -> Result<(), SourceAdapterError> {
    if depth > MAX_NAVIGATION_NESTING_DEPTH {
        return Err(resource_limit(&format!(
            "{label} exceeds navigation nesting limit"
        )));
    }
    match value {
        Value::String(value) if value.len() > max_string_bytes => Err(resource_limit(&format!(
            "{label} string exceeds its byte limit"
        ))),
        Value::Array(values) => {
            let child_depth = depth.checked_add(1).ok_or_else(|| {
                resource_limit(&format!("{label} nesting depth cannot be represented"))
            })?;
            for value in values {
                validate_navigation_json_structure(value, child_depth, max_string_bytes, label)?;
            }
            Ok(())
        }
        Value::Object(values) => {
            let child_depth = depth.checked_add(1).ok_or_else(|| {
                resource_limit(&format!("{label} nesting depth cannot be represented"))
            })?;
            for (key, value) in values {
                if key.len() > max_string_bytes {
                    return Err(resource_limit(&format!(
                        "{label} key exceeds its byte limit"
                    )));
                }
                validate_navigation_json_structure(value, child_depth, max_string_bytes, label)?;
            }
            Ok(())
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => Ok(()),
    }
}

struct BoundedCountingWriter {
    limit: usize,
    bytes: usize,
    exceeded: bool,
}

impl BoundedCountingWriter {
    fn new(limit: usize) -> Self {
        Self {
            limit,
            bytes: 0,
            exceeded: false,
        }
    }
}

impl Write for BoundedCountingWriter {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        let Some(next) = self.bytes.checked_add(buffer.len()) else {
            self.exceeded = true;
            return Err(std::io::Error::other("serialized navigation size overflow"));
        };
        if next > self.limit {
            self.exceeded = true;
            return Err(std::io::Error::new(
                std::io::ErrorKind::WriteZero,
                "serialized navigation exceeds input limit",
            ));
        }
        self.bytes = next;
        Ok(buffer.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn decode_error(message: &str) -> SourceAdapterError {
    SourceAdapterError::new(SourceAdapterErrorKind::DecodeCorrupted, message)
}

fn resource_limit(message: &str) -> SourceAdapterError {
    SourceAdapterError::new(SourceAdapterErrorKind::ResourceLimit, message)
}

pub(crate) fn metadata_navigation_command(
    args: &serde_json::Map<String, Value>,
) -> Result<unica_application::MetadataNavigationCommand, SourceAdapterError> {
    use unica_application::{MetadataNavigationCommand, MetadataNavigationTarget};
    use unica_format_core::{
        navigation::{ObjectKey, OpaqueNavigationCursor},
        source::{SourceId, SourceRevision},
    };

    let selection = args
        .get("select")
        .map(|selection| parse_navigation_selection(Some(selection)))
        .transpose()?;
    let object_path = args
        .get("ObjectPath")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty());
    let object_ref = args.get("objectRef");
    let snapshot_revision = args
        .get("snapshotRevision")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty());
    let cursor = args.get("cursor");
    let target = match (object_path, object_ref, snapshot_revision, cursor) {
        (Some(_), None, None, None) => MetadataNavigationTarget::Source,
        (None, Some(object_ref), Some(revision), None) => {
            let object = object_ref
                .as_object()
                .filter(|object| {
                    object.len() == 2
                        && object.contains_key("sourceId")
                        && object.contains_key("objectKey")
                })
                .ok_or_else(|| decode_error("objectRef has unknown or missing fields"))?;
            MetadataNavigationTarget::ObjectRef {
                source_id: SourceId::new(
                    object
                        .get("sourceId")
                        .and_then(Value::as_str)
                        .ok_or_else(|| decode_error("objectRef has no valid sourceId"))?,
                )?,
                object_key: ObjectKey::new(
                    object
                        .get("objectKey")
                        .and_then(Value::as_str)
                        .ok_or_else(|| decode_error("objectRef has no valid objectKey"))?,
                )?,
                snapshot_revision: SourceRevision::new(revision)?,
            }
        }
        (None, None, None, Some(cursor)) => {
            MetadataNavigationTarget::Cursor(OpaqueNavigationCursor::from_token(
                cursor
                    .as_str()
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| decode_error("cursor must be a non-empty opaque string"))?,
            ))
        }
        _ => return Err(decode_error("meta.info requires exactly one target mode")),
    };
    if matches!(target, MetadataNavigationTarget::Cursor(_)) && selection.is_some() {
        return Err(decode_error("cursor mode does not accept select"));
    }
    Ok(MetadataNavigationCommand { target, selection })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn runtime_selection_validation_is_schema_strict() {
        let default_and_explicit = parse_navigation_selection(Some(&json!({"relations": [
            {"role": "attributes"}, {"role": "attributes", "kind": "contains", "pageSize": 25}
        ]})))
        .unwrap();
        assert_eq!(default_and_explicit.relations.len(), 1);
        assert_eq!(
            default_and_explicit.relations[0].kind,
            RelationKind::Contains
        );
        assert_eq!(default_and_explicit.relations[0].page_size, 25);
        let first = parse_navigation_selection(Some(&json!({"relations": [
            {"role": "attributes", "pageSize": 20}, {"role": "attributes", "pageSize": 10}
        ]})))
        .unwrap();
        let second = parse_navigation_selection(Some(&json!({"relations": [
            {"role": "attributes", "pageSize": 10}, {"role": "attributes", "pageSize": 20}
        ]})))
        .unwrap();
        assert_eq!(first.relations[0].page_size, 10);
        assert_eq!(first, second);
        assert_eq!(
            unica_format_core::navigation::normalized_selection_hash(&first).unwrap(),
            unica_format_core::navigation::normalized_selection_hash(&second).unwrap(),
        );
        for value in [
            json!({"properties": ["", "metadata.name"]}),
            json!({"properties": ["metadata.name", "metadata.name"]}),
            json!({"properties": ["native.name"]}),
            json!({"facets": 1}),
            json!({"relations": [{"role": "unknown"}]}),
            json!({"relations": [{"role": "native.references"}]}),
            json!({"relations": [{"role": "attributes", "pageSize": "1"}]}),
            json!({"relations": [{"role": "attributes", "offset": 0}]}),
        ] {
            let error = parse_navigation_selection(Some(&value)).unwrap_err();
            assert_eq!(error.code(), "decode_corrupted", "{value}");
        }
    }

    #[test]
    fn runtime_cursor_conversion_accepts_only_a_non_empty_string() {
        let command =
            metadata_navigation_command(json!({"cursor": "AQID"}).as_object().unwrap()).unwrap();
        assert!(matches!(
            command.target,
            unica_application::MetadataNavigationTarget::Cursor(_)
        ));

        for invalid in [json!({"cursor": {}}), json!({"cursor": ""})] {
            let error = metadata_navigation_command(invalid.as_object().unwrap()).unwrap_err();
            assert_eq!(error.kind, SourceAdapterErrorKind::DecodeCorrupted);
        }
    }

    #[test]
    fn relation_page_size_rejects_values_above_the_contract_bound() {
        let error = parse_navigation_selection(Some(&json!({
            "relations": [{"role": "attributes", "pageSize": 101}]
        })))
        .unwrap_err();
        assert_eq!(error.code(), "decode_corrupted");
    }
}
