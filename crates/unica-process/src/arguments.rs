//! Scalar/array representation encoding for process argument vectors.
//! Operation admission and backend dispatch belong to their callers.

use serde_json::{Map, Value};
use unica_redaction::{is_secret_key, redactor};

pub fn value_to_cli_string(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Number(number) => number.to_string(),
        Value::Bool(flag) => flag.to_string(),
        other => serde_json::to_string(other).unwrap_or_else(|_| other.to_string()),
    }
}

pub fn append_arg(
    result: &mut Vec<String>,
    flag: &str,
    args: &Map<String, Value>,
    key: &str,
    redact: bool,
) {
    if let Some(value) = string_arg(args, key, redact) {
        result.push(flag.to_string());
        result.push(value);
    }
}

pub fn append_array_args(
    result: &mut Vec<String>,
    flag: &str,
    args: &Map<String, Value>,
    key: &str,
    redact: bool,
) {
    let Some(items) = args.get(key).and_then(Value::as_array) else {
        return;
    };
    for item in items {
        result.push(flag.to_string());
        result.push(reported_cli_value(key, item, redact));
    }
}

pub fn append_bool_flag(
    result: &mut Vec<String>,
    flag: &str,
    args: &Map<String, Value>,
    key: &str,
) {
    if args.get(key).and_then(Value::as_bool).unwrap_or(false) {
        result.push(flag.to_string());
    }
}

pub fn string_arg(args: &Map<String, Value>, key: &str, redact: bool) -> Option<String> {
    args.get(key).and_then(|value| {
        if value.is_null() {
            return None;
        }
        Some(reported_cli_value(key, value, redact))
    })
}

pub fn reported_cli_value(key: &str, value: &Value, redact: bool) -> String {
    if !redact {
        return value_to_cli_string(value);
    }
    if is_secret_key(key) {
        return "<redacted>".to_string();
    }
    redactor(&value_to_cli_string(value))
}

pub fn kebab_case(key: &str) -> String {
    let mut out = String::new();
    for (index, ch) in key.chars().enumerate() {
        if ch == '_' {
            out.push('-');
        } else if ch.is_ascii_uppercase() {
            if index > 0 {
                out.push('-');
            }
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

pub fn process_timeout_error(label: &str, timeout: Option<std::time::Duration>) -> String {
    match timeout {
        Some(timeout) => format!(
            "internal {label} adapter timed out after {} seconds",
            timeout.as_secs()
        ),
        None => format!("internal {label} adapter timed out"),
    }
}

pub fn required_string<'a>(args: &'a Map<String, Value>, key: &str) -> Result<&'a str, String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("missing required `{key}` argument"))
}
