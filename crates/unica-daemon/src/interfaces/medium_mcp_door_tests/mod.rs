//! Tests of the MCP door `unica-mcp` against the live daemon: the door
//! routes calls over the daemon protocol to a protocol-v5 runtime of this
//! crate with an injected canonical service.
#![cfg(test)]

mod daemon_router;
mod live_daemon;
mod mcp;

#[allow(unused_imports)]
mod prelude {
    pub(super) use super::live_daemon::*;
    pub(super) use serde_json::{Map, Value};
    pub(super) use std::collections::HashSet;
    pub(super) use std::sync::{Arc, Condvar, Mutex};
    pub(super) use std::time::{Duration, Instant};
    pub(super) use std::time::{SystemTime, UNIX_EPOCH};
    pub(super) use unica_coder::application;
    pub(super) use unica_daemon_protocol::client_v5::V5DaemonProcessOwner;
    pub(super) use unica_daemon_protocol::client_v5::V5TransportError;
    pub(super) use unica_daemon_protocol::client_v5::{
        TaskObservation, TaskObservationStage, TaskObservationTimer, V5DaemonClient,
        V5TaskExchangeError,
    };
    pub(super) use unica_daemon_protocol::protocol_v5::V5DaemonErrorCode;
    pub(super) use unica_daemon_protocol::protocol_v5::{V5InvocationRequest, V5ServerResponse};
    pub(super) use unica_execution::invocation::{
        handoff_budget, normalized_arguments_hash, INVOCATION_HANDOFF_WINDOW,
        RESPONSE_SERIALIZATION_MARGIN, RESPONSE_SERIALIZATION_MARGIN_MS,
    };
    pub(super) use unica_execution::receipt_ledger::V5ToolIdentity;
    pub(super) use unica_execution::receipt_ledger::{
        request_scope_hash, ReceiptKey, RequestIdentity, MAX_ORIGINAL_RESPONSE_BUDGET_MS,
    };
    pub(super) use unica_execution::receipt_runtime::response::V5DaemonTaskSnapshot;
    pub(super) use unica_execution::receipt_runtime::response::{
        V5InvocationPhase, V5InvocationResponse,
    };
    pub(super) use unica_execution_contract::invocation::{InvocationId, TaskId};
    pub(super) use unica_mcp::test_support::rmcp::model::{
        CacheScope, CallToolRequestParams, CallToolResponse, CallToolResult, CancelTaskParams,
        ClientJsonRpcMessage, ClientNotification, ClientRequest, ContentBlock, DiscoverResult,
        ErrorCode, ErrorData, GetMeta, GetTaskParams, GetTaskResult, Implementation,
        InitializeRequestParams, InitializeResult, ListPromptsResult, ListResourceTemplatesResult,
        ListResourcesResult, ListToolsResult, NotificationMetaObject, PaginatedRequestParams,
        ProgressNotificationParam, ProgressToken, ProtocolVersion, RequestMetaObject,
        ServerCapabilities, ServerInfo, ServerJsonRpcMessage, ServerResult, Tool, UpdateTaskParams,
        TASKS_EXTENSION_ID,
    };
    pub(super) use unica_mcp::test_support::rmcp::service::{
        NotificationContext, RequestContext, ServerInitializeError,
    };
    pub(super) use unica_mcp::test_support::rmcp::transport::Transport;
    pub(super) use unica_mcp::test_support::rmcp::{RoleServer, ServerHandler, ServiceExt};
    pub(super) use unica_mcp::test_support::*;
    pub(super) use unica_refusal::RefusalCode;
    pub(super) use unica_refusal::RefusalDetail;
    pub(super) use unica_work_control::cancellation::CancellationToken;
    pub(super) use unica_work_control::progress::{NoopProgressSink, ProgressEvent, ProgressSink};
}
