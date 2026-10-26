//! Existing authenticated workspace-service wire facts.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use unica_source_contract::events::DomainEvent;
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceRequest {
    pub token: String,
    pub kind: ServiceRequestKind,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "type")]
pub enum ServiceRequestKind {
    Ping,
    BslMcp {
        operation_id: String,
        tool_name: String,
        tool_args: Value,
        timeout_seconds: u64,
        timeout_nanos: u32,
        /// The call gives way to one that does not: an editor's call is
        /// cancelled at the analyzer when an MCP call needs it.
        #[serde(default)]
        yields: bool,
    },
    RlmReady {
        operation_id: String,
        args: Value,
        timeout_seconds: u64,
        timeout_nanos: u32,
    },
    RlmMcp {
        operation_id: String,
        operation: WorkspaceRlmOperation,
        timeout_seconds: u64,
        timeout_nanos: u32,
    },
    Cancel {
        operation_id: String,
    },
    Invalidate {
        events: Vec<DomainEvent>,
    },
    Shutdown,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ServiceResponse {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stderr: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index_maintenance: Option<IndexMaintenanceStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub db_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index_freshness: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index_build_id: Option<String>,
    #[serde(default)]
    pub shutdown: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "operation")]
pub enum WorkspaceRlmOperation {
    Search {
        query: String,
        limit: usize,
    },
    Definition {
        name: String,
        module_hint: String,
        limit: usize,
    },
    Outline {
        path: String,
        include_methods: bool,
    },
    ObjectProfile {
        name: String,
        sections: Option<Vec<String>>,
        limit: usize,
    },
}

impl ServiceRequestKind {
    pub fn operation_id(&self) -> Option<&str> {
        match self {
            Self::BslMcp { operation_id, .. }
            | Self::RlmReady { operation_id, .. }
            | Self::RlmMcp { operation_id, .. } => Some(operation_id),
            _ => None,
        }
    }

    pub fn is_control(&self) -> bool {
        matches!(
            self,
            Self::Ping | Self::Invalidate { .. } | Self::Shutdown | Self::Cancel { .. }
        )
    }
}

impl ServiceResponse {
    pub(crate) fn error(message: impl Into<String>) -> Self {
        Self {
            ok: false,
            error: Some(message.into()),
            ..Self::default()
        }
    }
}

/// Observed work, independent of the persisted index's readiness.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", content = "reason", rename_all = "snake_case")]
pub enum IndexMaintenanceStatus {
    #[default]
    Unknown,
    Queued,
    Running,
    Completed,
    StartFailed(String),
    Failed(String),
    Cancelled(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    // Concrete pre-extraction serde schema: field order, omission, the
    // complete maintenance schema, and the default shutdown field all matter.
    const READY_REPLY: &str = "{\"ok\":true,\"result_text\":\"rows\",\"stderr\":\"tail\",\"warnings\":[\"maintenance\"],\"index_status\":\"ready\",\"index_maintenance\":{\"state\":\"failed\",\"reason\":\"interrupted\"},\"db_path\":\"/cache/build/index.db\",\"index_freshness\":\"unknown\",\"index_build_id\":\"0123456789abcdef0123456789abcdef\",\"shutdown\":false}";

    #[test]
    fn full_existing_reply_schema_keeps_exact_bytes_for_every_client() {
        let reply: ServiceResponse = serde_json::from_str(READY_REPLY).unwrap();
        assert_eq!(
            reply.index_maintenance,
            Some(IndexMaintenanceStatus::Failed("interrupted".into()))
        );
        assert_eq!(serde_json::to_string(&reply).unwrap(), READY_REPLY);
        assert_eq!(
            serde_json::to_string(&ServiceResponse::default()).unwrap(),
            "{\"ok\":false,\"shutdown\":false}"
        );
        for maintenance in [
            r#"{"state":"unknown"}"#,
            r#"{"state":"queued"}"#,
            r#"{"state":"running"}"#,
            r#"{"state":"completed"}"#,
            r#"{"state":"start_failed","reason":"x"}"#,
            r#"{"state":"failed","reason":"x"}"#,
            r#"{"state":"cancelled","reason":"x"}"#,
        ] {
            let encoded =
                format!("{{\"ok\":true,\"index_maintenance\":{maintenance},\"shutdown\":false}}");
            let reply: ServiceResponse = serde_json::from_str(&encoded).unwrap();
            assert_eq!(serde_json::to_string(&reply).unwrap(), encoded);
        }
    }

    #[test]
    fn bsl_reply_cannot_silently_accept_invalid_rlm_maintenance_fields() {
        for maintenance in [
            r#"{"state":"future_state"}"#,
            r#"{"state":"failed"}"#,
            r#"{"state":"failed","reason":7}"#,
            r#"{"state":7}"#,
        ] {
            let reply = format!("{{\"ok\":true,\"result_text\":\"bsl result\",\"index_maintenance\":{maintenance}}}");
            assert!(
                serde_json::from_str::<ServiceResponse>(&reply).is_err(),
                "invalid shared reply was accepted: {reply}"
            );
        }
    }

    #[test]
    fn existing_work_request_tags_and_lossless_timeout_parts_keep_their_wire() {
        for encoded in [
            r#"{"token":"secret","kind":{"type":"ping"}}"#,
            r#"{"token":"secret","kind":{"type":"bsl-mcp","operation_id":"bsl-op","tool_name":"search","tool_args":{"action":"search_code"},"timeout_seconds":18446744073709551615,"timeout_nanos":999999999}}"#,
            r#"{"token":"secret","kind":{"type":"rlm-ready","operation_id":"ready-op","args":{},"timeout_seconds":120,"timeout_nanos":5}}"#,
            r#"{"token":"secret","kind":{"type":"rlm-mcp","operation_id":"read-op","operation":{"operation":"definition","name":"Symbol","module_hint":"CommonModules/Shared/Ext/Module.bsl","limit":9},"timeout_seconds":3600,"timeout_nanos":7}}"#,
            r#"{"token":"secret","kind":{"type":"cancel","operation_id":"read-op"}}"#,
            r#"{"token":"secret","kind":{"type":"invalidate","events":[]}}"#,
            r#"{"token":"secret","kind":{"type":"shutdown"}}"#,
        ] {
            let request: ServiceRequest = serde_json::from_str(encoded).unwrap();
            assert_eq!(serde_json::to_string(&request).unwrap(), encoded);
        }
    }
}
