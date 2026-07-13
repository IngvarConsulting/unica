use crate::application::{input_schema_for_tool, ToolSpec, UnicaApplication};
use serde_json::{json, Value};
use std::io::{self, BufRead, Write};

const PROTOCOL_VERSION: &str = "2024-11-05";

pub fn run_stdio() {
    let app = UnicaApplication::new();
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    for line in stdin.lock().lines() {
        let line = match line {
            Ok(line) if !line.trim().is_empty() => line,
            Ok(_) => continue,
            Err(err) => {
                let _ = writeln!(io::stderr(), "failed to read stdin: {err}");
                break;
            }
        };

        let response = match serde_json::from_str::<Value>(&line) {
            Ok(message) => handle_message(&app, message),
            Err(err) => Some(error_response(
                Value::Null,
                -32700,
                &format!("parse error: {err}"),
            )),
        };

        if let Some(response) = response {
            if writeln!(stdout, "{}", response).is_err() {
                break;
            }
            let _ = stdout.flush();
        }
    }
}

pub fn handle_message(app: &UnicaApplication, message: Value) -> Option<Value> {
    let id = message.get("id").cloned().unwrap_or(Value::Null);
    let method = message.get("method").and_then(Value::as_str).unwrap_or("");

    if method.starts_with("notifications/") {
        return None;
    }

    match method {
        "initialize" => Some(success_response(
            id,
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {
                    "tools": { "listChanged": false }
                },
                "serverInfo": {
                    "name": "unica",
                    "version": env!("CARGO_PKG_VERSION")
                }
            }),
        )),
        "ping" => Some(success_response(id, json!({}))),
        "tools/list" => Some(success_response(
            id,
            json!({ "tools": list_tools(app.tools()) }),
        )),
        "tools/call" => Some(match call_tool_from_message(app, &message) {
            Ok(result) => success_response(
                id,
                json!({ "content": [{ "type": "text", "text": result }] }),
            ),
            Err((code, msg)) => error_response(id, code, &msg),
        }),
        _ => Some(error_response(
            id,
            -32601,
            &format!("method not found: {method}"),
        )),
    }
}

fn list_tools(tools: Vec<ToolSpec>) -> Vec<Value> {
    tools
        .iter()
        .map(|tool| {
            json!({
                "name": tool.name,
                "description": tool.description,
                "inputSchema": input_schema_for_tool(tool)
            })
        })
        .collect()
}

fn call_tool_from_message(
    app: &UnicaApplication,
    message: &Value,
) -> Result<String, (i64, String)> {
    let params = message
        .get("params")
        .ok_or((-32602, "missing params".to_string()))?;
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .ok_or((-32602, "missing tool name".to_string()))?;
    let args = params
        .get("arguments")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();

    let result = app.call_tool(name, &args).map_err(|msg| (-32000, msg))?;
    serde_json::to_string_pretty(&result).map_err(|err| (-32603, err.to_string()))
}

fn success_response(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn error_response(id: Value, code: i64, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": code,
            "message": message
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initialize_uses_single_public_server_name() {
        let app = UnicaApplication::new();
        let request = json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize" });
        let response = handle_message(&app, request).unwrap();
        assert_eq!(response["result"]["serverInfo"]["name"], "unica");
    }

    #[test]
    fn tools_list_contains_orchestrated_tool_names() {
        let app = UnicaApplication::new();
        let request = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" });
        let response = handle_message(&app, request).unwrap();
        let listed = response["result"]["tools"].as_array().unwrap();
        assert_eq!(listed[0]["name"], "unica.cf.edit");
        assert!(listed
            .iter()
            .any(|tool| tool["name"] == "unica.project.status"));
        assert!(listed
            .iter()
            .any(|tool| tool["name"] == "unica.project.map"));
        assert!(listed
            .iter()
            .any(|tool| tool["name"] == "unica.project.discover"));
        assert!(listed
            .iter()
            .any(|tool| tool["name"] == "unica.standards.explain"));
    }

    #[test]
    fn native_tool_schema_is_contract_specific_and_does_not_expose_raw_args() {
        let app = UnicaApplication::new();
        let request = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" });
        let response = handle_message(&app, request).unwrap();
        let listed = response["result"]["tools"].as_array().unwrap();
        let cf_info = listed
            .iter()
            .find(|tool| tool["name"] == "unica.cf.info")
            .expect("unica.cf.info must be listed");

        let schema = &cf_info["inputSchema"];
        assert_eq!(schema["additionalProperties"], false);
        assert!(schema["properties"].get("ConfigPath").is_some());
        assert!(schema["properties"].get("cwd").is_some());
        assert!(schema["properties"].get("dryRun").is_some());
        assert!(schema["properties"].get("args").is_none());
    }

    #[test]
    fn no_public_tool_schema_exposes_raw_adapter_args() {
        let app = UnicaApplication::new();
        let request = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" });
        let response = handle_message(&app, request).unwrap();
        let listed = response["result"]["tools"].as_array().unwrap();

        for tool in listed {
            let properties = &tool["inputSchema"]["properties"];
            assert!(
                properties.get("args").is_none(),
                "{} must not expose raw adapter args",
                tool["name"]
            );
        }
    }
}
