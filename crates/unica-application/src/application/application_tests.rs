//! Снимки схем изменяющих инструментов и их отпечатки. Проверку снимков
//! зовёт и реентерабельная проверка формата в `unica-coder`, поэтому она
//! открыта под `test-support`.

use crate::application::*;
#[cfg(test)]
use serde_json::json;
use serde_json::Value;

// Call on a cloned schema before existing write_canonical. Only schema nodes
// lose annotations; const/default/enum values and property names are data.
fn remove_schema_descriptions(schema: &mut Value) {
    let Value::Object(object) = schema else {
        return;
    };
    object.remove("description");
    for (keyword, value) in object {
        match keyword.as_str() {
            "properties" | "patternProperties" | "$defs" | "definitions" | "dependentSchemas" => {
                if let Value::Object(schemas) = value {
                    for child in schemas.values_mut() {
                        remove_schema_descriptions(child);
                    }
                }
            }
            "allOf" | "anyOf" | "oneOf" | "prefixItems" => {
                if let Value::Array(schemas) = value {
                    for child in schemas {
                        remove_schema_descriptions(child);
                    }
                }
            }
            "items" => match value {
                Value::Array(schemas) => {
                    for child in schemas {
                        remove_schema_descriptions(child);
                    }
                }
                child => remove_schema_descriptions(child),
            },
            "additionalProperties"
            | "unevaluatedProperties"
            | "additionalItems"
            | "unevaluatedItems"
            | "contains"
            | "propertyNames"
            | "not"
            | "if"
            | "then"
            | "else"
            | "contentSchema" => remove_schema_descriptions(value),
            "dependencies" => {
                if let Value::Object(dependencies) = value {
                    for dependency in dependencies.values_mut() {
                        if dependency.is_object() {
                            remove_schema_descriptions(dependency);
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

#[test]
fn native_schema_fingerprint_ignores_annotations_but_keeps_description_fields() {
    let schema = json!({
        "type": "object", "description": "Old wording",
        "properties": {
            "description": {"type": "string", "description": "Explain the field"},
            "payload": {"const": {"description": "literal data"}}
        },
        "required": ["description"],
        "allOf": [{"description": "Nested wording", "minProperties": 1}]
    });
    let mut edited = schema.clone();
    edited["description"] = json!("Clear wording");
    edited["properties"]["description"]["description"] = json!("Clear field wording");
    edited["allOf"][0]["description"] = json!("Clear nested wording");
    assert_eq!(
        native_mutation_schema_signature(&schema),
        native_mutation_schema_signature(&edited)
    );

    let mut type_changed = schema.clone();
    type_changed["properties"]["description"]["type"] = json!("integer");
    assert_ne!(
        native_mutation_schema_signature(&schema),
        native_mutation_schema_signature(&type_changed)
    );
    let mut removed = schema.clone();
    removed["properties"]
        .as_object_mut()
        .unwrap()
        .remove("description");
    assert_ne!(
        native_mutation_schema_signature(&schema),
        native_mutation_schema_signature(&removed)
    );
    let mut constant_changed = schema.clone();
    constant_changed["properties"]["payload"]["const"]["description"] = json!("changed data");
    assert_ne!(
        native_mutation_schema_signature(&schema),
        native_mutation_schema_signature(&constant_changed)
    );
}

#[cfg(test)]
/// The compared form of a mutating tool's input schema: annotations
/// removed, keys sorted, one value per indented line. Snapshot files use
/// it so independent additions to one schema merge as separate lines.
fn native_mutation_schema_signature(schema: &Value) -> String {
    let mut contract = schema.clone();
    remove_schema_descriptions(&mut contract);
    canonical_pretty_json(&contract)
}

/// Deterministic pretty JSON: the workspace enables `preserve_order`, so
/// object keys are sorted here instead of relying on map order.
fn canonical_pretty_json(value: &Value) -> String {
    fn pad(depth: usize, output: &mut String) {
        for _ in 0..depth {
            output.push_str("  ");
        }
    }
    fn write(value: &Value, depth: usize, output: &mut String) {
        match value {
            Value::Array(items) if !items.is_empty() => {
                output.push_str("[\n");
                for (index, item) in items.iter().enumerate() {
                    pad(depth + 1, output);
                    write(item, depth + 1, output);
                    if index + 1 != items.len() {
                        output.push(',');
                    }
                    output.push('\n');
                }
                pad(depth, output);
                output.push(']');
            }
            Value::Object(object) if !object.is_empty() => {
                output.push_str("{\n");
                let mut keys = object.keys().collect::<Vec<_>>();
                keys.sort_unstable();
                for (index, key) in keys.iter().enumerate() {
                    pad(depth + 1, output);
                    output.push_str(&serde_json::to_string(key).expect("schema key serializes"));
                    output.push_str(": ");
                    write(&object[key.as_str()], depth + 1, output);
                    if index + 1 != keys.len() {
                        output.push(',');
                    }
                    output.push('\n');
                }
                pad(depth, output);
                output.push('}');
            }
            _ => output.push_str(&serde_json::to_string(value).expect("schema scalar serializes")),
        }
    }
    let mut output = String::new();
    write(value, 0, &mut output);
    output.push('\n');
    output
}

#[test]
fn native_mutation_schema_fingerprint_detects_nested_contract_changes() {
    let tool = tools()
        .into_iter()
        .find(|tool| tool.name == "unica.cf.edit")
        .expect("cf.edit is registered");
    let schema = input_schema_for_tool(&tool);
    let mut changed = schema.clone();
    changed["properties"]["dryRun"]["type"] = json!("string");

    assert_ne!(
        native_mutation_schema_signature(&schema),
        native_mutation_schema_signature(&changed),
        "a nested schema type change must alter the complete fingerprint",
    );
}

/// The snapshot file of one tool: its operation and the compared schema.
fn native_mutation_snapshot(operation: &str, schema: &Value) -> String {
    let mut contract = schema.clone();
    remove_schema_descriptions(&mut contract);
    canonical_pretty_json(&json!({"operation": operation, "inputSchema": contract}))
}

#[test]
fn native_mutation_surface_has_exact_operations_and_schemas() {
    check_native_mutation_surface();
}

/// Изменяющие инструменты и их схемы совпадают со снимками
/// `tests/fixtures/mutation-schemas` этого пакета.
pub fn check_native_mutation_surface() {
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};

    const UPDATE: &str = "UNICA_UPDATE_SCHEMA_SNAPSHOTS";
    let actual = tools()
        .into_iter()
        .filter(|tool| tool.execution.is_mutating())
        .filter(|tool| {
            matches!(
                tool.handler,
                ToolHandler::NativeOperation { .. } | ToolHandler::Metadata { .. }
            )
        })
        .map(|tool| {
            let operation = match tool.handler {
                ToolHandler::NativeOperation { operation, .. } => operation.to_string(),
                ToolHandler::Metadata { operation } => format!("metadata:{operation:?}"),
                _ => unreachable!(),
            };
            let snapshot = native_mutation_snapshot(&operation, &input_schema_for_tool(&tool));
            (tool.name.to_string(), snapshot)
        })
        .collect::<BTreeMap<_, _>>();
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mutation-schemas");
    let recorded = |directory: &Path| -> BTreeMap<String, (PathBuf, String)> {
        let Ok(entries) = std::fs::read_dir(directory) else {
            return BTreeMap::new();
        };
        entries
            .map(|entry| entry.expect("snapshot entry is readable").path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
            .map(|path| {
                let name = path
                    .file_stem()
                    .and_then(|stem| stem.to_str())
                    .expect("snapshot name is UTF-8")
                    .to_string();
                // A Windows checkout may convert the line endings.
                let text = std::fs::read_to_string(&path)
                    .expect("snapshot is UTF-8")
                    .replace("\r\n", "\n");
                (name, (path, text))
            })
            .collect()
    };
    if std::env::var(UPDATE).as_deref() == Ok("1") {
        std::fs::create_dir_all(&directory).expect("create the snapshot directory");
        for (name, (path, _)) in recorded(&directory) {
            if !actual.contains_key(&name) {
                std::fs::remove_file(path).expect("remove a stale snapshot");
            }
        }
        for (name, snapshot) in &actual {
            // Thread id and clock keep parallel updates in one run apart.
            let staged = directory.join(format!(
                ".{name}.{:?}.{}.json.tmp",
                std::thread::current().id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |elapsed| elapsed.as_nanos())
            ));
            std::fs::write(&staged, snapshot).expect("stage a snapshot");
            std::fs::rename(&staged, directory.join(format!("{name}.json")))
                .expect("publish a snapshot");
        }
    }
    let recorded = recorded(&directory);
    let mut problems = Vec::new();
    for name in actual.keys().filter(|name| !recorded.contains_key(*name)) {
        problems.push(format!("{name}: snapshot {name}.json is missing"));
    }
    for name in recorded.keys().filter(|name| !actual.contains_key(*name)) {
        problems.push(format!(
            "{name}: snapshot exists, but no mutating native tool has this name"
        ));
    }
    for (name, snapshot) in &actual {
        let Some((_, text)) = recorded.get(name) else {
            continue;
        };
        if text == snapshot {
            continue;
        }
        if text.trim_end() == snapshot.trim_end() {
            problems.push(format!(
                "{name}: {name}.json differs only in its final line ending"
            ));
            continue;
        }
        let expected = text.lines().collect::<Vec<_>>();
        let observed = snapshot.lines().collect::<Vec<_>>();
        let line = expected
            .iter()
            .zip(&observed)
            .position(|(left, right)| left != right)
            .unwrap_or(expected.len().min(observed.len()));
        problems.push(format!(
            "{name}: schema differs from {name}.json at line {}\n  snapshot: {}\n  actual:   {}",
            line + 1,
            expected.get(line).unwrap_or(&"<end of file>"),
            observed.get(line).unwrap_or(&"<end of file>"),
        ));
    }
    assert!(
        problems.is_empty(),
        "mutating tool contract differs from tests/fixtures/mutation-schemas:\n{}\n\
         If the change is intended, run `{UPDATE}=1 cargo test -p unica-application --lib \
         native_mutation_surface_has_exact_operations_and_schemas` and review the snapshot diff.",
        problems.join("\n")
    );
}
