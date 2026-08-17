//! Deferred delivery of oversized typed reader results (ADR-0070).
//!
//! The application layer owns the threshold and the manifest; the bounded
//! snapshot store lives in infrastructure. A continuation call re-enters the
//! same tool with `resultRef` and a selection and is served from the immutable
//! snapshot without re-reading the source.

use std::sync::Arc;

use serde_json::{json, Map, Value};

use super::result_store::{unix_ms_now, ResultStore, SnapshotIdentity, StoredView};

/// Serialized `OperationResult.data` above this many bytes is delivered as a
/// deferred manifest (ADR-0070: ~16 KiB ≈ 4 000 o200k_base tokens).
pub const DEFAULT_THRESHOLD_BYTES: usize = 16 * 1024;

const CONTINUATION_PAGE_SIZE: usize = 50;

pub const CONTINUATION_ARGS: &[&str] = &["delivery", "filter", "page", "resultRef", "section"];

/// Narrowed native readers that publish continuation arguments and defer
/// oversized results. `cf.info` keeps its own `section` drill-down, and
/// `xdto.info`/`meta.info` keep their own selection disciplines, so they stay
/// outside this first slice.
pub const SUPPORTED_OPERATIONS: &[&str] = &[
    "dcs-info",
    "form-info",
    "mxl-info",
    "role-info",
    "subsystem-info",
];

pub fn supports_operation(operation: &str) -> bool {
    SUPPORTED_OPERATIONS.contains(&operation)
}

pub fn supports(spec: &super::ToolSpec) -> bool {
    matches!(
        spec.handler,
        super::ToolHandler::NativeOperation { operation, .. } if supports_operation(operation)
    )
}

pub struct DeferredDelivery {
    pub store: Arc<ResultStore>,
    pub threshold_bytes: usize,
}

impl Default for DeferredDelivery {
    fn default() -> Self {
        Self {
            store: Arc::new(ResultStore::default()),
            threshold_bytes: DEFAULT_THRESHOLD_BYTES,
        }
    }
}

/// Canonical identity of the original read arguments: continuation keys are
/// excluded, the rest is serialized with sorted keys.
pub fn args_identity(args: &Map<String, Value>) -> String {
    let mut sorted: Vec<(&String, &Value)> = args
        .iter()
        .filter(|(key, _)| !CONTINUATION_ARGS.contains(&key.as_str()))
        .collect();
    sorted.sort_by(|left, right| left.0.cmp(right.0));
    let mut canonical = Map::new();
    for (key, value) in sorted {
        canonical.insert(key.clone(), value.clone());
    }
    Value::Object(canonical).to_string()
}

pub struct Selection {
    pub section: Option<String>,
    pub filter: Option<String>,
    pub page: Option<u64>,
    pub full: bool,
}

impl Selection {
    pub fn from_args(args: &Map<String, Value>) -> Self {
        Self {
            section: args
                .get("section")
                .and_then(Value::as_str)
                .map(str::to_string),
            filter: args
                .get("filter")
                .and_then(Value::as_str)
                .map(str::to_string),
            page: args.get("page").and_then(Value::as_u64),
            full: args.get("delivery").and_then(Value::as_str) == Some("full"),
        }
    }

    pub fn requests_anything(&self) -> bool {
        self.section.is_some() || self.filter.is_some() || self.page.is_some() || self.full
    }
}

fn section_entity_count(value: &Value) -> usize {
    match value {
        Value::Array(items) => items.len(),
        Value::Object(map) => map.len(),
        _ => 1,
    }
}

fn snapshot_json(snapshot: &SnapshotIdentity) -> Value {
    json!({
        "workspaceEpoch": snapshot.workspace_epoch,
        "cacheRoot": snapshot.cache_root,
        "asOf": snapshot.as_of_unix_ms,
    })
}

/// Builds the deferred manifest for a stored full result.
pub fn manifest(
    summary: &str,
    data: &Value,
    bytes: usize,
    result_ref: &str,
    snapshot: &SnapshotIdentity,
    expires_at_unix_ms: u64,
) -> Value {
    let mut sections = Map::new();
    let mut suggested: Vec<(String, usize)> = Vec::new();
    if let Value::Object(map) = data {
        for (key, value) in map {
            let count = section_entity_count(value);
            sections.insert(key.clone(), json!(count));
            if matches!(value, Value::Array(_) | Value::Object(_)) {
                suggested.push((key.clone(), count));
            }
        }
    }
    suggested.sort_by(|left, right| right.1.cmp(&left.1).then(left.0.cmp(&right.0)));
    let suggested_selections: Vec<Value> = suggested
        .into_iter()
        .map(|(section, _)| json!({ "section": section }))
        .collect();
    json!({
        "state": "deferred",
        "summary": summary,
        "sections": Value::Object(sections),
        "bytes": bytes,
        "estimatedTokens": bytes.div_ceil(4),
        "suggestedSelections": suggested_selections,
        "resultRef": result_ref,
        "snapshot": snapshot_json(snapshot),
        "expiresAt": expires_at_unix_ms,
    })
}

#[derive(Debug, PartialEq, Eq)]
pub struct SliceError {
    pub code: &'static str,
    pub message: String,
}

impl SliceError {
    fn unknown_section(section: &str, data: &Value) -> Self {
        let known: Vec<&str> = match data {
            Value::Object(map) => map.keys().map(String::as_str).collect(),
            _ => Vec::new(),
        };
        Self {
            code: "selection_unknown_section",
            message: format!(
                "section {section:?} is not part of the stored result; known sections: {}",
                known.join(", ")
            ),
        }
    }
}

fn entity_matches(entity: &Value, needle: &str) -> bool {
    let needle = needle.to_lowercase();
    match entity {
        Value::String(text) => text.to_lowercase().contains(&needle),
        Value::Object(map) => ["name", "Name", "id", "path", "object"].iter().any(|key| {
            map.get(*key)
                .and_then(Value::as_str)
                .is_some_and(|text| text.to_lowercase().contains(&needle))
        }),
        _ => false,
    }
}

/// Serves a byte-stable slice of the stored snapshot for a continuation call.
pub fn slice(
    view: &StoredView,
    result_ref: &str,
    selection: &Selection,
) -> Result<Value, SliceError> {
    if selection.full {
        return Ok(json!({
            "state": "full",
            "resultRef": result_ref,
            "snapshot": snapshot_json(&view.snapshot),
            "expiresAt": view.expires_at_unix_ms,
            "value": view.data,
        }));
    }
    let Some(section) = selection.section.as_deref() else {
        // A bare resultRef re-serves the manifest so the caller can re-orient.
        return Ok(manifest(
            "deferred result manifest re-served",
            &view.data,
            view.bytes,
            result_ref,
            &view.snapshot,
            view.expires_at_unix_ms,
        ));
    };
    let Some(section_value) = view.data.get(section) else {
        return Err(SliceError::unknown_section(section, &view.data));
    };
    let mut envelope = Map::new();
    envelope.insert("state".to_string(), json!("slice"));
    envelope.insert("resultRef".to_string(), json!(result_ref));
    envelope.insert("section".to_string(), json!(section));
    envelope.insert("snapshot".to_string(), snapshot_json(&view.snapshot));
    envelope.insert("expiresAt".to_string(), json!(view.expires_at_unix_ms));
    match section_value {
        Value::Array(items) => {
            let filtered: Vec<&Value> = match selection.filter.as_deref() {
                Some(needle) => items
                    .iter()
                    .filter(|item| entity_matches(item, needle))
                    .collect(),
                None => items.iter().collect(),
            };
            let total = filtered.len();
            let page = selection.page.unwrap_or(1).max(1) as usize;
            let start = (page - 1).saturating_mul(CONTINUATION_PAGE_SIZE);
            let page_items: Vec<Value> = filtered
                .into_iter()
                .skip(start)
                .take(CONTINUATION_PAGE_SIZE)
                .cloned()
                .collect();
            envelope.insert("totalInSection".to_string(), json!(total));
            envelope.insert("page".to_string(), json!(page));
            envelope.insert("pageSize".to_string(), json!(CONTINUATION_PAGE_SIZE));
            if let Some(filter) = selection.filter.as_deref() {
                envelope.insert("filter".to_string(), json!(filter));
            }
            envelope.insert("items".to_string(), Value::Array(page_items));
        }
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let filtered: Vec<&String> = match selection.filter.as_deref() {
                Some(needle) => {
                    let needle = needle.to_lowercase();
                    keys.into_iter()
                        .filter(|key| key.to_lowercase().contains(&needle))
                        .collect()
                }
                None => keys,
            };
            let total = filtered.len();
            let page = selection.page.unwrap_or(1).max(1) as usize;
            let start = (page - 1).saturating_mul(CONTINUATION_PAGE_SIZE);
            let mut page_map = Map::new();
            for key in filtered
                .into_iter()
                .skip(start)
                .take(CONTINUATION_PAGE_SIZE)
            {
                page_map.insert(key.clone(), map[key].clone());
            }
            envelope.insert("totalInSection".to_string(), json!(total));
            envelope.insert("page".to_string(), json!(page));
            envelope.insert("pageSize".to_string(), json!(CONTINUATION_PAGE_SIZE));
            if let Some(filter) = selection.filter.as_deref() {
                envelope.insert("filter".to_string(), json!(filter));
            }
            envelope.insert("value".to_string(), Value::Object(page_map));
        }
        other => {
            envelope.insert("value".to_string(), other.clone());
        }
    }
    Ok(Value::Object(envelope))
}

pub fn now_unix_ms() -> u64 {
    unix_ms_now()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(data: Value) -> StoredView {
        StoredView {
            data,
            snapshot: SnapshotIdentity {
                workspace_epoch: 3,
                cache_root: "/ws".to_string(),
                as_of_unix_ms: 1000,
            },
            bytes: 20000,
            expires_at_unix_ms: 2000,
        }
    }

    #[test]
    fn identity_ignores_continuation_keys_and_sorts() {
        let mut original = Map::new();
        original.insert("role".to_string(), json!("Full"));
        original.insert("cwd".to_string(), json!("/ws"));
        let mut continuation = original.clone();
        continuation.insert("resultRef".to_string(), json!("res-1"));
        continuation.insert("section".to_string(), json!("rights"));
        continuation.insert("page".to_string(), json!(2));
        assert_eq!(args_identity(&original), args_identity(&continuation));
        let mut reordered = Map::new();
        reordered.insert("cwd".to_string(), json!("/ws"));
        reordered.insert("role".to_string(), json!("Full"));
        assert_eq!(args_identity(&original), args_identity(&reordered));
    }

    #[test]
    fn manifest_counts_entities_and_suggests_largest_sections() {
        let data = json!({
            "rights": [1, 2, 3],
            "summary": "text",
            "objects": {"a": 1, "b": 2, "c": 3, "d": 4},
        });
        let snapshot = SnapshotIdentity {
            workspace_epoch: 3,
            cache_root: "/ws".to_string(),
            as_of_unix_ms: 1000,
        };
        let manifest = manifest("big role", &data, 40000, "res-9", &snapshot, 2000);
        assert_eq!(manifest["state"], "deferred");
        assert_eq!(manifest["sections"]["rights"], 3);
        assert_eq!(manifest["sections"]["objects"], 4);
        assert_eq!(manifest["sections"]["summary"], 1);
        assert_eq!(manifest["suggestedSelections"][0]["section"], "objects");
        assert_eq!(manifest["suggestedSelections"][1]["section"], "rights");
        assert_eq!(manifest["resultRef"], "res-9");
        assert_eq!(manifest["snapshot"]["workspaceEpoch"], 3);
        assert_eq!(manifest["bytes"], 40000);
        assert_eq!(manifest["estimatedTokens"], 10000);
    }

    #[test]
    fn bare_result_ref_re_serves_the_manifest() {
        let view = view(json!({"rights": [1, 2]}));
        let selection = Selection {
            section: None,
            filter: None,
            page: None,
            full: false,
        };
        let value = slice(&view, "res-1", &selection).unwrap();
        assert_eq!(value["state"], "deferred");
        assert_eq!(value["resultRef"], "res-1");
    }

    #[test]
    fn slices_are_byte_stable_and_filtered() {
        let view = view(json!({
            "rights": [
                {"name": "Catalog.Валюты", "read": true},
                {"name": "Catalog.Номенклатура", "read": true},
                {"name": "Document.Заказ", "read": false},
            ]
        }));
        let selection = Selection {
            section: Some("rights".to_string()),
            filter: Some("catalog".to_string()),
            page: None,
            full: false,
        };
        let first = slice(&view, "res-1", &selection).unwrap();
        let second = slice(&view, "res-1", &selection).unwrap();
        assert_eq!(
            serde_json::to_vec(&first).unwrap(),
            serde_json::to_vec(&second).unwrap()
        );
        assert_eq!(first["totalInSection"], 2);
        assert_eq!(first["items"].as_array().unwrap().len(), 2);
        assert_eq!(first["state"], "slice");
    }

    #[test]
    fn unknown_section_is_a_stable_error() {
        let view = view(json!({"rights": []}));
        let selection = Selection {
            section: Some("nope".to_string()),
            filter: None,
            page: None,
            full: false,
        };
        let error = slice(&view, "res-1", &selection).unwrap_err();
        assert_eq!(error.code, "selection_unknown_section");
        assert!(error.message.contains("rights"));
    }

    #[test]
    fn full_delivery_returns_the_whole_value() {
        let data = json!({"rights": [1, 2, 3]});
        let view = view(data.clone());
        let selection = Selection {
            section: None,
            filter: None,
            page: None,
            full: true,
        };
        let value = slice(&view, "res-1", &selection).unwrap();
        assert_eq!(value["state"], "full");
        assert_eq!(value["value"], data);
    }
}
