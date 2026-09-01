use crate::domain::address::QualifiedAddress;
use serde::Serialize;
use serde_json::{Map, Value};
use std::collections::{BTreeSet, HashMap};
use std::fmt;
use std::sync::Mutex;
use uuid::Uuid;

const DEFAULT_LIMIT: usize = 100;
const MAX_LIMIT: usize = 1_000;
const MAX_CURSOR_ENTRIES: usize = 64;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DiffRequest {
    left: QualifiedAddress,
    right: QualifiedAddress,
    filter: Value,
    limit: usize,
    cursor: Option<String>,
}

impl DiffRequest {
    pub(crate) fn new(left: &str, right: &str) -> Result<Self, DiffError> {
        let left = QualifiedAddress::parse(left).map_err(|error| DiffError::BadValue {
            field: "left".to_string(),
            message: error.to_string(),
        })?;
        let right = QualifiedAddress::parse(right).map_err(|error| DiffError::BadValue {
            field: "right".to_string(),
            message: error.to_string(),
        })?;
        Ok(Self {
            left,
            right,
            filter: Value::Object(Map::new()),
            limit: DEFAULT_LIMIT,
            cursor: None,
        })
    }

    pub(crate) fn with_filter(mut self, filter: Value) -> Result<Self, DiffError> {
        validate_filter(&filter)?;
        self.filter = filter;
        Ok(self)
    }

    pub(crate) fn with_limit(mut self, limit: usize) -> Self {
        self.limit = limit.clamp(1, MAX_LIMIT);
        self
    }

    pub(crate) fn with_cursor(mut self, cursor: String) -> Self {
        self.cursor = Some(cursor);
        self
    }

    fn fingerprint(&self) -> String {
        format!(
            "{}|{}|{}|{}",
            self.left,
            self.right,
            canonical_json(&self.filter),
            self.limit
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DiffSource {
    at: QualifiedAddress,
    kind: String,
    revision: String,
    data: Value,
}

impl DiffSource {
    pub(crate) fn new(
        at: QualifiedAddress,
        kind: impl Into<String>,
        revision: impl Into<String>,
        data: Value,
    ) -> Self {
        Self {
            at,
            kind: kind.into(),
            revision: revision.into(),
            data,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct DiffChange {
    path: String,
    left: Option<Value>,
    right: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct DiffPage {
    equal: bool,
    changes: Vec<DiffChange>,
    truncated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    cursor: Option<String>,
}

impl DiffPage {
    pub(crate) fn changes(&self) -> &[DiffChange] {
        &self.changes
    }

    pub(crate) fn truncated(&self) -> bool {
        self.truncated
    }

    pub(crate) fn cursor(&self) -> Option<&str> {
        self.cursor.as_deref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DiffError {
    BadValue { field: String, message: String },
    UnsupportedFilter(String),
    IncomparableNodes,
    InvalidCursor,
    StaleCursor,
}

impl DiffError {
    pub(crate) const fn code(&self) -> &'static str {
        match self {
            Self::BadValue { .. } => "bad_value",
            Self::UnsupportedFilter(_) => "unsupported_filter",
            Self::IncomparableNodes => "incomparable_nodes",
            Self::InvalidCursor => "invalid_cursor",
            Self::StaleCursor => "stale_cursor",
        }
    }
}

impl fmt::Display for DiffError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadValue { field, message } => write!(formatter, "{field}: {message}"),
            Self::UnsupportedFilter(message) => formatter.write_str(message),
            Self::IncomparableNodes => {
                formatter.write_str("diff requires nodes of the same logical kind")
            }
            Self::InvalidCursor => {
                formatter.write_str("diff cursor is invalid or belongs to another question")
            }
            Self::StaleCursor => {
                formatter.write_str("a source revision changed after the diff cursor was issued")
            }
        }
    }
}

#[derive(Debug, Clone)]
struct CursorEntry {
    fingerprint: String,
    left_revision: String,
    right_revision: String,
    offset: usize,
}

#[derive(Debug, Default)]
struct DiffCursorStore {
    entries: Mutex<HashMap<String, CursorEntry>>,
}

impl DiffCursorStore {
    fn issue(&self, entry: CursorEntry) -> Option<String> {
        let mut entries = self.entries.lock().expect("diff cursor store poisoned");
        if entries.len() >= MAX_CURSOR_ENTRIES {
            let key = entries.keys().next()?.clone();
            entries.remove(&key);
        }
        let token = format!("dc1.{}", Uuid::new_v4().simple());
        entries.insert(token.clone(), entry);
        Some(token)
    }

    fn read(&self, token: &str) -> Result<CursorEntry, DiffError> {
        if token.len() != 36
            || !token.starts_with("dc1.")
            || !token[4..]
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        {
            return Err(DiffError::InvalidCursor);
        }
        self.entries
            .lock()
            .expect("diff cursor store poisoned")
            .get(token)
            .cloned()
            .ok_or(DiffError::InvalidCursor)
    }
}

#[derive(Debug, Default)]
pub(crate) struct DiffHandler {
    cursors: DiffCursorStore,
}

impl DiffHandler {
    pub(crate) fn compare(
        &self,
        request: &DiffRequest,
        left: &DiffSource,
        right: &DiffSource,
    ) -> Result<DiffPage, DiffError> {
        if request.left != left.at || request.right != right.at {
            return Err(DiffError::BadValue {
                field: "source".to_string(),
                message: "sources do not match the logical diff request".to_string(),
            });
        }
        if left.kind != right.kind {
            return Err(DiffError::IncomparableNodes);
        }
        let left_data = project_data(&left.data, &request.filter)?;
        let right_data = project_data(&right.data, &request.filter)?;
        if left_data.get("kind") != right_data.get("kind") {
            return Err(DiffError::IncomparableNodes);
        }

        let fingerprint = request.fingerprint();
        let offset = if let Some(token) = request.cursor.as_deref() {
            let entry = self.cursors.read(token)?;
            if entry.fingerprint != fingerprint {
                return Err(DiffError::InvalidCursor);
            }
            if entry.left_revision != left.revision || entry.right_revision != right.revision {
                return Err(DiffError::StaleCursor);
            }
            entry.offset
        } else {
            0
        };
        let mut skip = offset;
        let mut changes = Vec::with_capacity(request.limit.saturating_add(1));
        collect_changes(
            "",
            &left_data,
            &right_data,
            &mut skip,
            request.limit.saturating_add(1),
            &mut changes,
        );
        let truncated = changes.len() > request.limit;
        changes.truncate(request.limit);
        let next_cursor = truncated
            .then(|| {
                self.cursors.issue(CursorEntry {
                    fingerprint,
                    left_revision: left.revision.clone(),
                    right_revision: right.revision.clone(),
                    offset: offset.saturating_add(changes.len()),
                })
            })
            .flatten();
        Ok(DiffPage {
            equal: changes.is_empty() && !truncated,
            changes,
            truncated,
            cursor: next_cursor,
        })
    }
}

fn collect_changes(
    path: &str,
    left: &Value,
    right: &Value,
    skip: &mut usize,
    limit: usize,
    changes: &mut Vec<DiffChange>,
) {
    if left == right || changes.len() >= limit {
        return;
    }
    match (left, right) {
        (Value::Object(left), Value::Object(right)) => {
            let keys = left
                .keys()
                .chain(right.keys())
                .map(String::as_str)
                .collect::<BTreeSet<_>>();
            for key in keys {
                let escaped = key.replace('~', "~0").replace('/', "~1");
                let child_path = format!("{path}/{escaped}");
                match (left.get(key), right.get(key)) {
                    (Some(left), Some(right)) => {
                        collect_changes(&child_path, left, right, skip, limit, changes)
                    }
                    (left, right) => emit_change(&child_path, left, right, skip, limit, changes),
                }
                if changes.len() >= limit {
                    break;
                }
            }
        }
        (Value::Array(left), Value::Array(right)) => {
            for index in 0..left.len().max(right.len()) {
                let child_path = format!("{path}/{index}");
                match (left.get(index), right.get(index)) {
                    (Some(left), Some(right)) => {
                        collect_changes(&child_path, left, right, skip, limit, changes)
                    }
                    (left, right) => emit_change(&child_path, left, right, skip, limit, changes),
                }
                if changes.len() >= limit {
                    break;
                }
            }
        }
        _ => emit_change(
            path_or_root(path),
            Some(left),
            Some(right),
            skip,
            limit,
            changes,
        ),
    }
}

fn emit_change(
    path: &str,
    left: Option<&Value>,
    right: Option<&Value>,
    skip: &mut usize,
    limit: usize,
    changes: &mut Vec<DiffChange>,
) {
    if *skip > 0 {
        *skip -= 1;
        return;
    }
    if changes.len() < limit {
        changes.push(DiffChange {
            path: path.to_string(),
            left: left.cloned(),
            right: right.cloned(),
        });
    }
}

fn path_or_root(path: &str) -> &str {
    if path.is_empty() {
        "/"
    } else {
        path
    }
}

fn validate_filter(filter: &Value) -> Result<(), DiffError> {
    let Some(filter) = filter.as_object() else {
        return Err(DiffError::BadValue {
            field: "filter".to_string(),
            message: "diff filter must be an object".to_string(),
        });
    };
    if filter.keys().any(|key| key != "paths" && key != "sections") {
        return Err(DiffError::UnsupportedFilter(
            "diff filter supports only paths or sections".to_string(),
        ));
    }
    if filter.contains_key("paths") && filter.contains_key("sections") {
        return Err(DiffError::BadValue {
            field: "filter".to_string(),
            message: "diff filter accepts either paths or sections, not both".to_string(),
        });
    }
    Ok(())
}

fn project_data(data: &Value, filter: &Value) -> Result<Value, DiffError> {
    validate_filter(filter)?;
    let Some(filter) = filter.as_object() else {
        unreachable!()
    };
    if filter.is_empty() {
        return Ok(data.clone());
    }
    let data_value = data.clone();
    let data = data_value.as_object().ok_or_else(|| DiffError::BadValue {
        field: "data".to_string(),
        message: "diff source data must be an object".to_string(),
    })?;
    let mut projected = Map::new();
    if let Some(kind) = data.get("kind") {
        projected.insert("kind".to_string(), kind.clone());
    }
    if let Some(sections) = filter.get("sections") {
        let sections = sections.as_array().ok_or_else(|| DiffError::BadValue {
            field: "filter.sections".to_string(),
            message: "diff sections must be an array of strings".to_string(),
        })?;
        const ALLOWED: &[&str] = &["at", "title", "props", "branches", "can", "limits", "items"];
        if sections.is_empty() {
            return Err(DiffError::BadValue {
                field: "filter.sections".to_string(),
                message: "diff sections must not be empty".to_string(),
            });
        }
        for section in sections {
            let section = section.as_str().ok_or_else(|| DiffError::BadValue {
                field: "filter.sections".to_string(),
                message: "diff sections must contain strings".to_string(),
            })?;
            if !ALLOWED.contains(&section) {
                return Err(DiffError::UnsupportedFilter(format!(
                    "unsupported diff section `{section}`"
                )));
            }
            if let Some(value) = data.get(section) {
                projected.insert(section.to_string(), value.clone());
            }
        }
        return Ok(Value::Object(projected));
    }
    let paths = filter["paths"]
        .as_array()
        .ok_or_else(|| DiffError::BadValue {
            field: "filter.paths".to_string(),
            message: "diff paths must be an array of JSON pointers".to_string(),
        })?;
    if paths.is_empty() {
        return Err(DiffError::BadValue {
            field: "filter.paths".to_string(),
            message: "diff paths must not be empty".to_string(),
        });
    }
    for path in paths {
        let path = path.as_str().ok_or_else(|| DiffError::BadValue {
            field: "filter.paths".to_string(),
            message: "diff paths must contain strings".to_string(),
        })?;
        if !path.starts_with('/') || path == "/" {
            return Err(DiffError::BadValue {
                field: "filter.paths".to_string(),
                message: "diff paths must be non-root JSON pointers".to_string(),
            });
        }
        if let Some(value) = data_value.pointer(path) {
            insert_pointer(&mut projected, path, value.clone())?;
        }
    }
    Ok(Value::Object(projected))
}

fn insert_pointer(
    root: &mut Map<String, Value>,
    pointer: &str,
    value: Value,
) -> Result<(), DiffError> {
    let segments = pointer
        .split('/')
        .skip(1)
        .map(|segment| segment.replace("~1", "/").replace("~0", "~"))
        .collect::<Vec<_>>();
    let Some((last, parents)) = segments.split_last() else {
        return Err(DiffError::BadValue {
            field: "filter.paths".to_string(),
            message: "diff JSON pointer is empty".to_string(),
        });
    };
    let mut current = root;
    for segment in parents {
        let entry = current
            .entry(segment.clone())
            .or_insert_with(|| Value::Object(Map::new()));
        current = entry.as_object_mut().ok_or_else(|| DiffError::BadValue {
            field: "filter.paths".to_string(),
            message: "diff path crosses a non-object JSON value".to_string(),
        })?;
    }
    current.insert(last.clone(), value);
    Ok(())
}

fn canonical_json(value: &Value) -> String {
    match value {
        Value::Object(map) => map
            .iter()
            .map(|(key, value)| format!("{}:{}", key, canonical_json(value)))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>()
            .join(","),
        Value::Array(values) => values
            .iter()
            .map(canonical_json)
            .collect::<Vec<_>>()
            .join(","),
        _ => value.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::{DiffError, DiffHandler, DiffRequest, DiffSource};
    use crate::domain::address::QualifiedAddress;
    use serde_json::json;

    fn source(at: &str, kind: &str, revision: &str, value: i64) -> DiffSource {
        DiffSource::new(
            QualifiedAddress::parse(at).unwrap(),
            kind,
            revision,
            json!({"kind": kind, "props": {"value": value}, "items": (0..50).map(|n| json!({"n": n + value})).collect::<Vec<_>>() }),
        )
    }

    #[test]
    fn diff_rejects_incomparable_logical_kinds() {
        let handler = DiffHandler::default();
        let request = DiffRequest::new("main:Catalog.Items", "main:Role.Manager").unwrap();
        let error = handler
            .compare(
                &request,
                &source("main:Catalog.Items", "Catalog", "left-1", 1),
                &source("main:Role.Manager", "Role", "right-1", 1),
            )
            .unwrap_err();
        assert_eq!(error.code(), "incomparable_nodes");
    }

    #[test]
    fn diff_limit_bounds_materialized_changes_and_issues_a_cursor() {
        let handler = DiffHandler::default();
        let request = DiffRequest::new("main:Catalog.Items", "main:Catalog.Items")
            .unwrap()
            .with_limit(2);
        let page = handler
            .compare(
                &request,
                &source("main:Catalog.Items", "Catalog", "left-1", 1),
                &source("main:Catalog.Items", "Catalog", "right-1", 2),
            )
            .unwrap();
        assert_eq!(page.changes().len(), 2);
        assert!(page.truncated());
        assert!(page.cursor().is_some());
    }

    #[test]
    fn diff_cursor_is_bound_to_both_source_revisions() {
        let handler = DiffHandler::default();
        let request = DiffRequest::new("main:Catalog.Items", "main:Catalog.Items")
            .unwrap()
            .with_limit(1);
        let left = source("main:Catalog.Items", "Catalog", "left-1", 1);
        let right = source("main:Catalog.Items", "Catalog", "right-1", 2);
        let first = handler.compare(&request, &left, &right).unwrap();
        let cursor = first.cursor().unwrap().to_string();
        let stale_left = source("main:Catalog.Items", "Catalog", "left-2", 1);
        let stale = handler
            .compare(
                &request.clone().with_cursor(cursor.clone()),
                &stale_left,
                &right,
            )
            .unwrap_err();
        assert_eq!(stale.code(), "stale_cursor");
        let stale_right = source("main:Catalog.Items", "Catalog", "right-2", 2);
        let stale = handler
            .compare(&request.with_cursor(cursor), &left, &stale_right)
            .unwrap_err();
        assert_eq!(stale.code(), "stale_cursor");
    }

    #[test]
    fn diff_cursor_cannot_be_replayed_for_another_question() {
        let handler = DiffHandler::default();
        let request = DiffRequest::new("main:Catalog.Items", "main:Catalog.Items")
            .unwrap()
            .with_limit(1);
        let left = source("main:Catalog.Items", "Catalog", "left-1", 1);
        let right = source("main:Catalog.Items", "Catalog", "right-1", 2);
        let first = handler.compare(&request, &left, &right).unwrap();
        let cursor = first.cursor().unwrap();
        let replay = request.with_cursor(cursor.to_string()).with_limit(2);
        assert!(matches!(
            handler.compare(&replay, &left, &right),
            Err(DiffError::InvalidCursor)
        ));
    }

    #[test]
    fn diff_cursor_continues_from_the_bounded_change_offset() {
        let handler = DiffHandler::default();
        let request = DiffRequest::new("main:Catalog.Items", "main:Catalog.Items")
            .unwrap()
            .with_limit(2);
        let left = source("main:Catalog.Items", "Catalog", "left-1", 1);
        let right = source("main:Catalog.Items", "Catalog", "right-1", 2);
        let first = handler.compare(&request, &left, &right).unwrap();
        let second = handler
            .compare(
                &request.with_cursor(first.cursor().unwrap().to_string()),
                &left,
                &right,
            )
            .unwrap();
        assert_eq!(first.changes()[0].path, "/items/0/n");
        assert_eq!(second.changes()[0].path, "/items/2/n");
        assert_ne!(first.changes()[0].path, second.changes()[0].path);
    }

    #[test]
    fn diff_paths_filter_is_closed_and_preserves_kind_identity() {
        let handler = DiffHandler::default();
        let request = DiffRequest::new("main:Catalog.Items", "main:Catalog.Items")
            .unwrap()
            .with_filter(json!({"paths": ["/props/value"]}))
            .unwrap();
        let page = handler
            .compare(
                &request,
                &source("main:Catalog.Items", "Catalog", "left-1", 1),
                &source("main:Catalog.Items", "Catalog", "right-1", 2),
            )
            .unwrap();
        assert_eq!(page.changes()[0].path, "/props/value");
        assert!(!page.changes()[0].left.as_ref().unwrap().is_null());
    }
}
