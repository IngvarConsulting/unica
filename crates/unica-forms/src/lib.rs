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

pub fn form_edit_definition_schema() -> Value {
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

pub fn validate_form_edit_definition(definition: &Value) -> Result<(), String> {
    let object = definition.as_object().ok_or_else(|| {
        "FORM_EDIT_DEFINITION_NOT_OBJECT: form edit definition must be an object".to_string()
    })?;

    // `textDoc` — внутренний ключ построения поля текстового документа;
    // JSON-описание формы такого ключа элемента не знает.
    fn contains_text_document_field(value: &Value) -> bool {
        match value {
            Value::Object(object) => {
                object.contains_key("textDoc") || object.values().any(contains_text_document_field)
            }
            Value::Array(items) => items.iter().any(contains_text_document_field),
            _ => false,
        }
    }
    if contains_text_document_field(definition) {
        return Err("FORM_EDIT_UNKNOWN_ELEMENT_KEY: unknown element key `textDoc`".to_string());
    }

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

    #[test]
    fn form_edit_definition_refuses_text_doc_as_an_unknown_element_key_at_any_depth() {
        for definition in [
            json!({"elements": [{"textDoc": "Protocol", "path": "ProtocolValue"}]}),
            json!({"elements": [{"group": "Main", "children": [{"textDoc": "Protocol"}]}]}),
        ] {
            let error = validate_form_edit_definition(&definition).unwrap_err();
            assert!(
                error.contains("FORM_EDIT_UNKNOWN_ELEMENT_KEY")
                    && error.contains("unknown element key `textDoc`")
                    && !error.contains("item.add"),
                "{error}"
            );
        }
    }
}

/// Роль элемента формы для словаря операций. Единственный владелец
/// отображения: `view` узнаёт роль по `props.tag` элемента, планировщик —
/// по тому же признаку элемента прочитанной формы.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormItemRole {
    /// Принимает вложенные элементы: группа, страницы, страница, таблица,
    /// группа колонок, командная панель, группа кнопок, подменю.
    Container,
    /// Автоматическая командная панель формы или таблицы: в неё добавляются
    /// кнопки, но удалить её отдельно от владельца нельзя.
    AutoCommandBar,
    /// Поле, декорация, кнопка: вложенных элементов не принимает.
    Leaf,
}

/// Observed form facts shared by operation dictionaries and staged preflight.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormObservation {
    Item(FormItemRole),
    ValueTableAttribute,
    ValueTableColumn,
}
impl FormObservation {
    pub fn from_attribute_type(type_name: &str) -> Option<Self> {
        (type_name == "ValueTable").then_some(Self::ValueTableAttribute)
    }
    pub fn from_column_parent_type(type_name: &str) -> Option<Self> {
        (type_name == "ValueTable").then_some(Self::ValueTableColumn)
    }
    pub fn item_role(self) -> Option<FormItemRole> {
        match self {
            Self::Item(role) => Some(role),
            Self::ValueTableAttribute | Self::ValueTableColumn => None,
        }
    }
}

impl FormItemRole {
    /// Роль по признаку `tag` элемента, как его публикует `view`
    /// (`[Group:V]`, `[Table]`, `[AutoCommandBar]`, `[Input]`).
    pub fn from_tag(tag: &str) -> Self {
        let kind = tag
            .trim_start_matches('[')
            .split([':', ',', ']'])
            .next()
            .unwrap_or_default();
        match kind {
            "AutoCommandBar" => Self::AutoCommandBar,
            "Group" | "Table" | "ColumnGroup" | "CmdBar" | "Pages" | "Page" | "Popup"
            | "BtnGroup" => Self::Container,
            _ => Self::Leaf,
        }
    }
}

pub mod applicability;
pub mod contracts;
pub mod operations;

pub mod root_properties;

pub mod events;

pub mod facts;

pub mod event_targets;

pub mod default_forms;
