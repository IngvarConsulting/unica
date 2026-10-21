use crate::environment::RlmEnvironment;
#[cfg(any(test, feature = "test-support"))]
use crate::index::rlm_generation_root;
use crate::index::{rlm_process_environment, IndexMaintenanceStatusExt, IndexReadiness};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;
use unica_analysis_services::persistent_provider_working_directory;
use unica_analysis_services::protocol::{
    IndexMaintenanceStatus, ServiceResponse, WorkspaceRlmOperation,
};
use unica_process::mcp_session::{McpToolOutput, PersistentMcpSession};
use unica_source_contract::workspace::WorkspaceContext;
use unica_work_control::cancellation::{cancelled_error, CancellationToken};
use unica_work_control::elapsed_budget::ElapsedBudgetDeadline;
const RLM_LOGICAL_SESSION_RETRY_LIMIT: usize = 2;
pub struct RlmMcpSession {
    transport: PersistentMcpSession,
    source_root: PathBuf,
    build_id: String,
    session_id: Option<String>,
}

#[derive(Debug, Clone)]
pub enum WorkspaceServiceRlmCall {
    Output(WorkspaceServiceRlmOutput),
    Unready(IndexReadiness),
}

#[derive(Debug, Clone)]
pub struct WorkspaceServiceRlmOutput {
    pub result_text: String,
    pub stderr: String,
    pub freshness: Option<String>,
    pub build_id: Option<String>,
    pub maintenance: Option<IndexMaintenanceStatus>,
}

impl RlmMcpSession {
    pub fn start_pinned(
        context: &WorkspaceContext,
        source_root: &Path,
        build_directory: &Path,
        build_id: &str,
        cancellation: &CancellationToken,
        environment: &dyn RlmEnvironment,
    ) -> Result<Self, String> {
        Ok(Self {
            transport: start_rlm_transport_pinned(
                context,
                source_root,
                build_directory,
                cancellation,
                environment,
            )?,
            source_root: source_root.to_path_buf(),
            build_id: build_id.to_string(),
            session_id: None,
        })
    }

    fn is_reusable(&mut self) -> bool {
        self.transport.is_reusable()
    }

    pub fn execute(
        &mut self,
        operation: &WorkspaceRlmOperation,
        timeout: Duration,
        cancellation: &CancellationToken,
    ) -> Result<WorkspaceServiceRlmOutput, String> {
        let deadline = ElapsedBudgetDeadline::new(timeout);
        for attempt in 0..=RLM_LOGICAL_SESSION_RETRY_LIMIT {
            match self.ensure_logical_session(operation.query_hint(), &deadline, cancellation) {
                Ok(()) => {}
                Err(error)
                    if attempt < RLM_LOGICAL_SESSION_RETRY_LIMIT
                        && is_retryable_rlm_session_error(&error) =>
                {
                    self.close_logical_session(&deadline, cancellation);
                    continue;
                }
                Err(error) => return Err(error),
            }
            match self.execute_once(operation, &deadline, cancellation) {
                Ok(output) => return Ok(output),
                Err(error)
                    if attempt < RLM_LOGICAL_SESSION_RETRY_LIMIT
                        && is_retryable_rlm_session_error(&error) =>
                {
                    self.close_logical_session(&deadline, cancellation);
                }
                Err(error) => {
                    self.close_logical_session(&deadline, cancellation);
                    return Err(error);
                }
            }
        }
        Err("persistent RLM session retry exhausted".to_string())
    }

    fn ensure_logical_session(
        &mut self,
        query_hint: &str,
        deadline: &ElapsedBudgetDeadline,
        cancellation: &CancellationToken,
    ) -> Result<(), String> {
        if self.session_id.is_some() {
            return Ok(());
        }
        let timeout = remaining_rlm_time(deadline, cancellation)?;
        let output = self.transport.call(
            "rlm_start",
            rlm_start_arguments(&self.source_root, query_hint, timeout)?,
            timeout,
            cancellation,
        )?;
        let envelope: Value = serde_json::from_str(output.result_text.trim())
            .map_err(|error| format!("invalid rlm_start JSON response: {error}"))?;
        if let Some(error) = envelope.get("error").and_then(Value::as_str) {
            return Err(format!("rlm_start failed: {error}"));
        }
        let session_id = envelope
            .get("session_id")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "rlm_start response is missing session_id".to_string())?
            .to_string();
        self.session_id = Some(session_id);
        let index_status = envelope
            .pointer("/index/index_status")
            .and_then(Value::as_str);
        if matches!(index_status, Some("missing" | "incomplete")) {
            let error = format!(
                "rlm_start reported unavailable index status {}",
                index_status.unwrap_or("unknown")
            );
            self.close_logical_session(deadline, cancellation);
            return Err(error);
        }
        Ok(())
    }

    fn execute_once(
        &mut self,
        operation: &WorkspaceRlmOperation,
        deadline: &ElapsedBudgetDeadline,
        cancellation: &CancellationToken,
    ) -> Result<WorkspaceServiceRlmOutput, String> {
        let session_id = self
            .session_id
            .as_deref()
            .ok_or_else(|| "persistent RLM logical session is missing".to_string())?;
        let code = fixed_rlm_helper_code(operation)?;
        let timeout = remaining_rlm_time(deadline, cancellation)?;
        let output = self.transport.call(
            "rlm_execute",
            json!({
                "session_id": session_id,
                "code": code,
                "detail_level": "compact"
            }),
            timeout,
            cancellation,
        )?;
        let envelope: Value = serde_json::from_str(output.result_text.trim())
            .map_err(|error| format!("invalid rlm_execute JSON response: {error}"))?;
        if let Some(error) = envelope
            .get("error")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
        {
            return Err(format!("rlm_execute failed: {error}"));
        }
        let stdout = envelope
            .get("stdout")
            .and_then(Value::as_str)
            .ok_or_else(|| "rlm_execute response is missing stdout".to_string())?
            .trim();
        serde_json::from_str::<Value>(stdout)
            .map_err(|error| format!("rlm_execute stdout is not helper JSON: {error}"))?;
        Ok(WorkspaceServiceRlmOutput {
            result_text: stdout.to_string(),
            stderr: output.stderr,
            freshness: None,
            build_id: Some(self.build_id.clone()),
            maintenance: None,
        })
    }

    fn close_logical_session(
        &mut self,
        deadline: &ElapsedBudgetDeadline,
        cancellation: &CancellationToken,
    ) {
        let Some(session_id) = self.session_id.take() else {
            return;
        };
        let Ok(timeout) = remaining_rlm_time(deadline, cancellation) else {
            self.transport.invalidate();
            return;
        };
        let _ = self.transport.call(
            "rlm_end",
            json!({"session_id": session_id}),
            timeout.min(Duration::from_secs(2)),
            cancellation,
        );
    }
}

impl Drop for RlmMcpSession {
    fn drop(&mut self) {
        let deadline = ElapsedBudgetDeadline::new(Duration::from_secs(2));
        self.close_logical_session(&deadline, &CancellationToken::new());
    }
}

pub trait RlmOperationHint {
    fn query_hint(&self) -> &str;
}
impl RlmOperationHint for WorkspaceRlmOperation {
    fn query_hint(&self) -> &str {
        match self {
            Self::Search { query, .. } => query,
            Self::Definition { name, .. } | Self::ObjectProfile { name, .. } => name,
            Self::Outline { path, .. } => path,
        }
    }
}

pub fn duration_seconds_ceil(duration: Duration) -> Result<u64, String> {
    duration
        .as_secs()
        .checked_add(u64::from(duration.subsec_nanos() != 0))
        .ok_or_else(|| {
            "workspace service timeout cannot be rounded up to whole seconds".to_string()
        })
}

pub fn rlm_start_arguments(
    source_root: &Path,
    query: &str,
    timeout: Duration,
) -> Result<Value, String> {
    let execution_timeout_seconds = duration_seconds_ceil(timeout)?;
    Ok(json!({
        "path": source_root.display().to_string(),
        "query": query,
        "effort": "low",
        "max_output_chars": 100_000,
        "max_execute_calls": 10_000,
        "execution_timeout_seconds": execution_timeout_seconds,
        "include_metadata": false,
        // RLM 1.41+ refuses a slim BSL session without an explicit helper
        // domain choice. Unica calls helpers by name and needs only the core
        // signatures; the empty choice keeps the start response small.
        "domains": []
    }))
}

pub fn fixed_rlm_helper_code(operation: &WorkspaceRlmOperation) -> Result<String, String> {
    let serialized = serde_json::to_string(operation)
        .map_err(|error| format!("failed to encode RLM operation: {error}"))?;
    let data_literal = serde_json::to_string(&serialized)
        .map_err(|error| format!("failed to encode RLM operation literal: {error}"))?;
    let body = match operation {
        WorkspaceRlmOperation::Search { .. } => {
            "_result = search(_args[\"query\"], scope=\"all\", limit=_args[\"limit\"])\nprint(json.dumps(_result, ensure_ascii=False))"
        }
        WorkspaceRlmOperation::Definition { .. } => {
            "_result = find_definition(_args[\"name\"], module_hint=_args[\"module_hint\"], limit=_args[\"limit\"])\nprint(json.dumps(_result, ensure_ascii=False))"
        }
        WorkspaceRlmOperation::Outline { .. } => {
            "_result = get_module_outline(_args[\"path\"], include_methods=_args[\"include_methods\"])\nprint(json.dumps(_result, ensure_ascii=False))"
        }
        // `find_predefined` is the only supported way to reach predefined items and
        // it has neither a count nor a category argument. When the cross-category
        // fetch is capped, post-filtering cannot establish a complete answer for
        // the requested category, so the section fails closed instead of claiming
        // an authoritative `ok` or `empty`.
        WorkspaceRlmOperation::ObjectProfile { .. } => {
            "_raw_sections = _args[\"sections\"]\n_section_aliases = {\"functionalOptions\": \"functional_options\"}\n_profile_sections = None if _raw_sections is None else [_section_aliases.get(_section, _section) for _section in _raw_sections if _section != \"predefinedItems\"]\n_result = get_object_profile(_args[\"name\"], sections=_profile_sections, include_flow=False, include_code_usages=False, limit=_args[\"limit\"])\nif _raw_sections is not None and \"predefinedItems\" in _raw_sections and \"error\" not in _result:\n    _category = _result.get(\"category\")\n    _fetched = find_predefined(object_name=_result.get(\"object_name\") or _args[\"name\"], limit=_args[\"limit\"] + 1)\n    _has_more = len(_fetched) > _args[\"limit\"]\n    _items = _fetched\n    if _category:\n        _items = [_item for _item in _items if not _item.get(\"category\") or _item.get(\"category\") == _category]\n    _items = _items[:_args[\"limit\"]]\n    if _category and _has_more:\n        _predefined = {\"status\": \"unavailable\", \"summary\": {}, \"items\": [], \"total\": 0, \"returned\": 0, \"has_more\": True, \"_meta\": {\"source\": \"index\", \"truncated\": True, \"error\": \"supported RLM API cannot establish category-complete predefined items after a capped cross-category fetch\"}}\n    else:\n        _predefined = {\"status\": \"ok\" if _items else \"empty\", \"summary\": {}, \"items\": _items, \"total\": len(_items), \"returned\": len(_items), \"has_more\": _has_more, \"_meta\": {\"source\": \"index\", \"truncated\": _has_more, \"total_is_lower_bound\": _has_more}}\n    _result.setdefault(\"sections\", {})[\"predefined_items\"] = _predefined\nprint(json.dumps(_result, ensure_ascii=False))"
        }
    };
    Ok(format!(
        "import json\n_args = json.loads({data_literal})\n{body}"
    ))
}

pub fn remaining_rlm_time(
    deadline: &ElapsedBudgetDeadline,
    cancellation: &CancellationToken,
) -> Result<Duration, String> {
    if cancellation.is_cancelled() {
        return Err(cancelled_error("persistent RLM request stopped"));
    }
    deadline
        .remaining()
        .ok_or_else(|| "timeout: persistent RLM request timed out".to_string())
}

pub fn is_retryable_rlm_session_error(error: &str) -> bool {
    [
        "not found or expired",
        "Sandbox not found",
        "was closed",
        "Execution call limit exceeded",
    ]
    .iter()
    .any(|marker| error.contains(marker))
}

#[cfg(any(test, feature = "test-support"))]
pub fn rlm_transport_command(
    program: PathBuf,
    context: &WorkspaceContext,
    source_root: &Path,
) -> Result<Command, String> {
    rlm_transport_command_for_build(
        program,
        context,
        source_root,
        &rlm_generation_root(context, source_root)?,
    )
}

pub fn rlm_transport_command_for_build(
    program: PathBuf,
    context: &WorkspaceContext,
    source_root: &Path,
    build_directory: &Path,
) -> Result<Command, String> {
    let mut command = Command::new(program);
    command
        .current_dir(persistent_provider_working_directory(
            &context.workspace_root,
            source_root,
        )?)
        .envs(rlm_process_environment(build_directory.to_path_buf()));
    Ok(command)
}

#[cfg(any(test, feature = "test-support"))]
pub fn start_rlm_transport(
    context: &WorkspaceContext,
    source_root: &Path,
    cancellation: &CancellationToken,
    environment: &dyn RlmEnvironment,
) -> Result<PersistentMcpSession, String> {
    let directory = rlm_generation_root(context, source_root)?;
    start_rlm_transport_pinned(context, source_root, &directory, cancellation, environment)
}

pub fn start_rlm_transport_pinned(
    context: &WorkspaceContext,
    source_root: &Path,
    build_directory: &Path,
    cancellation: &CancellationToken,
    environment: &dyn RlmEnvironment,
) -> Result<PersistentMcpSession, String> {
    if cancellation.is_cancelled() {
        return Err(cancelled_error("persistent RLM MCP start stopped"));
    }
    let plugin_root = environment.find_plugin_root(&context.cwd).ok_or_else(|| {
        "could not locate Unica plugin root for workspace RLM service".to_string()
    })?;
    let program = environment
        .resolve_bundled_tool(&plugin_root, "rlm-bsl-mcp", true)?
        .program;
    let command = rlm_transport_command_for_build(program, context, source_root, build_directory)?;
    PersistentMcpSession::start_with_command(command, cancellation)
}

pub trait RlmServiceResponse {
    fn from_readiness(readiness: IndexReadiness, warnings: Vec<String>) -> Self;
    fn unavailable_rlm_execution(readiness: IndexReadiness, warnings: Vec<String>) -> Self;
    fn into_rlm_call(self) -> Result<WorkspaceServiceRlmCall, String>;
    fn index_readiness(&self) -> IndexReadiness;
}
impl RlmServiceResponse for ServiceResponse {
    fn from_readiness(readiness: IndexReadiness, warnings: Vec<String>) -> Self {
        match readiness {
            IndexReadiness::Ready { db_path } => Self {
                ok: true,
                index_status: Some("ready".to_string()),
                db_path: Some(db_path.display().to_string()),
                warnings,
                ..Self::default()
            },
            IndexReadiness::Missing => Self {
                ok: true,
                index_status: Some("missing".to_string()),
                warnings,
                ..Self::default()
            },
            IndexReadiness::Stale { status } => Self {
                ok: true,
                index_status: Some("stale".to_string()),
                error: Some(status),
                warnings,
                ..Self::default()
            },
            IndexReadiness::Building => Self {
                ok: true,
                index_status: Some("building".to_string()),
                warnings,
                ..Self::default()
            },
            IndexReadiness::Queued => Self {
                ok: true,
                index_status: Some("queued".to_string()),
                warnings,
                ..Self::default()
            },
            IndexReadiness::Incomplete => Self {
                ok: true,
                index_status: Some("incomplete".to_string()),
                warnings,
                ..Self::default()
            },
            IndexReadiness::MaintenanceUnavailable(evidence) => Self {
                ok: true,
                index_status: Some("unavailable".to_string()),
                error: Some(evidence.summary().to_string()),
                warnings,
                ..Self::default()
            },
            IndexReadiness::Failed(error) => Self {
                ok: true,
                index_status: Some("failed".to_string()),
                error: Some(error),
                warnings,
                ..Self::default()
            },
            IndexReadiness::Unavailable(error) => Self {
                ok: true,
                index_status: Some("unavailable".to_string()),
                error: Some(error),
                warnings,
                ..Self::default()
            },
        }
    }
    fn unavailable_rlm_execution(readiness: IndexReadiness, warnings: Vec<String>) -> Self {
        let mut response = Self::from_readiness(readiness, warnings);
        response.ok = false;
        if response.error.is_none() {
            response.error = Some(match response.index_status.as_deref() {
                Some("missing") => "rlm index is missing".to_string(),
                Some("building") => "rlm index building".to_string(),
                Some("queued") => "rlm index maintenance queued".to_string(),
                Some("incomplete") => {
                    "rlm index is incomplete; maintenance state is unknown".to_string()
                }
                _ => "rlm index unavailable at execution boundary".to_string(),
            });
        }
        response.result_text = None;
        response.stderr = None;
        response
    }
    fn into_rlm_call(self) -> Result<WorkspaceServiceRlmCall, String> {
        let response = self;
        if !response.ok {
            if response.index_status.is_some() {
                return Ok(WorkspaceServiceRlmCall::Unready(response.index_readiness()));
            }
            return Err(response
                .error
                .unwrap_or_else(|| "workspace service RLM request failed".to_string()));
        }
        Ok(WorkspaceServiceRlmCall::Output(WorkspaceServiceRlmOutput {
            result_text: response.result_text.unwrap_or_default(),
            stderr: response.stderr.unwrap_or_default(),
            freshness: response.index_freshness,
            build_id: response.index_build_id,
            maintenance: response.index_maintenance,
        }))
    }
    fn index_readiness(&self) -> IndexReadiness {
        if self.index_status.as_deref() != Some("ready") {
            if let Some(maintenance) = &self.index_maintenance {
                return maintenance.readiness();
            }
        }
        match self.index_status.as_deref() {
            Some("ready") => self
                .db_path
                .as_ref()
                .map(|path| IndexReadiness::Ready {
                    db_path: PathBuf::from(path),
                })
                .unwrap_or_else(|| {
                    IndexReadiness::Unavailable(
                        "workspace service reported ready without db path".to_string(),
                    )
                }),
            Some("missing") => IndexReadiness::Missing,
            Some("stale") => IndexReadiness::Stale {
                status: self.error.clone().unwrap_or_else(|| "stale".to_string()),
            },
            Some("building") => IndexReadiness::Building,
            Some("queued") => IndexReadiness::Queued,
            Some("incomplete") => IndexReadiness::Incomplete,
            Some("failed") => IndexReadiness::Failed(self.error.clone().unwrap_or_default()),
            Some("unavailable") => {
                IndexReadiness::Unavailable(self.error.clone().unwrap_or_default())
            }
            other => IndexReadiness::Unavailable(format!(
                "workspace service reported unknown RLM status {:?}",
                other
            )),
        }
    }
}

impl unica_analysis_services::backend::RlmSession for RlmMcpSession {
    fn is_reusable(&mut self) -> bool {
        RlmMcpSession::is_reusable(self)
    }
    fn build_id(&self) -> &str {
        &self.build_id
    }
    fn execute(
        &mut self,
        operation: &WorkspaceRlmOperation,
        timeout: Duration,
        cancellation: &CancellationToken,
    ) -> Result<McpToolOutput, String> {
        let result = RlmMcpSession::execute(self, operation, timeout, cancellation)?;
        Ok(McpToolOutput {
            result_text: result.result_text,
            stderr: result.stderr,
        })
    }
}

#[cfg(any(test, feature = "test-support"))]
impl RlmMcpSession {
    pub fn test_from_transport(
        transport: PersistentMcpSession,
        source_root: PathBuf,
        build_id: String,
        session_id: Option<String>,
    ) -> Self {
        Self {
            transport,
            source_root,
            build_id,
            session_id,
        }
    }
    pub fn test_reader_terminal(&self) -> std::sync::Arc<std::sync::atomic::AtomicBool> {
        self.transport.test_reader_terminal()
    }
}
