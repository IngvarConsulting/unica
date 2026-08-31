use crate::domain::address::{NodeKind, QualifiedAddress};
use crate::infrastructure::metadata_kinds::metadata_kind;
use serde_json::{Map, Value};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ReadModeError {
    code: &'static str,
    message: String,
}

impl ReadModeError {
    fn bad_value(message: impl Into<String>) -> Self {
        Self {
            code: "bad_value",
            message: message.into(),
        }
    }

    fn unsupported_filter(message: impl Into<String>) -> Self {
        Self {
            code: "unsupported_filter",
            message: message.into(),
        }
    }

    fn unsupported_scope(message: impl Into<String>) -> Self {
        Self {
            code: "unsupported_scope",
            message: message.into(),
        }
    }

    pub(super) const fn code(&self) -> &'static str {
        self.code
    }
}

impl fmt::Display for ReadModeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.message.fmt(formatter)
    }
}

impl std::error::Error for ReadModeError {}

const VIEW_IDENTITY_SLOTS: &[&str] = &["at", "kind", "title"];
const VIEW_SECTION_SLOTS: &[&str] = &["props", "branches", "can", "limits", "items"];

pub(super) fn project_view_sections(
    data: &Value,
    sections: &Value,
) -> Result<Value, ReadModeError> {
    let data = data
        .as_object()
        .ok_or_else(|| ReadModeError::bad_value("view section projection requires object data"))?;
    let sections = sections
        .as_array()
        .ok_or_else(|| ReadModeError::bad_value("view sections must be an array of strings"))?;
    if sections.is_empty() {
        return Err(ReadModeError::bad_value(
            "view sections must contain at least one section",
        ));
    }
    let mut selected = Vec::with_capacity(sections.len());
    for section in sections {
        let section = section
            .as_str()
            .ok_or_else(|| ReadModeError::bad_value("view sections must contain strings"))?;
        if !VIEW_SECTION_SLOTS.contains(&section) {
            return Err(ReadModeError::unsupported_filter(format!(
                "unsupported view section `{section}`"
            )));
        }
        if !selected.contains(&section) {
            selected.push(section);
        }
    }
    let mut projected = Map::new();
    for slot in VIEW_IDENTITY_SLOTS {
        if let Some(value) = data.get(*slot) {
            projected.insert((*slot).to_string(), value.clone());
        }
    }
    for slot in selected {
        if let Some(value) = data.get(slot) {
            projected.insert(slot.to_string(), value.clone());
        }
    }
    Ok(Value::Object(projected))
}

pub(super) fn search_scope_prefix(
    scope: &QualifiedAddress,
) -> Result<Option<String>, ReadModeError> {
    if scope.segments().len() != 1 {
        return Err(ReadModeError::unsupported_scope(
            "literal search currently accepts only Configuration or one metadata-object subtree",
        ));
    }
    let Some(owner) = scope.segments().first() else {
        return Err(ReadModeError::bad_value(
            "search scope has no logical segments",
        ));
    };
    if owner.kind() == NodeKind::Configuration {
        return Ok(None);
    }
    let layout = metadata_kind(owner.kind().as_str()).ok_or_else(|| {
        ReadModeError::unsupported_scope(format!(
            "search scope kind `{}` has no Platform XML subtree",
            owner.kind().as_str()
        ))
    })?;
    let mut prefix = layout.directory.to_string();
    if let Some(name) = owner.name() {
        prefix.push('/');
        prefix.push_str(name);
    }
    Ok(Some(prefix))
}

pub(super) fn validation_profile(filter: &Value) -> Result<Option<String>, ReadModeError> {
    let filter = filter
        .as_object()
        .ok_or_else(|| ReadModeError::bad_value("check filter must be an object"))?;
    if filter.is_empty() {
        return Ok(None);
    }
    if filter.keys().any(|key| key != "validation") {
        return Err(ReadModeError::unsupported_filter(
            "check filter supports only `validation`",
        ));
    }
    let validation = filter
        .get("validation")
        .and_then(Value::as_object)
        .ok_or_else(|| ReadModeError::bad_value("check validation filter must be an object"))?;
    if validation.keys().any(|key| key != "profile") {
        return Err(ReadModeError::unsupported_filter(
            "check validation filter supports only `profile`",
        ));
    }
    let profile = validation
        .get("profile")
        .and_then(Value::as_str)
        .ok_or_else(|| ReadModeError::bad_value("validation profile must be a string"))?;
    const PROFILES: &[&str] = &[
        "meta",
        "cf",
        "cfe",
        "form",
        "dcs",
        "mxl",
        "role",
        "subsystem",
        "interface",
    ];
    if !PROFILES.contains(&profile) {
        return Err(ReadModeError::unsupported_filter(format!(
            "unsupported validation profile `{profile}`"
        )));
    }
    Ok(Some(profile.to_string()))
}

pub(super) fn filter_diff_data(data: &Value, filter: &Value) -> Result<Value, ReadModeError> {
    let filter = filter
        .as_object()
        .ok_or_else(|| ReadModeError::bad_value("diff filter must be an object"))?;
    if filter.is_empty() {
        return Ok(data.clone());
    }
    if filter
        .keys()
        .any(|key| !matches!(key.as_str(), "paths" | "sections"))
    {
        return Err(ReadModeError::unsupported_filter(
            "diff filter supports only `paths` or `sections`",
        ));
    }
    if filter.contains_key("paths") && filter.contains_key("sections") {
        return Err(ReadModeError::bad_value(
            "diff filter accepts either `paths` or `sections`, not both",
        ));
    }
    if let Some(sections) = filter.get("sections") {
        return project_view_sections(data, sections);
    }
    let paths = filter
        .get("paths")
        .and_then(Value::as_array)
        .ok_or_else(|| ReadModeError::bad_value("diff paths must be an array of JSON pointers"))?;
    if paths.is_empty() {
        return Err(ReadModeError::bad_value(
            "diff paths must contain at least one JSON pointer",
        ));
    }
    let mut projected = Map::new();
    if let Some(kind) = data.get("kind") {
        projected.insert("kind".to_string(), kind.clone());
    }
    for path in paths {
        let path = path
            .as_str()
            .ok_or_else(|| ReadModeError::bad_value("diff paths must contain strings"))?;
        if !path.starts_with('/') || path == "/" {
            return Err(ReadModeError::bad_value(
                "diff paths must be non-root JSON pointers",
            ));
        }
        let Some(value) = data.pointer(path) else {
            continue;
        };
        insert_pointer(&mut projected, path, value.clone())?;
    }
    Ok(Value::Object(projected))
}

fn insert_pointer(
    root: &mut Map<String, Value>,
    pointer: &str,
    value: Value,
) -> Result<(), ReadModeError> {
    let segments = pointer
        .split('/')
        .skip(1)
        .map(|segment| segment.replace("~1", "/").replace("~0", "~"))
        .collect::<Vec<_>>();
    let Some((last, parents)) = segments.split_last() else {
        return Err(ReadModeError::bad_value("diff JSON pointer is empty"));
    };
    let mut current = root;
    for segment in parents {
        let entry = current
            .entry(segment.clone())
            .or_insert_with(|| Value::Object(Map::new()));
        current = entry
            .as_object_mut()
            .ok_or_else(|| ReadModeError::bad_value("diff path crosses a non-object JSON value"))?;
    }
    current.insert(last.clone(), value);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{filter_diff_data, project_view_sections, search_scope_prefix, validation_profile};
    use crate::domain::address::QualifiedAddress;
    use serde_json::json;

    #[test]
    fn sections_keep_identity_and_only_selected_optional_slots() {
        let projected = project_view_sections(
            &json!({
                "at": "main:Catalog.Items",
                "kind": "Catalog",
                "title": "Items",
                "props": {"hierarchical": false},
                "branches": [{"at": "main:Catalog.Items.Attribute", "count": 2}],
                "can": [{"op": "props.set"}],
                "limits": ["read-only"]
            }),
            &json!(["props", "can"]),
        )
        .unwrap();

        assert_eq!(
            projected,
            json!({
                "at": "main:Catalog.Items",
                "kind": "Catalog",
                "title": "Items",
                "props": {"hierarchical": false},
                "can": [{"op": "props.set"}]
            })
        );
        assert!(project_view_sections(&projected, &json!(["physicalPath"])).is_err());
    }

    #[test]
    fn logical_scope_maps_to_a_retained_relative_prefix_without_accepting_paths() {
        let root = QualifiedAddress::parse("main:Configuration").unwrap();
        let catalog = QualifiedAddress::parse("main:Catalog.Items").unwrap();
        assert_eq!(search_scope_prefix(&root).unwrap(), None);
        assert_eq!(
            search_scope_prefix(&catalog).unwrap().as_deref(),
            Some("Catalogs/Items")
        );
        let attribute = QualifiedAddress::parse("main:Catalog.Items.Attribute.Code").unwrap();
        assert!(search_scope_prefix(&attribute).is_err());
    }

    #[test]
    fn validation_profile_is_a_closed_union() {
        assert_eq!(
            validation_profile(&json!({"validation": {"profile": "meta"}})).unwrap(),
            Some("meta".to_string())
        );
        assert!(validation_profile(&json!({"validation": {"profile": "shell"}})).is_err());
        assert!(validation_profile(&json!({"validation": "meta"})).is_err());
    }

    #[test]
    fn diff_filter_selects_closed_json_pointer_prefixes() {
        let filtered = filter_diff_data(
            &json!({
                "at": "main:Catalog.Items",
                "kind": "Catalog",
                "title": "Items",
                "props": {"synonym": "Items", "hierarchical": false},
                "can": [{"op": "props.set"}]
            }),
            &json!({"paths": ["/props/synonym"]}),
        )
        .unwrap();
        assert_eq!(
            filtered,
            json!({
                "kind": "Catalog",
                "props": {"synonym": "Items"}
            })
        );
        assert!(filter_diff_data(&filtered, &json!({"command": "rm"})).is_err());
    }
}
