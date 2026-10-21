//! HTTP and JSON-RPC transport for the standards documentation source.

use serde_json::Value;
use std::time::Duration;

pub struct StandardsAdapter;

pub trait HttpClient {
    fn post_json(&self, endpoint: &str, payload: &Value) -> Result<String, String>;
}

/// Продовый клиент сервера стандартов поверх общего HTTP-клиента процесса.
#[derive(Default)]
pub(crate) struct NetworkHttpClient {
    /// `None` — общий клиент процесса.
    client: Option<std::sync::Arc<unica_network::NetworkClient>>,
}

impl NetworkHttpClient {
    #[cfg(test)]
    pub(crate) fn with_client(client: std::sync::Arc<unica_network::NetworkClient>) -> Self {
        Self {
            client: Some(client),
        }
    }
}

/// Общий продовый HTTP-клиент для потребителей за пределами модуля:
/// поставщик `v8std` реестра документации держит его в `Arc`, а не через
/// статическую ссылку, чтобы тесты подменяли транспорт значением.
pub fn shared_http_client() -> std::sync::Arc<dyn HttpClient + Send + Sync> {
    std::sync::Arc::new(NetworkHttpClient::default())
}

impl StandardsAdapter {
    const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

    pub fn outcome_from_http_body(
        operation: &str,
        endpoint: &str,
        remote_method: &str,
        text: &str,
    ) -> StandardsOutcome {
        let normalized = match normalize_mcp_http_body(text) {
            Ok(text) => text,
            Err(error) => {
                return StandardsOutcome::failure(
                    format!("unica.standards.{operation} received invalid v8std MCP response"),
                    error,
                    endpoint,
                    remote_method,
                )
            }
        };
        match serde_json::from_str::<Value>(&normalized) {
            Ok(Value::Object(object)) if object.contains_key("error") => {
                let message = object
                    .get("error")
                    .and_then(|error| error.get("message"))
                    .and_then(Value::as_str)
                    .unwrap_or("remote JSON-RPC error");
                StandardsOutcome::failure(
                    format!("unica.standards.{operation} failed through internal v8std MCP proxy"),
                    message.to_string(),
                    endpoint,
                    remote_method,
                )
            }
            Ok(Value::Object(mut object)) if object.contains_key("result") => StandardsOutcome {
                ok: true,
                summary: format!(
                    "unica.standards.{operation} completed through internal v8std MCP proxy"
                ),
                errors: Vec::new(),
                artifacts: vec![endpoint.to_string(), remote_method.to_string()],
                data: object.remove("result"),
            },
            Ok(_) => StandardsOutcome::failure(
                format!("unica.standards.{operation} received non-JSON-RPC v8std MCP response"),
                "missing JSON-RPC result or error".to_string(),
                endpoint,
                remote_method,
            ),
            Err(error) => StandardsOutcome::failure(
                format!("unica.standards.{operation} received invalid v8std MCP JSON"),
                error.to_string(),
                endpoint,
                remote_method,
            ),
        }
    }
}

/// Parsed JSON-RPC payload with transport-owned diagnostics and provenance.
/// Application publication outcomes are constructed by their caller.
#[derive(Debug)]
pub struct StandardsOutcome {
    pub ok: bool,
    pub summary: String,
    pub errors: Vec<String>,
    pub artifacts: Vec<String>,
    pub data: Option<Value>,
}

impl StandardsOutcome {
    fn failure(summary: String, error: String, endpoint: &str, remote_method: &str) -> Self {
        Self {
            ok: false,
            summary,
            errors: vec![error],
            artifacts: vec![endpoint.to_string(), remote_method.to_string()],
            data: None,
        }
    }
}

impl HttpClient for NetworkHttpClient {
    fn post_json(&self, endpoint: &str, payload: &Value) -> Result<String, String> {
        let client = match &self.client {
            Some(client) => std::sync::Arc::clone(client),
            None => unica_network::NetworkClient::shared().map_err(|err| err.to_string())?,
        };
        client
            .post(
                endpoint,
                &[
                    ("Content-Type", "application/json"),
                    ("Accept", "application/json, text/event-stream"),
                    ("User-Agent", unica_network::DOCS_USER_AGENT),
                ],
                &payload.to_string(),
                Some(StandardsAdapter::DEFAULT_TIMEOUT),
            )
            .map_err(|err| err.to_string())?
            .into_string()
            .map_err(|err| err.to_string())
    }
}

fn normalize_mcp_http_body(text: &str) -> Result<String, String> {
    let data_lines = text
        .lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    if data_lines.is_empty() {
        return Ok(text.trim().to_string());
    }
    let joined = data_lines.join("\n");
    serde_json::from_str::<Value>(&joined)
        .map_err(|err| format!("invalid JSON-RPC SSE data: {err}"))?;
    Ok(joined)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn standards_mcp_error_body_is_reported_as_failure() {
        let outcome = StandardsAdapter::outcome_from_http_body(
            "explain",
            "https://example.test/mcp",
            "v8std_get_page",
            r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32602,"message":"bad id"}}"#,
        );

        assert!(!outcome.ok);
        assert!(outcome.errors.iter().any(|error| error.contains("bad id")));
        assert!(outcome.data.is_none());
    }

    #[test]
    fn standards_sse_body_extracts_structured_json_result() {
        let outcome = StandardsAdapter::outcome_from_http_body(
            "search",
            "https://example.test/mcp",
            "v8std_search",
            "event: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"ok\":true}}\n\n",
        );

        assert!(outcome.ok);
        // ADR-0023: the JSON-RPC envelope is transport; the tool publishes the
        // result it carried.
        assert_eq!(outcome.data.unwrap(), json!({"ok": true}));
    }

    #[test]
    fn standards_protocol_mismatch_is_failure() {
        let outcome = StandardsAdapter::outcome_from_http_body(
            "search",
            "https://example.test/mcp",
            "v8std_search",
            r#"{"not":"json-rpc"}"#,
        );

        assert!(!outcome.ok);
        assert!(outcome
            .errors
            .iter()
            .any(|error| error.contains("missing JSON-RPC")));
    }
}
