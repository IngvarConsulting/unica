use crate::application::invocation_store::{
    canonical_result_size, CanonicalResultSizeError, MAX_CANONICAL_RESULT_BYTES,
    MAX_TASK_RECORD_ENVELOPE_BYTES,
};
use crate::application::invocation_store_v5::V5SafeFailureReason;
use crate::application::receipt_ledger::ReceiptTerminalOutcome;
use crate::domain::invocation::{DomainResult, InvocationStatus};
use crate::infrastructure::daemon::protocol_v5::V5DaemonTaskSnapshot;
use chrono::{SecondsFormat, Utc};
use rmcp::model::{
    CallToolResult, CreateTaskResult, DetailedTask, ErrorCode, ErrorData, JsonObject, Task,
    TaskPayload, TaskStatus,
};
use serde::Serialize;
use std::io::{self, Write};

const MAX_MCP_TASK_PROJECTION_BYTES: usize =
    MAX_CANONICAL_RESULT_BYTES + MAX_TASK_RECORD_ENVELOPE_BYTES;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TaskProjectionError {
    TimestampOutOfRange,
    ReverseTimestampOrder,
    ResultTooLarge,
    Serialization,
}

struct ProjectionSizeWriter {
    bytes: usize,
    too_large: bool,
}

impl Write for ProjectionSizeWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let next = self.bytes.saturating_add(buffer.len());
        if next > MAX_MCP_TASK_PROJECTION_BYTES {
            self.too_large = true;
            return Err(io::Error::new(
                io::ErrorKind::FileTooLarge,
                "MCP task projection exceeds byte limit",
            ));
        }
        self.bytes = next;
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn ensure_projection_bounded(value: &impl Serialize) -> Result<(), TaskProjectionError> {
    let mut writer = ProjectionSizeWriter {
        bytes: 0,
        too_large: false,
    };
    let serialized = serde_json::to_writer(&mut writer, value);
    if writer.too_large {
        return Err(TaskProjectionError::ResultTooLarge);
    }
    serialized.map_err(|_| TaskProjectionError::Serialization)
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

pub(super) fn call_tool_result(
    result: &DomainResult,
) -> Result<CallToolResult, TaskProjectionError> {
    match canonical_result_size(result) {
        Ok(_) => {}
        Err(CanonicalResultSizeError::TooLarge) => return Err(TaskProjectionError::ResultTooLarge),
        Err(CanonicalResultSizeError::Serialization) => {
            return Err(TaskProjectionError::Serialization)
        }
        Err(CanonicalResultSizeError::Checkpoint(never)) => match never {},
    }
    let value = serde_json::to_value(result).map_err(|_| TaskProjectionError::Serialization)?;
    // `CallToolResult::structured` mirrors the complete JSON value into a text
    // ContentBlock. The canonical V13 result is already self-describing
    // structured content, so that convenience constructor would double an
    // allowed 8 MiB result on the MCP wire.
    let mut projected = CallToolResult::default();
    projected.structured_content = Some(value);
    projected.is_error = Some(!result.ok);
    ensure_projection_bounded(&projected)?;
    Ok(projected)
}

fn call_result_object(result: &DomainResult) -> Result<JsonObject, TaskProjectionError> {
    serde_json::to_value(call_tool_result(result)?)
        .map_err(|_| TaskProjectionError::Serialization)?
        .as_object()
        .cloned()
        .ok_or(TaskProjectionError::Serialization)
}

/// Native Task seed from a protocol-v5 snapshot: the durable identity, status
/// and timing exactly as the daemon stored them.
pub(super) fn create_task_result_v5(
    snapshot: &V5DaemonTaskSnapshot,
) -> Result<CreateTaskResult, TaskProjectionError> {
    let projected = CreateTaskResult::new(task_v5(snapshot)?);
    ensure_projection_bounded(&projected)?;
    Ok(projected)
}

/// `tasks/get` projection of a protocol-v5 snapshot. The v5 snapshot is a
/// closed union, so the status/result/failure matrix cannot be violated on
/// the wire; the only projection failures left are size, time and encoding.
pub(super) fn detailed_task_v5(
    snapshot: &V5DaemonTaskSnapshot,
) -> Result<DetailedTask, TaskProjectionError> {
    let payload = match snapshot {
        V5DaemonTaskSnapshot::Queued { .. } | V5DaemonTaskSnapshot::Working { .. } => {
            TaskPayload::Working
        }
        V5DaemonTaskSnapshot::Completed { result, .. } => TaskPayload::Completed {
            result: call_result_object(result)?,
        },
        V5DaemonTaskSnapshot::Failed { reason, .. } => TaskPayload::Failed {
            error: closed_failure_object(*reason)?,
        },
        V5DaemonTaskSnapshot::Cancelled { .. } => TaskPayload::Cancelled,
    };
    let projected = DetailedTask::new(task_v5(snapshot)?, payload);
    ensure_projection_bounded(&projected)?;
    Ok(projected)
}

fn task_v5(snapshot: &V5DaemonTaskSnapshot) -> Result<Task, TaskProjectionError> {
    if snapshot.updated_at_epoch_ms() < snapshot.created_at_epoch_ms() {
        return Err(TaskProjectionError::ReverseTimestampOrder);
    }
    let projected = Task::new(
        snapshot.task_id().to_string(),
        task_status(snapshot.status()),
        iso8601(snapshot.created_at_epoch_ms())?,
        iso8601(snapshot.updated_at_epoch_ms())?,
    )
    .with_ttl_ms(snapshot.ttl_ms())
    .with_poll_interval_ms(snapshot.poll_interval_ms());
    ensure_projection_bounded(&projected)?;
    Ok(projected)
}

/// The closed failure vocabulary of the v5 daemon and its fixed host-facing
/// text. The daemon never sends prose for a failure, so this table is the only
/// place a failed invocation gets words.
pub(super) fn closed_failure(reason: V5SafeFailureReason) -> (&'static str, &'static str) {
    match reason {
        V5SafeFailureReason::InvocationFailed => ("invocation_failed", "daemon invocation failed"),
        V5SafeFailureReason::ResultTooLarge => (
            "result_too_large",
            "daemon invocation result exceeded the canonical byte limit",
        ),
        V5SafeFailureReason::Interrupted => ("interrupted", "daemon invocation was interrupted"),
        V5SafeFailureReason::ResumeUnsupported => (
            "resume_unsupported",
            "daemon invocation cannot be resumed after restart",
        ),
        V5SafeFailureReason::PersistenceFailed => (
            "persistence_failed",
            "daemon invocation terminal state could not be persisted",
        ),
        V5SafeFailureReason::OutcomeUncertain => (
            "outcome_uncertain",
            "daemon invocation outcome is uncertain",
        ),
        V5SafeFailureReason::TaskCapacity => (
            "task_capacity",
            "daemon Task capacity was exhausted before execution",
        ),
        V5SafeFailureReason::WorkspaceCapacity => {
            ("workspace_capacity", "workspace capacity was exhausted")
        }
        V5SafeFailureReason::WorkspaceRegistryFailed => (
            "workspace_registry_failed",
            "workspace registry is unavailable",
        ),
    }
}

fn closed_failure_error(reason: V5SafeFailureReason) -> ErrorData {
    let (code, message) = closed_failure(reason);
    ErrorData::new(
        ErrorCode::INTERNAL_ERROR,
        message,
        Some(serde_json::json!({"code": code})),
    )
}

fn closed_failure_object(reason: V5SafeFailureReason) -> Result<JsonObject, TaskProjectionError> {
    serde_json::to_value(closed_failure_error(reason))
        .map_err(|_| TaskProjectionError::Serialization)?
        .as_object()
        .cloned()
        .ok_or(TaskProjectionError::Serialization)
}

/// Final interface value of a Direct terminal: the same `CallToolResult` a
/// completed Task later embeds, or the closed error a failed or cancelled
/// terminal answers with. Built before the acknowledgement is sent, so a
/// receipt is acknowledged only once its host-facing value exists.
pub(super) enum DirectProjection {
    Result(CallToolResult),
    Error(ErrorData),
}

pub(super) fn project_direct_terminal(
    terminal: &ReceiptTerminalOutcome,
) -> Result<DirectProjection, TaskProjectionError> {
    Ok(match terminal {
        ReceiptTerminalOutcome::Completed { result } => {
            DirectProjection::Result(call_tool_result(result)?)
        }
        ReceiptTerminalOutcome::Failed { reason } => {
            DirectProjection::Error(closed_failure_error(*reason))
        }
        ReceiptTerminalOutcome::Cancelled => DirectProjection::Error(ErrorData::new(
            ErrorCode::INTERNAL_ERROR,
            "daemon invocation was cancelled",
            Some(serde_json::json!({"code": "invocation_cancelled"})),
        )),
    })
}

pub(super) fn projection_error(error: TaskProjectionError) -> ErrorData {
    let code = match error {
        TaskProjectionError::ResultTooLarge => "result_too_large",
        _ => "task_projection_failed",
    };
    ErrorData::new(
        ErrorCode::INTERNAL_ERROR,
        code,
        Some(serde_json::json!({"code": code})),
    )
}

#[cfg(test)]
mod tests {
    use crate::domain::invocation::{DomainResult, InvocationId, TaskId};
    use crate::infrastructure::daemon::protocol_v5::V5DaemonTaskSnapshot;
    use serde_json::{json, Value};

    fn v5_common() -> (
        TaskId,
        InvocationId,
        crate::application::receipt_ledger::ReceiptKeyDigest,
    ) {
        (
            "f741d562-9d42-4a4f-a626-fcd5c3fb9bc4".parse().unwrap(),
            InvocationId::new(),
            "07".repeat(32).parse().unwrap(),
        )
    }

    fn v5_working() -> V5DaemonTaskSnapshot {
        let (task_id, invocation_id, receipt_key_digest) = v5_common();
        V5DaemonTaskSnapshot::Working {
            task_id,
            invocation_id,
            receipt_key_digest,
            created_at_epoch_ms: 1_777_012_345_678,
            updated_at_epoch_ms: 1_777_012_346_789,
            ttl_ms: 3_600_000,
            poll_interval_ms: 250,
            version: 2,
            cancel_requested: false,
        }
    }

    fn v5_terminal_digest() -> crate::application::receipt_ledger::TerminalDigest {
        "09".repeat(32).parse().unwrap()
    }

    #[test]
    fn v5_native_projection_keeps_durable_time_ttl_and_maps_queued_to_working() {
        let (task_id, invocation_id, receipt_key_digest) = v5_common();
        let queued = V5DaemonTaskSnapshot::Queued {
            task_id,
            invocation_id,
            receipt_key_digest,
            created_at_epoch_ms: 1_777_012_345_678,
            updated_at_epoch_ms: 1_777_012_346_789,
            ttl_ms: 3_600_000,
            poll_interval_ms: 250,
            version: 1,
            cancel_requested: false,
        };

        let seed = super::create_task_result_v5(&queued).expect("project queued seed");
        let detailed = super::detailed_task_v5(&queued).expect("project queued task");

        assert_eq!(seed.task.task_id, task_id.to_string());
        assert_eq!(seed.task.status, rmcp::model::TaskStatus::Working);
        assert_eq!(seed.task.created_at, "2026-04-24T06:32:25.678Z");
        assert_eq!(seed.task.last_updated_at, "2026-04-24T06:32:26.789Z");
        assert_eq!(seed.task.ttl_ms, Some(3_600_000));
        assert_eq!(seed.task.poll_interval_ms, Some(250));
        assert!(matches!(
            detailed.payload,
            rmcp::model::TaskPayload::Working
        ));
        let raw = serde_json::to_value(&detailed).unwrap();
        let mut keys = raw.as_object().unwrap().keys().collect::<Vec<_>>();
        keys.sort();
        assert_eq!(
            keys,
            [
                "createdAt",
                "lastUpdatedAt",
                "pollIntervalMs",
                "status",
                "taskId",
                "ttlMs"
            ]
        );
    }

    #[test]
    fn v5_completed_task_embeds_the_exact_direct_call_result() {
        let result = DomainResult {
            ok: false,
            at: Some("main:Catalog.Товары".into()),
            summary: "checked".into(),
            data: Some(json!({"nested": [1, 2, 3]})),
            changed: vec![],
            warnings: vec![],
            diagnostics: vec![json!({"code": "bad_value"})],
            artifacts: vec![],
            next: vec![],
            rev: None,
            cursor: None,
        };
        let direct = match super::project_direct_terminal(
            &crate::application::receipt_ledger::ReceiptTerminalOutcome::Completed {
                result: Box::new(result.clone()),
            },
        )
        .expect("project direct terminal")
        {
            super::DirectProjection::Result(projected) => projected,
            super::DirectProjection::Error(error) => panic!("completed terminal errored: {error}"),
        };
        let (task_id, invocation_id, receipt_key_digest) = v5_common();
        let completed = V5DaemonTaskSnapshot::Completed {
            task_id,
            invocation_id,
            receipt_key_digest,
            created_at_epoch_ms: 1_777_012_345_678,
            updated_at_epoch_ms: 1_777_012_346_789,
            ttl_ms: 3_600_000,
            poll_interval_ms: 250,
            version: 3,
            cancel_requested: false,
            terminal_epoch_ms: 1_777_012_346_789,
            terminal_digest: v5_terminal_digest(),
            result: Box::new(result),
        };

        let detailed = super::detailed_task_v5(&completed).expect("completed task");
        let rmcp::model::TaskPayload::Completed { result: embedded } = detailed.payload else {
            panic!("completed task must embed the original call result");
        };

        assert_eq!(
            Value::Object(embedded),
            serde_json::to_value(&direct).expect("serialize direct result")
        );
        assert_eq!(direct.is_error, Some(true));
        assert!(direct.content.is_empty());
    }

    #[test]
    fn v5_failed_and_cancelled_terminals_answer_only_the_closed_vocabulary() {
        use crate::application::invocation_store_v5::V5SafeFailureReason;

        for reason in V5SafeFailureReason::ALL {
            let (code, message) = super::closed_failure(reason);
            assert_eq!(code, reason.wire_name(), "code follows the wire name");
            assert!(!message.is_empty());
            let (task_id, invocation_id, receipt_key_digest) = v5_common();
            let failed = V5DaemonTaskSnapshot::Failed {
                task_id,
                invocation_id,
                receipt_key_digest,
                created_at_epoch_ms: 1_777_012_345_678,
                updated_at_epoch_ms: 1_777_012_346_789,
                ttl_ms: 3_600_000,
                poll_interval_ms: 250,
                version: 3,
                cancel_requested: false,
                terminal_epoch_ms: 1_777_012_346_789,
                terminal_digest: v5_terminal_digest(),
                reason,
            };
            let detailed = super::detailed_task_v5(&failed).expect("failed task");
            let rmcp::model::TaskPayload::Failed { error } = detailed.payload else {
                panic!("failed task must embed a JSON-RPC error");
            };
            assert_eq!(error["code"], -32603);
            assert_eq!(error["message"], message);
            assert_eq!(error["data"], json!({"code": code}));
            let direct = super::project_direct_terminal(
                &crate::application::receipt_ledger::ReceiptTerminalOutcome::Failed { reason },
            )
            .expect("project failed direct terminal");
            let super::DirectProjection::Error(direct) = direct else {
                panic!("failed direct terminal must be an error");
            };
            assert_eq!(direct.message, message);
            assert_eq!(direct.data, Some(json!({"code": code})));
        }

        let cancelled = super::project_direct_terminal(
            &crate::application::receipt_ledger::ReceiptTerminalOutcome::Cancelled,
        )
        .expect("project cancelled direct terminal");
        let super::DirectProjection::Error(cancelled) = cancelled else {
            panic!("cancelled direct terminal must be an error");
        };
        assert_eq!(cancelled.code, rmcp::model::ErrorCode::INTERNAL_ERROR);
        assert_eq!(cancelled.message, "daemon invocation was cancelled");
        assert_eq!(
            cancelled.data,
            Some(json!({"code": "invocation_cancelled"}))
        );
        let (task_id, invocation_id, receipt_key_digest) = v5_common();
        let cancelled_task = V5DaemonTaskSnapshot::Cancelled {
            task_id,
            invocation_id,
            receipt_key_digest,
            created_at_epoch_ms: 1_777_012_345_678,
            updated_at_epoch_ms: 1_777_012_346_789,
            ttl_ms: 3_600_000,
            poll_interval_ms: 250,
            version: 3,
            cancel_requested: true,
            terminal_epoch_ms: 1_777_012_346_789,
            terminal_digest: v5_terminal_digest(),
        };
        assert!(matches!(
            super::detailed_task_v5(&cancelled_task).unwrap().payload,
            rmcp::model::TaskPayload::Cancelled
        ));
    }

    #[test]
    fn v5_projection_rejects_reverse_timestamps_and_oversized_results() {
        use crate::application::invocation_store::MAX_CANONICAL_RESULT_BYTES;

        let mut reversed = v5_working();
        if let V5DaemonTaskSnapshot::Working {
            updated_at_epoch_ms,
            created_at_epoch_ms,
            ..
        } = &mut reversed
        {
            *updated_at_epoch_ms = *created_at_epoch_ms - 1;
        }
        assert!(super::create_task_result_v5(&reversed).is_err());
        assert!(super::detailed_task_v5(&reversed).is_err());

        let oversized = DomainResult::success("x".repeat(MAX_CANONICAL_RESULT_BYTES + 1));
        assert!(matches!(
            super::project_direct_terminal(
                &crate::application::receipt_ledger::ReceiptTerminalOutcome::Completed {
                    result: Box::new(oversized),
                },
            ),
            Err(super::TaskProjectionError::ResultTooLarge)
        ));
    }
}
