use serde_json::{json, Map, Value};
use std::collections::BTreeSet;

#[derive(Clone, Copy)]
enum FormEditSectionKind {
    Array,
    String,
    RemoveElements,
}

const FORM_EDIT_SECTIONS: &[(&str, FormEditSectionKind)] = &[
    ("into", FormEditSectionKind::String),
    ("after", FormEditSectionKind::String),
    ("elements", FormEditSectionKind::Array),
    ("attributes", FormEditSectionKind::Array),
    ("commands", FormEditSectionKind::Array),
    ("formEvents", FormEditSectionKind::Array),
    ("elementEvents", FormEditSectionKind::Array),
    ("removeElements", FormEditSectionKind::RemoveElements),
];

pub(crate) fn form_edit_definition_schema() -> Value {
    let mut properties = Map::new();
    for (name, kind) in FORM_EDIT_SECTIONS {
        let schema = match kind {
            FormEditSectionKind::Array => json!({"type": "array"}),
            FormEditSectionKind::String => json!({"type": "string"}),
            FormEditSectionKind::RemoveElements => json!({
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {"name": {"type": "string", "minLength": 1, "pattern": r"\S"}},
                    "required": ["name"]
                }
            }),
        };
        properties.insert((*name).to_string(), schema);
    }
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": properties,
    })
}

pub(crate) fn validate_form_edit_definition(definition: &Value) -> Result<(), String> {
    let object = definition.as_object().ok_or_else(|| {
        "FORM_EDIT_DEFINITION_NOT_OBJECT: form edit definition must be an object".to_string()
    })?;

    for (name, value) in object {
        let Some((_, kind)) = FORM_EDIT_SECTIONS
            .iter()
            .find(|(section, _)| *section == name)
        else {
            return Err(format!(
                "FORM_EDIT_UNKNOWN_SECTION: unsupported form edit section `{name}`"
            ));
        };
        match kind {
            FormEditSectionKind::Array if !value.is_array() => {
                return Err(format!(
                    "FORM_EDIT_SECTION_TYPE: form edit section `{name}` must be an array"
                ));
            }
            FormEditSectionKind::String if !value.is_string() => {
                return Err(format!(
                    "FORM_EDIT_SECTION_TYPE: form edit section `{name}` must be a string"
                ));
            }
            FormEditSectionKind::RemoveElements => validate_remove_elements(value)?,
            _ => {}
        }
    }

    Ok(())
}

fn validate_remove_elements(value: &Value) -> Result<(), String> {
    let entries = value.as_array().ok_or_else(|| {
        "FORM_EDIT_SECTION_TYPE: form edit section `removeElements` must be an array".to_string()
    })?;
    let mut names = BTreeSet::new();
    for entry in entries {
        let object = entry.as_object().ok_or_else(|| {
            "FORM_EDIT_REMOVE_ELEMENT_INVALID: every removeElements entry must be an object"
                .to_string()
        })?;
        for field in object.keys() {
            if field != "name" {
                return Err(format!(
                    "FORM_EDIT_REMOVE_ELEMENT_UNKNOWN_FIELD: removeElements entry does not accept `{field}`"
                ));
            }
        }
        let name = object.get("name").and_then(Value::as_str).ok_or_else(|| {
            "FORM_EDIT_REMOVE_ELEMENT_MISSING_NAME: removeElements entry requires string `name`"
                .to_string()
        })?;
        if name.trim().is_empty() {
            return Err(
                "FORM_EDIT_REMOVE_ELEMENT_EMPTY_NAME: removeElements entry name must not be empty"
                    .to_string(),
            );
        }
        if !names.insert(name.to_string()) {
            return Err(format!(
                "FORM_EDIT_REMOVE_ELEMENT_DUPLICATE: duplicate removeElements name `{name}`"
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remove_elements_rejects_non_object_entries() {
        let error =
            validate_form_edit_definition(&json!({"removeElements": ["Name"]})).unwrap_err();
        assert!(
            error.contains("FORM_EDIT_REMOVE_ELEMENT_INVALID"),
            "{error}"
        );
    }

    #[test]
    fn remove_elements_treats_whitespace_distinct_names_as_distinct() {
        let result = validate_form_edit_definition(&json!({
            "removeElements": [
                {"name": "Target"},
                {"name": "Target "}
            ]
        }));

        assert!(result.is_ok(), "{result:?}");
    }

    #[test]
    fn remove_elements_rejects_exact_duplicate_names() {
        let error = validate_form_edit_definition(&json!({
            "removeElements": [
                {"name": "Target"},
                {"name": "Target"}
            ]
        }))
        .unwrap_err();

        assert!(
            error.contains("FORM_EDIT_REMOVE_ELEMENT_DUPLICATE") && error.contains("`Target`"),
            "{error}"
        );
    }
}
