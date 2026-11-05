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
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        yields: bool,
    },
    /// Reserves an operation before the client may submit its Start frame.
    BslMcpBegin {
        operation_id: String,
        call: PreparedBslCall,
        deadline: BslOperationDeadline,
    },
    BslMcpStart {
        operation_id: String,
        deadline: BslOperationDeadline,
    },
    BslMcpStop {
        operation_id: String,
        cause: BslStopCause,
    },
    Expire {
        operation_id: String,
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

/// The new request shape is closed independently of the legacy wire.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedBslCall {
    pub tool_name: String,
    pub tool_args: Value,
    pub yields: bool,
}

/// Absence is explicit; a finite zero remainder is an expired deadline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum BslOperationDeadline {
    Absent {},
    Finite { seconds: u64, nanos: u32 },
}

impl BslOperationDeadline {
    pub fn from_remaining(remaining: Option<std::time::Duration>) -> Self {
        match remaining {
            None => Self::Absent {},
            Some(remaining) => Self::Finite {
                seconds: remaining.as_secs(),
                nanos: remaining.subsec_nanos(),
            },
        }
    }

    pub fn remaining(&self) -> Result<Option<std::time::Duration>, String> {
        match *self {
            Self::Absent {} => Ok(None),
            Self::Finite { seconds, nanos } if nanos < 1_000_000_000 => {
                Ok(Some(std::time::Duration::new(seconds, nanos)))
            }
            Self::Finite { .. } => Err("invalid workspace service operation deadline".into()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BslStopCause {
    Cancel,
    Expire,
}

/// Admission is not the result of the requested provider operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "phase", rename_all = "snake_case", deny_unknown_fields)]
pub enum PreparedBslReply {
    Admitted { operation_id: String },
    Finished { response: Box<ServiceResponse> },
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
            | Self::BslMcpBegin { operation_id, .. }
            | Self::RlmReady { operation_id, .. }
            | Self::RlmMcp { operation_id, .. } => Some(operation_id),
            _ => None,
        }
    }

    pub fn is_control(&self) -> bool {
        matches!(
            self,
            Self::Ping
                | Self::Invalidate { .. }
                | Self::Shutdown
                | Self::Cancel { .. }
                | Self::Expire { .. }
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

    #[test]
    fn prepared_deadline_wire_distinguishes_absence_expiry_and_rejects_unknown_modes() {
        assert_eq!(
            serde_json::from_str::<BslOperationDeadline>(r#"{"mode":"absent"}"#)
                .unwrap()
                .remaining()
                .unwrap(),
            None
        );
        assert_eq!(
            serde_json::from_str::<BslOperationDeadline>(
                r#"{"mode":"finite","seconds":0,"nanos":0}"#
            )
            .unwrap()
            .remaining()
            .unwrap(),
            Some(std::time::Duration::ZERO)
        );
        for invalid in [
            "null",
            "{}",
            r#"{"mode":"unknown"}"#,
            r#"{"mode":"absent","seconds":1}"#,
            r#"{"mode":"finite","seconds":1}"#,
            r#"{"mode":"finite","seconds":1,"nanos":0,"extra":1}"#,
        ] {
            assert!(
                serde_json::from_str::<BslOperationDeadline>(invalid).is_err(),
                "invalid deadline accepted: {invalid}"
            );
        }
        let invalid = r#"{"mode":"finite","seconds":1,"nanos":1000000000}"#;
        assert!(serde_json::from_str::<BslOperationDeadline>(invalid)
            .unwrap()
            .remaining()
            .is_err());
    }

    #[test]
    fn prepared_finite_wire_preserves_the_full_duration_domain_without_becoming_absent() {
        for duration in [
            std::time::Duration::ZERO,
            std::time::Duration::from_secs(i64::MAX as u64),
            std::time::Duration::MAX,
        ] {
            let wire = BslOperationDeadline::from_remaining(Some(duration));
            let encoded = serde_json::to_string(&wire).unwrap();
            let decoded: BslOperationDeadline = serde_json::from_str(&encoded).unwrap();
            assert_eq!(decoded.remaining().unwrap(), Some(duration));
            assert_ne!(decoded, BslOperationDeadline::Absent {});
        }
    }

    #[test]
    fn prepared_begin_requires_explicit_deadline_and_admission_is_not_final_success() {
        let begin = r#"{"token":"secret","kind":{"type":"bsl-mcp-begin","operation_id":"read-op","call":{"tool_name":"symbol_info","tool_args":{},"yields":true},"deadline":{"mode":"absent"}}}"#;
        let request: ServiceRequest = serde_json::from_str(begin).unwrap();
        assert_eq!(serde_json::to_string(&request).unwrap(), begin);
        assert_eq!(request.kind.operation_id(), Some("read-op"));
        assert!(serde_json::from_str::<ServiceRequest>(
            &begin.replace(r#","deadline":{"mode":"absent"}"#, "")
        )
        .is_err());
        let admitted = r#"{"phase":"admitted","operation_id":"read-op"}"#;
        assert!(matches!(
            serde_json::from_str::<PreparedBslReply>(admitted).unwrap(),
            PreparedBslReply::Admitted { .. }
        ));
        assert!(serde_json::from_str::<PreparedBslReply>(r#"{"ok":true}"#).is_err());
    }

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
            r#"{"token":"secret","kind":{"type":"bsl-mcp","operation_id":"lsp-op","tool_name":"symbol_info","tool_args":{},"timeout_seconds":5,"timeout_nanos":0,"yields":true}}"#,
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
