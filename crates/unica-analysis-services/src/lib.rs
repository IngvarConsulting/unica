//! Provider-neutral authenticated workspace service lifecycle.
pub mod backend;
pub mod protocol;
pub mod provider_host;
mod service;
pub use service::{
    checked_persistent_provider_working_directory, persistent_provider_working_directory,
    run_workspace_service_from_args, WorkspaceServiceBslCall, WorkspaceServiceConfig,
    WorkspaceServiceIdentity, WorkspaceServiceManager, WorkspaceServiceRecord, WorkspaceServices,
    ANALYZER_BUSY,
};
#[cfg(any(test, feature = "test-support"))]
pub mod test_support {
    pub use crate::protocol_test_support::error_response;
    pub use crate::service::{
        acquire_record_lock, acquire_spawn_lock, cancellation_error, duration_from_timeout_parts,
        duration_timeout_millis, env_u64, flush_with_deadline, handle_workspace_service_request,
        handle_workspace_service_stream, invalidation_dispatch_hooks, is_retry_safe_bsl_mcp_tool,
        is_retryable_workspace_service_transport_error, new_token, now_secs,
        read_bounded_service_line, read_bounded_service_line_with_deadline, read_record,
        read_record_unlocked, read_service_response, read_workspace_service_stderr_tail,
        remaining_or_control_timeout, remaining_or_timeout, remove_service_record_if_owned,
        remove_service_record_if_owned_with_hook, report_workspace_service_handler_result_to,
        response_line_too_large, serve_workspace_service, service_cache_root, service_key,
        service_response_is_alive, spawn_lock_is_contended, spawn_lock_path,
        terminate_failed_workspace_service_spawn, update_service_record_last_access,
        update_service_record_last_access_with_hook, wait_for_record_while,
        wait_for_record_with_connector, with_record_lock, workspace_service_command,
        workspace_service_handler_diagnostic, workspace_service_request_timeout_error,
        workspace_service_startup_failure, write_control_with_deadline, write_record,
        write_record_unlocked, write_record_with_hook, write_service_response, write_with_deadline,
        AnalyzerLane, AnalyzerLanePermit, BoundedServiceLine, ConnectorClock, ConnectorIo,
        ConnectorRequestDeadline, ConnectorStream, Deadline, InvalidationDispatchHook,
        ServiceConnector, ServiceElapsedBudget, ServiceSpawner, SessionTeardownLifecycle,
        SystemClock, SystemConnectorIo, SystemServiceConnector,
        SystemWorkspaceServiceOperationExecutor, WorkspaceCallerMonitorPoint,
        WorkspaceServiceCallDeadline, WorkspaceServiceOperationExecutor, WorkspaceServicesState,
        SYSTEM_SERVICE_CONNECTOR,
    };
    pub use crate::service::{
        DEFAULT_IDLE_SECS, LEGACY_WORKSPACE_SERVICE_PROVIDER_PROFILE,
        LEGACY_WORKSPACE_SERVICE_SOURCE_SET, SERVICE_CONNECT_TIMEOUT,
        SERVICE_CONTROL_CONNECT_TIMEOUT, SERVICE_RECORD_LOCK_FILE, SERVICE_REQUEST_HEADER_TIMEOUT,
        SERVICE_REQUEST_LINE_LIMIT, SERVICE_REQUEST_TIMEOUT, SERVICE_RESPONSE_LINE_LIMIT,
        SERVICE_SCHEMA_VERSION, SERVICE_SHUTDOWN_GRACE, SERVICE_SPAWN_CLEANUP_WAIT,
        SERVICE_STARTUP_STDERR_TAIL_LIMIT, SESSION_TEARDOWN_GRACE,
    };
}

#[cfg(any(test, feature = "test-support"))]
pub mod protocol_test_support {
    pub fn error_response(message: impl Into<String>) -> crate::protocol::ServiceResponse {
        crate::protocol::ServiceResponse::error(message)
    }
}
