use crate::domain::invocation::{DomainResult, InvocationStatus};
use crate::infrastructure::daemon::protocol::DaemonTaskSnapshot;
use chrono::{SecondsFormat, Utc};
use rmcp::model::{
    CallToolResult, CreateTaskResult, DetailedTask, ErrorCode, ErrorData, JsonObject, Task,
    TaskPayload, TaskStatus,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TaskProjectionError {
    TimestampOutOfRange,
    MissingCompletedResult,
    MissingFailure,
    UnexpectedPayload,
    Serialization,
}

fn iso8601(epoch_ms: u64) -> Result<String, TaskProjectionError> {
    let millis = i64::try_from(epoch_ms).map_err(|_| TaskProjectionError::TimestampOutOfRange)?;
    chrono::DateTime::<Utc>::from_timestamp_millis(millis)
        .map(|timestamp| timestamp.to_rfc3339_opts(SecondsFormat::Millis, true))
        .ok_or(TaskProjectionError::TimestampOutOfRange)
}

fn task_status(status: InvocationStatus) -> TaskStatus {
    match status {
        InvocationStatus::Queued | InvocationStatus::Working => TaskStatus::Working,
        InvocationStatus::Completed => TaskStatus::Completed,
        InvocationStatus::Failed => TaskStatus::Failed,
        InvocationStatus::Cancelled => TaskStatus::Cancelled,
    }
}

fn task(snapshot: &DaemonTaskSnapshot) -> Result<Task, TaskProjectionError> {
    Ok(Task::new(
        snapshot.task_id.to_string(),
        task_status(snapshot.status),
        iso8601(snapshot.created_at_epoch_ms)?,
        iso8601(snapshot.updated_at_epoch_ms)?,
    )
    .with_ttl_ms(snapshot.ttl_ms)
    .with_poll_interval_ms(snapshot.poll_interval_ms))
}

pub(super) fn call_tool_result(
    result: &DomainResult,
) -> Result<CallToolResult, TaskProjectionError> {
    let value = serde_json::to_value(result).map_err(|_| TaskProjectionError::Serialization)?;
    Ok(if result.ok {
        CallToolResult::structured(value)
    } else {
        CallToolResult::structured_error(value)
    })
}

pub(super) fn create_task_result(
    snapshot: &DaemonTaskSnapshot,
) -> Result<CreateTaskResult, TaskProjectionError> {
    Ok(CreateTaskResult::new(task(snapshot)?))
}

fn call_result_object(result: &DomainResult) -> Result<JsonObject, TaskProjectionError> {
    serde_json::to_value(call_tool_result(result)?)
        .map_err(|_| TaskProjectionError::Serialization)?
        .as_object()
        .cloned()
        .ok_or(TaskProjectionError::Serialization)
}

fn failure_object(snapshot: &DaemonTaskSnapshot) -> Result<JsonObject, TaskProjectionError> {
    let failure = snapshot
        .failure
        .as_ref()
        .ok_or(TaskProjectionError::MissingFailure)?;
    // The durable record stores a closed SafeFailureReason, but the
    // transport-neutral domain snapshot still carries the historical open
    // InvocationFailure shape. Re-close it at the untrusted wire edge: even a
    // malformed/foreign daemon response can never project its prose.
    let (code, message) = match failure.code.as_str() {
        "result_too_large" => (
            "result_too_large",
            "daemon invocation result exceeded the canonical byte limit",
        ),
        "interrupted" => ("interrupted", "daemon invocation was interrupted"),
        "resume_unsupported" => (
            "resume_unsupported",
            "daemon invocation cannot be resumed after restart",
        ),
        "persistence_failed" => (
            "persistence_failed",
            "daemon invocation terminal state could not be persisted",
        ),
        _ => ("invocation_failed", "daemon invocation failed"),
    };
    let error = ErrorData::new(
        ErrorCode::INTERNAL_ERROR,
        message,
        Some(serde_json::json!({"code": code})),
    );
    serde_json::to_value(error)
        .map_err(|_| TaskProjectionError::Serialization)?
        .as_object()
        .cloned()
        .ok_or(TaskProjectionError::Serialization)
}

pub(super) fn detailed_task(
    snapshot: &DaemonTaskSnapshot,
) -> Result<DetailedTask, TaskProjectionError> {
    let payload = match snapshot.status {
        InvocationStatus::Queued | InvocationStatus::Working
            if snapshot.result.is_none() && snapshot.failure.is_none() =>
        {
            TaskPayload::Working
        }
        InvocationStatus::Completed if snapshot.failure.is_none() => TaskPayload::Completed {
            result: call_result_object(
                snapshot
                    .result
                    .as_ref()
                    .ok_or(TaskProjectionError::MissingCompletedResult)?,
            )?,
        },
        InvocationStatus::Failed if snapshot.result.is_none() => TaskPayload::Failed {
            error: failure_object(snapshot)?,
        },
        InvocationStatus::Cancelled if snapshot.result.is_none() && snapshot.failure.is_none() => {
            TaskPayload::Cancelled
        }
        _ => return Err(TaskProjectionError::UnexpectedPayload),
    };
    Ok(DetailedTask::new(task(snapshot)?, payload))
}

pub(super) fn projection_error() -> ErrorData {
    ErrorData::new(
        ErrorCode::INTERNAL_ERROR,
        "task_projection_failed",
        Some(serde_json::json!({"code": "task_projection_failed"})),
    )
}

#[cfg(test)]
mod tests {
    use crate::domain::invocation::{
        DomainResult, InvocationFailure, InvocationId, InvocationStatus, TaskId,
    };
    use crate::infrastructure::daemon::protocol::DaemonTaskSnapshot;
    use serde_json::{json, Value};

    fn snapshot(status: InvocationStatus) -> DaemonTaskSnapshot {
        DaemonTaskSnapshot {
            task_id: TaskId::new(),
            invocation_id: InvocationId::new(),
            status,
            result: None,
            failure: None,
            poll_interval_ms: 250,
            created_at_epoch_ms: 1_777_012_345_678,
            updated_at_epoch_ms: 1_777_012_346_789,
            ttl_ms: 3_600_000,
        }
    }

    #[test]
    fn tasks_projection_preserves_durable_time_ttl_and_status() {
        let working = snapshot(InvocationStatus::Working);
        let seed = super::create_task_result(&working).expect("project task seed");

        assert_eq!(seed.task.task_id, working.task_id.to_string());
        assert_eq!(seed.task.status, rmcp::model::TaskStatus::Working);
        assert_eq!(seed.task.created_at, "2026-04-24T06:32:25.678Z");
        assert_eq!(seed.task.last_updated_at, "2026-04-24T06:32:26.789Z");
        assert_eq!(seed.task.ttl_ms, Some(3_600_000));
        assert_eq!(seed.task.poll_interval_ms, Some(250));
    }

    #[test]
    fn tasks_projection_reuses_exact_call_tool_result_for_terminal_task() {
        let result = DomainResult {
            ok: false,
            at: Some("main:Catalog.Товары".into()),
            summary: "checked".into(),
            data: Some(json!({"nested": [1, 2, 3]})),
            changed: vec![json!({"at": "main:Catalog.Товары.Attribute.Код"})],
            warnings: vec![json!({"code": "warning"})],
            diagnostics: vec![json!({"code": "bad_value"})],
            artifacts: vec![json!({"kind": "report"})],
            next: vec![json!({"op": "view"})],
            rev: Some("rev-7".into()),
            cursor: Some("cursor-2".into()),
        };
        let direct = super::call_tool_result(&result).expect("direct result");
        let mut completed = snapshot(InvocationStatus::Completed);
        completed.result = Some(result);
        let detailed = super::detailed_task(&completed).expect("completed task");
        let rmcp::model::TaskPayload::Completed { result } = detailed.payload else {
            panic!("completed task must embed the original call result");
        };

        assert_eq!(
            Value::Object(result),
            serde_json::to_value(direct).expect("serialize direct result")
        );
    }

    #[test]
    fn tasks_projection_uses_closed_failure_and_rejects_impossible_payloads() {
        let mut failed = snapshot(InvocationStatus::Failed);
        failed.failure = Some(InvocationFailure::new(
            "runtime_secret_code",
            "/private/workspace/provider stderr: bearer-secret",
        ));
        let projected = super::detailed_task(&failed).expect("failed task");
        let rmcp::model::TaskPayload::Failed { error } = projected.payload else {
            panic!("failed task must embed a JSON-RPC error");
        };
        assert_eq!(error["data"]["code"], "invocation_failed");
        let encoded = Value::Object(error).to_string();
        assert!(!encoded.contains("/private/"), "{encoded}");
        assert!(!encoded.contains("bearer-secret"), "{encoded}");
        assert!(!encoded.contains("runtime_secret_code"), "{encoded}");

        let completed_without_result = snapshot(InvocationStatus::Completed);
        assert!(super::detailed_task(&completed_without_result).is_err());
    }
}
