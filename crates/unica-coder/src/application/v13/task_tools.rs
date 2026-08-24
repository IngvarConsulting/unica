use crate::domain::invocation::{DomainResult, InvocationStatus, TaskId};
use serde_json::{json, Map, Value};

pub(crate) const DEFAULT_TASK_RESULT_WAIT_MS: u64 = 7_000;

#[derive(Debug)]
pub(crate) struct CompatibilityToolContract {
    pub(crate) name: &'static str,
    pub(crate) description: &'static str,
    pub(crate) input_schema: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TaskToolAction {
    Get,
    Result { wait_ms: u64 },
    Cancel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TaskToolRequest {
    pub(crate) task_id: TaskId,
    pub(crate) action: TaskToolAction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TaskToolError {
    InvalidTaskId,
    BadWaitMs,
    BadArguments,
    TaskNotFound,
    TaskExpired,
    TaskBackendFailed,
    TaskTransportFailed,
    TaskSessionClosed,
    TaskProtocolFailed,
    ProjectionFailed,
}

impl TaskToolError {
    pub(crate) const fn code(self) -> &'static str {
        match self {
            Self::InvalidTaskId => "invalid_task_id",
            Self::BadWaitMs => "bad_wait_ms",
            Self::BadArguments => "bad_task_arguments",
            Self::TaskNotFound => "task_not_found",
            Self::TaskExpired => "task_expired",
            Self::TaskBackendFailed => "task_backend_failed",
            Self::TaskTransportFailed => "task_transport_failed",
            Self::TaskSessionClosed => "task_session_closed",
            Self::TaskProtocolFailed => "task_protocol_failed",
            Self::ProjectionFailed => "task_projection_failed",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CompatibilityProjection {
    State,
    TerminalResult,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CompatibilityTaskSnapshot {
    pub(crate) task_id: TaskId,
    pub(crate) status: InvocationStatus,
    pub(crate) result: Option<DomainResult>,
    /// Closed presence only: failure code/message remains on the daemon side.
    pub(crate) has_failure: bool,
    pub(crate) created_at_epoch_ms: u64,
    pub(crate) updated_at_epoch_ms: u64,
    pub(crate) ttl_ms: u64,
    pub(crate) poll_interval_ms: u64,
}

impl CompatibilityTaskSnapshot {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        task_id: TaskId,
        status: InvocationStatus,
        result: Option<DomainResult>,
        has_failure: bool,
        created_at_epoch_ms: u64,
        updated_at_epoch_ms: u64,
        ttl_ms: u64,
        poll_interval_ms: u64,
    ) -> Self {
        Self {
            task_id,
            status,
            result,
            has_failure,
            created_at_epoch_ms,
            updated_at_epoch_ms,
            ttl_ms,
            poll_interval_ms,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CompatibilityProjectionError {
    InvalidStatusPayload,
    ReverseTimestampOrder,
}

pub(crate) fn compatibility_tool_contracts() -> Vec<CompatibilityToolContract> {
    let task_id = || {
        json!({
            "type": "string",
            "pattern": "^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$",
            "description": "Opaque Task identifier returned by Unica"
        })
    };
    vec![
        CompatibilityToolContract {
            name: "task.get",
            description: "Read the current durable Task state immediately without waiting or re-running the subject tool.",
            input_schema: task_schema(json!({"taskId": task_id()})),
        },
        CompatibilityToolContract {
            name: "task.result",
            description: "Wait for a Task result for a bounded interval; returns the canonical result or a new working receipt without re-running the subject tool.",
            input_schema: task_schema(json!({
                "taskId": task_id(),
                "waitMs": {
                    "type": "integer",
                    "minimum": 0,
                    "maximum": DEFAULT_TASK_RESULT_WAIT_MS,
                    "default": DEFAULT_TASK_RESULT_WAIT_MS,
                    "description": "Bounded wait in milliseconds; defaults to 7000"
                }
            })),
        },
        CompatibilityToolContract {
            name: "task.cancel",
            description: "Idempotently request cancellation and return the current durable Task state without re-running the subject tool.",
            input_schema: task_schema(json!({"taskId": task_id()})),
        },
    ]
}

fn task_schema(properties: Value) -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": properties,
        "required": ["taskId"]
    })
}

pub(crate) fn parse_task_tool_call(
    name: &str,
    arguments: &Map<String, Value>,
) -> Option<Result<TaskToolRequest, TaskToolError>> {
    let action = match name {
        "unica.task.get" => TaskToolAction::Get,
        "unica.task.result" => TaskToolAction::Result {
            wait_ms: DEFAULT_TASK_RESULT_WAIT_MS,
        },
        "unica.task.cancel" => TaskToolAction::Cancel,
        _ => return None,
    };
    Some(parse_known_task_call(action, arguments))
}

fn parse_known_task_call(
    mut action: TaskToolAction,
    arguments: &Map<String, Value>,
) -> Result<TaskToolRequest, TaskToolError> {
    let expected_len = usize::from(matches!(action, TaskToolAction::Result { .. })) + 1;
    if arguments.len() > expected_len
        || arguments
            .keys()
            .any(|key| key != "taskId" && key != "waitMs")
        || (!matches!(action, TaskToolAction::Result { .. }) && arguments.contains_key("waitMs"))
    {
        return Err(TaskToolError::BadArguments);
    }
    let task_id = arguments
        .get("taskId")
        .and_then(Value::as_str)
        .ok_or(TaskToolError::InvalidTaskId)?
        .parse()
        .map_err(|_| TaskToolError::InvalidTaskId)?;
    if let TaskToolAction::Result { .. } = action {
        let wait_ms = match arguments.get("waitMs") {
            None => DEFAULT_TASK_RESULT_WAIT_MS,
            Some(Value::Number(number)) => number.as_u64().ok_or(TaskToolError::BadWaitMs)?,
            Some(_) => return Err(TaskToolError::BadWaitMs),
        };
        if wait_ms > DEFAULT_TASK_RESULT_WAIT_MS {
            return Err(TaskToolError::BadWaitMs);
        }
        action = TaskToolAction::Result { wait_ms };
    }
    Ok(TaskToolRequest { task_id, action })
}

pub(crate) fn project_task_snapshot(
    snapshot: &CompatibilityTaskSnapshot,
    projection: CompatibilityProjection,
) -> Result<DomainResult, CompatibilityProjectionError> {
    let valid_shape = matches!(
        (
            snapshot.status,
            snapshot.result.is_some(),
            snapshot.has_failure
        ),
        (InvocationStatus::Queued, false, false)
            | (InvocationStatus::Working, false, false)
            | (InvocationStatus::Completed, true, false)
            | (InvocationStatus::Failed, false, true)
            | (InvocationStatus::Cancelled, false, false)
    );
    if !valid_shape {
        return Err(CompatibilityProjectionError::InvalidStatusPayload);
    }
    if snapshot.updated_at_epoch_ms < snapshot.created_at_epoch_ms {
        return Err(CompatibilityProjectionError::ReverseTimestampOrder);
    }
    if projection == CompatibilityProjection::TerminalResult
        && snapshot.status == InvocationStatus::Completed
    {
        return snapshot
            .result
            .clone()
            .ok_or(CompatibilityProjectionError::InvalidStatusPayload);
    }

    let (ok, summary, code) = match snapshot.status {
        InvocationStatus::Queued | InvocationStatus::Working => {
            (true, "Task is still working", None)
        }
        InvocationStatus::Completed => (true, "Task completed", None),
        InvocationStatus::Failed => (false, "Task failed", Some("task_failed")),
        InvocationStatus::Cancelled => (false, "Task was cancelled", Some("task_cancelled")),
    };
    let status = match snapshot.status {
        InvocationStatus::Queued => "queued",
        InvocationStatus::Working => "working",
        InvocationStatus::Completed => "completed",
        InvocationStatus::Failed => "failed",
        InvocationStatus::Cancelled => "cancelled",
    };
    let mut data = json!({
        "task": {
            "taskId": snapshot.task_id.to_string(),
            "status": status,
            "createdAtEpochMs": snapshot.created_at_epoch_ms,
            "updatedAtEpochMs": snapshot.updated_at_epoch_ms,
            "ttlMs": snapshot.ttl_ms,
            "pollIntervalMs": snapshot.poll_interval_ms
        }
    });
    if let Some(code) = code {
        data["code"] = json!(code);
    }
    let mut result = DomainResult::success(summary);
    result.ok = ok;
    result.data = Some(data);
    if matches!(
        snapshot.status,
        InvocationStatus::Queued | InvocationStatus::Working
    ) {
        result.next.push(json!({
            "tool": "unica.task.result",
            "args": {
                "taskId": snapshot.task_id.to_string(),
                "waitMs": snapshot.poll_interval_ms.min(DEFAULT_TASK_RESULT_WAIT_MS)
            }
        }));
    }
    Ok(result)
}

pub(crate) fn task_tool_error_result(error: TaskToolError) -> DomainResult {
    let mut result = DomainResult::success(match error {
        TaskToolError::InvalidTaskId => "Task identifier is not canonical",
        TaskToolError::BadWaitMs => "Task wait must be within 0..=7000 milliseconds",
        TaskToolError::BadArguments => "Task tool arguments are invalid",
        TaskToolError::TaskNotFound => "Task was not found",
        TaskToolError::TaskExpired => "Task has expired",
        TaskToolError::TaskBackendFailed => "Task state is unavailable",
        TaskToolError::TaskTransportFailed => "Task transport is unavailable",
        TaskToolError::TaskSessionClosed => "Task session is closed",
        TaskToolError::TaskProtocolFailed => "Task response failed validation",
        TaskToolError::ProjectionFailed => "Task state cannot be projected safely",
    });
    result.ok = false;
    result.data = Some(json!({"code": error.code()}));
    result
}

#[cfg(test)]
mod tests {
    use super::{
        compatibility_tool_contracts, parse_task_tool_call, project_task_snapshot,
        task_tool_error_result, CompatibilityProjection, CompatibilityTaskSnapshot, TaskToolAction,
        TaskToolError, DEFAULT_TASK_RESULT_WAIT_MS,
    };
    use crate::domain::invocation::{DomainResult, InvocationStatus, TaskId};
    use serde_json::{json, Map, Value};

    fn arguments(value: Value) -> Map<String, Value> {
        value.as_object().expect("argument object").clone()
    }

    fn snapshot(status: InvocationStatus) -> CompatibilityTaskSnapshot {
        CompatibilityTaskSnapshot::new(
            "f741d562-9d42-4a4f-a626-fcd5c3fb9bc4"
                .parse::<TaskId>()
                .unwrap(),
            status,
            None,
            status == InvocationStatus::Failed,
            1_777_012_345_678,
            1_777_012_346_789,
            3_600_000,
            250,
        )
    }

    #[test]
    fn compatibility_contracts_are_exact_and_teach_bounded_wait() {
        let contracts = compatibility_tool_contracts();
        assert_eq!(
            contracts
                .iter()
                .map(|contract| contract.name)
                .collect::<Vec<_>>(),
            ["task.get", "task.result", "task.cancel"]
        );
        for (name, required, fields) in [
            ("task.get", json!(["taskId"]), vec!["taskId"]),
            ("task.result", json!(["taskId"]), vec!["taskId", "waitMs"]),
            ("task.cancel", json!(["taskId"]), vec!["taskId"]),
        ] {
            let contract = contracts.iter().find(|entry| entry.name == name).unwrap();
            assert_eq!(contract.input_schema["additionalProperties"], false);
            assert_eq!(contract.input_schema["required"], required);
            assert_eq!(
                contract.input_schema["properties"]
                    .as_object()
                    .unwrap()
                    .keys()
                    .map(String::as_str)
                    .collect::<Vec<_>>(),
                fields
            );
            assert!(contract.description.contains("Task"));
        }
        let result = contracts
            .iter()
            .find(|entry| entry.name == "task.result")
            .unwrap();
        assert!(result.description.contains("bounded"));
        assert_eq!(
            result.input_schema["properties"]["waitMs"]["type"],
            "integer"
        );
        assert_eq!(result.input_schema["properties"]["waitMs"]["minimum"], 0);
        assert_eq!(
            result.input_schema["properties"]["waitMs"]["maximum"],
            7_000
        );
        assert_eq!(
            result.input_schema["properties"]["waitMs"]["default"],
            7_000
        );
    }

    #[test]
    fn compatibility_parser_accepts_only_canonical_ids_and_wait_zero_through_7000() {
        let task_id = "f741d562-9d42-4a4f-a626-fcd5c3fb9bc4";
        for (args, expected_wait) in [
            (json!({"taskId": task_id}), DEFAULT_TASK_RESULT_WAIT_MS),
            (json!({"taskId": task_id, "waitMs": 0}), 0),
            (json!({"taskId": task_id, "waitMs": 7_000}), 7_000),
        ] {
            let request = parse_task_tool_call("unica.task.result", &arguments(args))
                .expect("known compatibility tool")
                .expect("valid result request");
            assert_eq!(
                request.action,
                TaskToolAction::Result {
                    wait_ms: expected_wait
                }
            );
        }
        for (args, code) in [
            (json!({"taskId": task_id, "waitMs": 7_001}), "bad_wait_ms"),
            (json!({"taskId": task_id, "waitMs": -1}), "bad_wait_ms"),
            (json!({"taskId": task_id, "waitMs": "7"}), "bad_wait_ms"),
            (json!({"taskId": "not-canonical"}), "invalid_task_id"),
            (
                json!({"taskId": task_id.to_ascii_uppercase()}),
                "invalid_task_id",
            ),
            (
                json!({"taskId": task_id, "extra": true}),
                "bad_task_arguments",
            ),
        ] {
            let error = parse_task_tool_call("unica.task.result", &arguments(args))
                .expect("known compatibility tool")
                .expect_err("invalid request must fail closed");
            assert_eq!(error.code(), code);
        }
        assert!(parse_task_tool_call("unica.check", &Map::new()).is_none());
    }

    #[test]
    fn compatibility_receipt_is_closed_and_terminal_result_reuses_domain_json() {
        let working = project_task_snapshot(
            &snapshot(InvocationStatus::Working),
            CompatibilityProjection::State,
        )
        .expect("working receipt");
        assert_eq!(
            serde_json::to_value(&working).unwrap(),
            json!({
                "ok": true,
                "summary": "Task is still working",
                "data": {
                    "task": {
                        "taskId": "f741d562-9d42-4a4f-a626-fcd5c3fb9bc4",
                        "status": "working",
                        "createdAtEpochMs": 1_777_012_345_678u64,
                        "updatedAtEpochMs": 1_777_012_346_789u64,
                        "ttlMs": 3_600_000,
                        "pollIntervalMs": 250
                    }
                },
                "next": [{
                    "tool": "unica.task.result",
                    "args": {
                        "taskId": "f741d562-9d42-4a4f-a626-fcd5c3fb9bc4",
                        "waitMs": 250
                    }
                }]
            })
        );
        let serialized_value = serde_json::to_value(&working).unwrap();
        for forbidden in ["job", "work"] {
            assert!(
                serialized_value.get(forbidden).is_none(),
                "leaked {forbidden} slot"
            );
        }
        let serialized = serde_json::to_string(&serialized_value).unwrap();
        for forbidden_text in ["/private/", "bearer-secret", "runtime-secret"] {
            assert!(
                !serialized.contains(forbidden_text),
                "leaked {forbidden_text}"
            );
        }

        let subject = DomainResult {
            ok: false,
            at: Some("main:Catalog.Товары".into()),
            summary: "subject validation failed".into(),
            data: Some(json!({"code":"bad_value"})),
            changed: Vec::new(),
            warnings: Vec::new(),
            diagnostics: vec![json!({"code":"bad_value"})],
            artifacts: Vec::new(),
            next: Vec::new(),
            rev: Some("rev-7".into()),
            cursor: None,
        };
        let mut completed = snapshot(InvocationStatus::Completed);
        completed.result = Some(subject.clone());
        assert_eq!(
            project_task_snapshot(&completed, CompatibilityProjection::TerminalResult).unwrap(),
            subject,
            "ok:false subject result remains a completed canonical result"
        );
        assert_ne!(
            project_task_snapshot(&completed, CompatibilityProjection::State).unwrap(),
            subject,
            "get projects state while result projects the subject payload"
        );
    }

    #[test]
    fn compatibility_failure_cancel_and_lookup_errors_are_distinct_and_safe() {
        for (status, code) in [
            (InvocationStatus::Failed, "task_failed"),
            (InvocationStatus::Cancelled, "task_cancelled"),
        ] {
            let projected =
                project_task_snapshot(&snapshot(status), CompatibilityProjection::TerminalResult)
                    .unwrap();
            assert!(!projected.ok);
            assert_eq!(projected.data.as_ref().unwrap()["code"], code);
        }
        for error in [
            TaskToolError::InvalidTaskId,
            TaskToolError::TaskNotFound,
            TaskToolError::TaskExpired,
            TaskToolError::TaskBackendFailed,
        ] {
            let projected = task_tool_error_result(error);
            assert!(!projected.ok);
            assert_eq!(projected.data.as_ref().unwrap()["code"], error.code());
        }

        let mut reversed = snapshot(InvocationStatus::Working);
        reversed.updated_at_epoch_ms = reversed.created_at_epoch_ms - 1;
        assert!(project_task_snapshot(&reversed, CompatibilityProjection::State).is_err());
    }

    #[test]
    fn compatibility_projection_accepts_only_the_complete_status_result_failure_matrix() {
        let hostile = DomainResult::success("hostile result /private/secret bearer-secret");
        for status in [
            InvocationStatus::Queued,
            InvocationStatus::Working,
            InvocationStatus::Completed,
            InvocationStatus::Failed,
            InvocationStatus::Cancelled,
        ] {
            for has_result in [false, true] {
                for has_failure in [false, true] {
                    let valid = matches!(
                        (status, has_result, has_failure),
                        (InvocationStatus::Queued, false, false)
                            | (InvocationStatus::Working, false, false)
                            | (InvocationStatus::Completed, true, false)
                            | (InvocationStatus::Failed, false, true)
                            | (InvocationStatus::Cancelled, false, false)
                    );
                    let mut candidate = snapshot(status);
                    candidate.result = has_result.then(|| hostile.clone());
                    candidate.has_failure = has_failure;
                    for projection in [
                        CompatibilityProjection::State,
                        CompatibilityProjection::TerminalResult,
                    ] {
                        assert_eq!(
                            project_task_snapshot(&candidate, projection).is_ok(),
                            valid,
                            "status={status:?} result={has_result} failure={has_failure} projection={projection:?}"
                        );
                    }
                }
            }
        }
    }
}
