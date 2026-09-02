use crate::domain::address::{NodeKind, QualifiedAddress};
use crate::domain::apply::{OperationRegistry, IMPLEMENTED_APPLY_OPERATIONS};
use crate::infrastructure::metadata_kinds::metadata_kind;
use serde_json::{json, Map, Value};
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

    fn unsupported_section(message: impl Into<String>) -> Self {
        Self {
            code: "unsupported_section",
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

/// How a canonical search matches one source line: a literal needle or a
/// bounded, pre-compiled regular expression. Both report byte offsets of match
/// starts so the caller projects the same logical location either way.
pub(crate) enum SearchMatcher {
    Literal(String),
    Regex(regex::Regex),
}

impl SearchMatcher {
    pub(super) fn match_starts(&self, line: &str) -> Vec<usize> {
        match self {
            Self::Literal(needle) => line
                .match_indices(needle.as_str())
                .map(|(start, _)| start)
                .collect(),
            Self::Regex(pattern) => pattern
                .find_iter(line)
                .filter(|found| !found.is_empty())
                .map(|found| found.start())
                .collect(),
        }
    }
}

const VIEW_IDENTITY_SLOTS: &[&str] = &["at", "kind", "title"];
const VIEW_SECTION_SLOTS: &[&str] = &["props", "branches", "can", "limits", "items"];

/// The operation dictionary of a node, computed from the one closed registry
/// that also validates `apply` calls: registry × applicability to the node
/// kind. `None` means the dictionary is not computed for this kind yet — the
/// caller answers `unsupported_section`, never a valid-looking empty node.
/// The first phase covers the Configuration root and the metadata kinds.
fn computed_can_entries(kind: &str) -> Option<Vec<Value>> {
    // Every parseable node kind carries its dictionary: the applicability
    // model covers all kinds, so the only uncomputable case is a kind the
    // address grammar does not know.
    let node_kind = NodeKind::parse(kind).ok()?;
    Some(
        OperationRegistry::closed()
            .descriptors()
            .iter()
            .filter(|descriptor| descriptor.applies_to(node_kind))
            .map(|descriptor| {
                json!({
                    "op": descriptor.name(),
                    "args": descriptor.skeleton_key(),
                    "implemented": IMPLEMENTED_APPLY_OPERATIONS.contains(&descriptor.name()),
                })
            })
            .collect(),
    )
}

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
    let kind = data.get("kind").and_then(Value::as_str).unwrap_or_default();
    let mut projected = Map::new();
    for slot in VIEW_IDENTITY_SLOTS {
        if let Some(value) = data.get(*slot) {
            projected.insert((*slot).to_string(), value.clone());
        }
    }
    for slot in selected {
        match slot {
            "can" => match computed_can_entries(kind) {
                Some(entries) => {
                    projected.insert(slot.to_string(), Value::Array(entries));
                }
                None => {
                    return Err(ReadModeError::unsupported_section(format!(
                        "view section `can` is not computed for kind `{kind}` yet"
                    )))
                }
            },
            "limits" => {
                return Err(ReadModeError::unsupported_section(
                    "view section `limits` is not computed yet",
                ))
            }
            slot => {
                if let Some(value) = data.get(slot) {
                    projected.insert(slot.to_string(), value.clone());
                }
            }
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
        // Diff compares source-backed node data. Computed sections belong to
        // the View result: the operation dictionary derives from the registry
        // and the node kind, so comparing it never observes a source change.
        if let Some(computed) = sections
            .as_array()
            .into_iter()
            .flatten()
            .find(|section| matches!(section.as_str(), Some("can") | Some("limits")))
        {
            return Err(ReadModeError::unsupported_filter(format!(
                "diff sections compare node data; computed section `{}` belongs to view",
                computed.as_str().unwrap_or_default()
            )));
        }
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
                "branches": [{"at": "main:Catalog.Items.Attribute", "count": 2}]
            }),
            &json!(["props", "can"]),
        )
        .unwrap();

        assert_eq!(projected["at"], "main:Catalog.Items");
        assert_eq!(projected["props"], json!({"hierarchical": false}));
        assert!(
            projected.get("branches").is_none(),
            "unselected sections stay out: {projected}"
        );
        let can = projected["can"].as_array().expect("computed can entries");
        let entry = |op: &str| {
            can.iter()
                .find(|entry| entry["op"] == op)
                .unwrap_or_else(|| panic!("missing `{op}` in {can:?}"))
                .clone()
        };
        // The dictionary comes from the one closed registry × applicability,
        // with the honesty flag mirroring the Run dictionary.
        assert_eq!(
            entry("props.set"),
            json!({"op": "props.set", "args": "values", "implemented": true})
        );
        assert_eq!(
            entry("attribute.add"),
            json!({"op": "attribute.add", "args": "items", "implemented": true})
        );
        assert_eq!(
            entry("object.create"),
            json!({"op": "object.create", "args": "values", "implemented": true})
        );
        assert_eq!(
            entry("form.add"),
            json!({"op": "form.add", "args": "items", "implemented": true})
        );
        // The honesty flag stays false for a registry name without a planner.
        let data_set = project_view_sections(
            &json!({
                "at": "main:Report.Sales.Template.Layout.DataSet.Main",
                "kind": "DataSet",
                "title": "Main",
                "props": {}
            }),
            &json!(["can"]),
        )
        .unwrap();
        let data_set_can = data_set["can"].as_array().expect("computed can entries");
        let dcs_set = data_set_can
            .iter()
            .find(|entry| entry["op"] == "dcs.set")
            .unwrap_or_else(|| panic!("missing `dcs.set` in {data_set_can:?}"));
        assert_eq!(
            dcs_set,
            &json!({"op": "dcs.set", "args": "values", "implemented": false})
        );
        assert_eq!(
            entry("object.remove"),
            json!({"op": "object.remove", "args": "at", "implemented": true})
        );
        assert_eq!(
            entry("help.create"),
            json!({"op": "help.create", "args": "values", "implemented": true})
        );
        assert!(
            can.iter().all(|entry| entry["op"] != "enumValue.add"),
            "a Catalog node must not advertise Enum-only operations: {can:?}"
        );
        assert!(project_view_sections(&projected, &json!(["physicalPath"])).is_err());
    }

    #[test]
    fn uncomputed_sections_answer_typed_unsupported_section() {
        let module = json!({
            "at": "main:CommonModule.Общий",
            "kind": "Module",
            "title": "Common module Общий"
        });
        let projected = project_view_sections(&module, &json!(["can"])).unwrap();
        let can = projected["can"].as_array().expect("module can entries");
        assert!(
            can.iter().any(|entry| entry["op"] == "code.insert"),
            "a module node advertises the code family: {can:?}"
        );
        assert!(
            can.iter().all(|entry| entry["op"] != "attribute.add"),
            "a module node must not advertise metadata-only operations: {can:?}"
        );

        let unknown = json!({"at": "main:Mystery.X", "kind": "Mystery", "title": "X"});
        let refusal = project_view_sections(&unknown, &json!(["can"])).unwrap_err();
        assert_eq!(refusal.code(), "unsupported_section");
        assert!(refusal.to_string().contains("`can`"), "{refusal}");

        let limits =
            project_view_sections(&json!({"kind": "Catalog"}), &json!(["limits"])).unwrap_err();
        assert_eq!(limits.code(), "unsupported_section");

        let configuration =
            project_view_sections(&json!({"kind": "Configuration"}), &json!(["can"])).unwrap();
        let can = configuration["can"].as_array().expect("root can entries");
        assert!(can.iter().any(|entry| entry["op"] == "subsystem.create"));
        assert!(can.iter().any(|entry| entry["op"] == "object.create"));
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
        let computed = filter_diff_data(
            &json!({"kind": "Catalog", "props": {}}),
            &json!({"sections": ["can"]}),
        )
        .unwrap_err();
        assert_eq!(computed.code(), "unsupported_filter");
        assert!(
            computed.to_string().contains("belongs to view"),
            "{computed}"
        );
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
