//! Existing generic JSON scalar reads; accepted aliases are supplied by each caller.
use serde_json::{Map, Value};

pub fn bool_arg(args: &Map<String, Value>, names: &[&str]) -> bool {
    names
        .iter()
        .any(|name| args.get(*name).and_then(Value::as_bool).unwrap_or(false))
}

pub fn int_arg(args: &Map<String, Value>, names: &[&str]) -> Option<i64> {
    names
        .iter()
        .find_map(|name| args.get(*name).and_then(json_i64_value))
}

pub fn json_i64_value(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_str().and_then(|text| text.parse::<i64>().ok()))
}

pub fn required_string<'a>(
    args: &'a Map<String, Value>,
    names: &[&str],
    label: &str,
) -> Result<&'a str, String> {
    string_arg(args, names).ok_or_else(|| format!("missing required {label} argument"))
}
pub fn string_arg<'a>(args: &'a Map<String, Value>, names: &[&str]) -> Option<&'a str> {
    names
        .iter()
        .find_map(|name| args.get(*name).and_then(Value::as_str))
        .filter(|value| !value.trim().is_empty())
}
pub fn optional_bool_arg(args: &Map<String, Value>, names: &[&str]) -> Option<bool> {
    names
        .iter()
        .find_map(|name| args.get(*name).and_then(Value::as_bool))
}
pub fn json_string_field(value: &Value, field: &str) -> Option<String> {
    value.get(field).map(json_value_to_python_string)
}
pub fn json_value_to_python_string(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        Value::Bool(value) => {
            if *value {
                "True".to_string()
            } else {
                "False".to_string()
            }
        }
        Value::Number(value) => value.to_string(),
        Value::Null => "None".to_string(),
        other => other.to_string(),
    }
}

use std::path::{Path, PathBuf};
pub fn required_path(
    args: &Map<String, Value>,
    names: &[&str],
    label: &str,
) -> Result<PathBuf, String> {
    path_arg(args, names).ok_or_else(|| format!("missing required {label} argument"))
}
pub fn path_arg(args: &Map<String, Value>, names: &[&str]) -> Option<PathBuf> {
    names
        .iter()
        .find_map(|name| args.get(*name).and_then(Value::as_str))
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from)
}
pub fn absolutize(path: PathBuf, cwd: &Path) -> PathBuf {
    if path.is_absolute() {
        path
    } else {
        cwd.join(path)
    }
}
