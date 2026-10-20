use crate::application::invocation_store::{
    canonical_result_size, CanonicalResultSizeError, MAX_CANONICAL_RESULT_BYTES,
    MAX_TASK_RECORD_ENVELOPE_BYTES,
};
use crate::application::invocation_store_v5::V5SafeFailureReason;
use crate::application::receipt_ledger::ReceiptTerminalOutcome;
use crate::application::result_delivery::error_delivery_text;
use crate::application::v13::source_text::apply_response_budget;
use crate::domain::invocation::{DomainResult, InvocationStatus};
use crate::infrastructure::daemon::protocol_v5::V5DaemonTaskSnapshot;
use chrono::{SecondsFormat, Utc};
use rmcp::model::{
    CallToolResult, ContentBlock, CreateTaskResult, DetailedTask, ErrorCode, ErrorData, JsonObject,
    Task, TaskPayload, TaskStatus,
};
use serde::Serialize;
use std::io::{self, Write};
use unica_bootstrap::ResponseBudget;

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
    budget: ResponseBudget,
) -> Result<CallToolResult, TaskProjectionError> {
    let mut public_result = apply_response_budget(result, budget.bytes());
    if public_result.source_text_read_hint.is_some() {
        public_result.to_mut().source_text_read_hint = None;
    }
    let result = public_result.as_ref();
    match canonical_result_size(result) {
        Ok(_) => {}
        Err(CanonicalResultSizeError::TooLarge) => return Err(TaskProjectionError::ResultTooLarge),
        Err(CanonicalResultSizeError::Serialization) => {
            return Err(TaskProjectionError::Serialization)
        }
        Err(CanonicalResultSizeError::Checkpoint(never)) => match never {},
    }
    if let (true, Some(text)) = (result.ok, result.source_text.as_deref()) {
        let projected = source_text_result(result, text)?;
        ensure_projection_bounded(&projected)?;
        return Ok(projected);
    }
    let value = serde_json::to_value(result).map_err(|_| TaskProjectionError::Serialization)?;
    // Hosts may ignore structuredContent when isError is true. Keep the
    // diagnostic fallback bounded, but mirror every recovery call intact:
    // truncating its arguments or dropping alternatives changes the route.
    // Subject data stays in structuredContent; the total projection is bounded below.
    let mut projected = CallToolResult::default();
    projected.structured_content = Some(value);
    projected.is_error = Some(!result.ok);
    if !result.ok {
        let text = error_delivery_text(result).map_err(|error| match error {
            CanonicalResultSizeError::TooLarge => TaskProjectionError::ResultTooLarge,
            CanonicalResultSizeError::Serialization => TaskProjectionError::Serialization,
            CanonicalResultSizeError::Checkpoint(never) => match never {},
        })?;
        projected.content.push(ContentBlock::text(text.diagnostic));
        if let Some(recovery) = text.recovery {
            projected.content.push(ContentBlock::text(recovery));
        }
    }
    ensure_projection_bounded(&projected)?;
    Ok(projected)
}

/// Ответ с исходным текстом BSL: два текстовых блока без `structuredContent`.
/// Первый — конверт результата JSON без самого текста, второй — текст UTF-8
/// как в файле. Хост, видящий `structuredContent`, может передать модели
/// только его, поэтому текст и конверт идут одним каналом `content`.
fn source_text_result(
    result: &DomainResult,
    text: &str,
) -> Result<CallToolResult, TaskProjectionError> {
    let mut envelope = result.clone();
    envelope.source_text = None;
    let envelope =
        serde_json::to_string(&envelope).map_err(|_| TaskProjectionError::Serialization)?;
    let mut projected = CallToolResult::default();
    projected.is_error = Some(false);
    projected.content.push(ContentBlock::text(envelope));
    projected.content.push(ContentBlock::text(text.to_string()));
    Ok(projected)
}

fn call_result_object(
    result: &DomainResult,
    budget: ResponseBudget,
) -> Result<JsonObject, TaskProjectionError> {
    serde_json::to_value(call_tool_result(result, budget)?)
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
    budget: ResponseBudget,
) -> Result<DetailedTask, TaskProjectionError> {
    let payload = match snapshot {
        V5DaemonTaskSnapshot::Queued { .. } | V5DaemonTaskSnapshot::Working { .. } => {
            TaskPayload::Working
        }
        V5DaemonTaskSnapshot::Completed { result, .. } => TaskPayload::Completed {
            result: call_result_object(result, budget)?,
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
    let mut projected = Task::new(
        snapshot.task_id().to_string(),
        task_status(snapshot.status()),
        iso8601(snapshot.created_at_epoch_ms())?,
        iso8601(snapshot.updated_at_epoch_ms())?,
    )
    .with_ttl_ms(snapshot.ttl_ms())
    .with_poll_interval_ms(snapshot.poll_interval_ms());
    if snapshot.cancel_requested() {
        projected = projected.with_status_message(
            "Cancellation was requested; check task status and result for the actual outcome",
        );
    }
    ensure_projection_bounded(&projected)?;
    Ok(projected)
}

pub(super) use crate::application::v13::task_tools::closed_task_failure as closed_failure;

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
    budget: ResponseBudget,
) -> Result<DirectProjection, TaskProjectionError> {
    Ok(match terminal {
        ReceiptTerminalOutcome::Completed { result } => {
            DirectProjection::Result(call_tool_result(result, budget)?)
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
    use unica_bootstrap::ResponseBudget;

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
        let detailed = super::detailed_task_v5(&queued, ResponseBudget::default())
            .expect("project queued task");

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

        let mut requested = queued.clone();
        let V5DaemonTaskSnapshot::Queued {
            cancel_requested, ..
        } = &mut requested
        else {
            unreachable!()
        };
        *cancel_requested = true;
        let seed = super::create_task_result_v5(&requested).unwrap();
        assert_eq!(
            serde_json::to_value(seed).unwrap()["statusMessage"],
            "Cancellation was requested; check task status and result for the actual outcome"
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
            next: vec![
                json!({"tool": "unica.view", "args": {"at": "main:Catalog.Товары.Can.props.set"}, "reason": "контракт операции"}),
                json!({"tool": "unica.view", "arguments": {"at": "main:Catalog.Товары.Attribute"}}),
            ],
            rev: None,
            cursor: None,
            page: None,
            source_text: None,
            source_text_read_hint: None,
        };
        let direct = match super::project_direct_terminal(
            &crate::application::receipt_ledger::ReceiptTerminalOutcome::Completed {
                result: Box::new(result.clone()),
            },
            ResponseBudget::default(),
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
            cancel_requested: true,
            terminal_epoch_ms: 1_777_012_346_789,
            terminal_digest: v5_terminal_digest(),
            result: Box::new(result),
        };

        let detailed =
            super::detailed_task_v5(&completed, ResponseBudget::default()).expect("completed task");
        assert_eq!(
            serde_json::to_value(&detailed).unwrap()["statusMessage"],
            "Cancellation was requested; check task status and result for the actual outcome"
        );
        let rmcp::model::TaskPayload::Completed { result: embedded } = detailed.payload else {
            panic!("completed task must embed the original call result");
        };

        assert_eq!(
            Value::Object(embedded),
            serde_json::to_value(&direct).expect("serialize direct result")
        );
        assert_eq!(direct.is_error, Some(true));
        let wire = serde_json::to_value(&direct).unwrap();
        let text = wire["content"][0]["text"]
            .as_str()
            .expect("readable refusal");
        let fallback: Value = serde_json::from_str(text).expect("complete small fallback");
        assert_eq!(fallback["summary"], "checked");
        assert_eq!(fallback["diagnostics"], json!([{"code": "bad_value"}]));
        assert!(fallback.get("data").is_none());
        let recovery: Value = serde_json::from_str(
            wire["content"][1]["text"]
                .as_str()
                .expect("complete recovery"),
        )
        .expect("recovery is a complete JSON object");
        assert_eq!(recovery["next"], wire["structuredContent"]["next"]);
        assert_eq!(recovery["next"].as_array().unwrap().len(), 2);
        assert!(recovery.get("data").is_none());
    }

    #[test]
    fn error_recovery_preserves_all_hints_beyond_the_diagnostic_text_budget() {
        let mut result = DomainResult::success("ambiguous operation target");
        result.ok = false;
        result.diagnostics = vec![json!({
            "code": "bad_value", "outcome": "fixCall", "message": "Ошибка🙂".repeat(4_096)
        })];
        result.next = (0..3)
            .map(|index| json!({
                "tool": "unica.view",
                "args": {"at": format!("main:Subsystem.{}.Can.props.set", "Раздел".repeat(160))},
                "reason": format!("кандидат {index}: \\\"\\n🙂; END_OF_HINT_{index}")
            }))
            .collect();
        result.data = Some(json!({"privateSubjectData": "must not enter text"}));
        assert!(serde_json::to_vec(&result.next).unwrap().len() > 4_096);
        let original = serde_json::to_value(&result).unwrap();

        let wire = serde_json::to_value(
            super::call_tool_result(&result, ResponseBudget::default()).unwrap(),
        )
        .unwrap();
        let diagnostic = wire["content"][0]["text"].as_str().unwrap();
        assert!(diagnostic.len() <= 4_096);
        assert!(diagnostic.ends_with(crate::application::result_delivery::ERROR_TEXT_TRUNCATED));
        let recovery: Value = serde_json::from_str(
            wire["content"][1]["text"]
                .as_str()
                .expect("untruncated recovery"),
        )
        .expect("long recovery remains valid JSON");
        assert_eq!(recovery, json!({"next": result.next}));
        assert_eq!(wire["structuredContent"], original);
        assert!(!serde_json::to_string(&wire["content"])
            .unwrap()
            .contains("privateSubjectData"));
    }

    #[test]
    fn error_recovery_that_exceeds_the_total_projection_limit_is_refused_whole() {
        use crate::application::invocation_store::MAX_CANONICAL_RESULT_BYTES;

        let mut result = DomainResult::success("ambiguous target");
        result.ok = false;
        result.next = vec![json!({
            "tool": "unica.view",
            "args": {"at": "x".repeat(MAX_CANONICAL_RESULT_BYTES / 2 + 65_536)}
        })];
        assert!(serde_json::to_vec(&result).unwrap().len() < MAX_CANONICAL_RESULT_BYTES);
        assert_eq!(
            super::call_tool_result(&result, ResponseBudget::default()).unwrap_err(),
            super::TaskProjectionError::ResultTooLarge
        );
    }

    #[test]
    fn error_fallback_is_bounded_without_losing_structured_diagnostics() {
        use crate::application::invocation_store::MAX_CANONICAL_RESULT_BYTES;

        for message in ["\"\\\n\u{0000}".repeat(4_096), "Ошибка🙂".repeat(4_096)] {
            let mut result = DomainResult::success("provider failed");
            result.ok = false;
            result.diagnostics = vec![json!({"code": "provider_unavailable", "message": message})];
            result.next = vec![json!({"tool":"unica.view", "args":{"at":"main:Catalog.Товары"}})];
            result.data = Some(json!({"padding": ""}));
            let base_size = serde_json::to_vec(&result).unwrap().len();
            result.data =
                Some(json!({"padding": "x".repeat(MAX_CANONICAL_RESULT_BYTES - base_size)}));
            assert_eq!(
                serde_json::to_vec(&result).unwrap().len(),
                MAX_CANONICAL_RESULT_BYTES
            );
            let outcome = crate::application::receipt_ledger::ReceiptTerminalOutcome::Completed {
                result: Box::new(result.clone()),
            };
            let admitted = crate::application::receipt_ledger::canonical_v5_terminal(&outcome)
                .expect("eight MiB with small recovery is admitted before persistence");
            let restored = crate::application::receipt_ledger::canonical_v5_terminal_from_shared(
                std::sync::Arc::new(outcome),
            )
            .unwrap();
            assert_eq!(admitted.payload(), restored.payload());
            assert_eq!(admitted.digest(), restored.digest());
            let projected = super::call_tool_result(&result, ResponseBudget::default())
                .expect("exact-limit error is admitted");
            let wire = serde_json::to_value(&projected).unwrap();
            let text = wire["content"][0]["text"].as_str().unwrap();
            assert!(
                text.len() <= crate::application::result_delivery::MAX_ERROR_DIAGNOSTIC_TEXT_BYTES
            );
            assert!(text.ends_with(crate::application::result_delivery::ERROR_TEXT_TRUNCATED));
            assert!(!text.contains("padding"));
            assert_eq!(
                wire["structuredContent"],
                serde_json::to_value(&result).unwrap()
            );

            let (task_id, invocation_id, receipt_key_digest) = v5_common();
            let maximum_timestamp =
                chrono::DateTime::<chrono::Utc>::MAX_UTC.timestamp_millis() as u64;
            let snapshot = V5DaemonTaskSnapshot::Completed {
                task_id,
                invocation_id,
                receipt_key_digest,
                created_at_epoch_ms: maximum_timestamp,
                updated_at_epoch_ms: maximum_timestamp,
                ttl_ms: u64::MAX,
                poll_interval_ms: u64::MAX,
                version: 3,
                cancel_requested: true,
                terminal_epoch_ms: 1_777_012_346_789,
                terminal_digest: v5_terminal_digest(),
                result: Box::new(result),
            };
            let detailed = super::detailed_task_v5(&snapshot, ResponseBudget::default())
                .expect("exact-limit Task is admitted");
            let detailed_wire = serde_json::to_value(&detailed).unwrap();
            assert_eq!(detailed_wire["result"], wire);
            let quoted_text_bytes: usize = wire["content"]
                .as_array()
                .unwrap()
                .iter()
                .map(|block| {
                    serde_json::to_vec(block["text"].as_str().unwrap())
                        .unwrap()
                        .len()
                })
                .sum();
            let envelope_bytes = serde_json::to_vec(&detailed).unwrap().len()
                - MAX_CANONICAL_RESULT_BYTES
                - quoted_text_bytes;
            assert!(
                envelope_bytes
                    <= crate::application::result_delivery::ERROR_DELIVERY_ENVELOPE_BYTES,
                "SDK metadata outgrew the pre-admission reserve: {envelope_bytes}"
            );
            assert!(
                serde_json::to_vec(&detailed).unwrap().len()
                    <= super::MAX_MCP_TASK_PROJECTION_BYTES
            );
        }
    }

    #[test]
    fn error_without_diagnostics_still_has_readable_summary() {
        let mut result = DomainResult::success("operation failed");
        result.ok = false;
        let wire = serde_json::to_value(
            super::call_tool_result(&result, ResponseBudget::default()).unwrap(),
        )
        .unwrap();
        let text: Value =
            serde_json::from_str(wire["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(
            text,
            json!({"summary": "operation failed", "diagnostics": []})
        );
        result.ok = true;
        assert!(super::call_tool_result(&result, ResponseBudget::default())
            .unwrap()
            .content
            .is_empty());
    }

    #[test]
    fn terminal_digest_covers_the_source_text() {
        use crate::application::receipt_ledger::{canonical_v5_terminal, ReceiptTerminalOutcome};
        let terminal = |text: &str| {
            let mut result = DomainResult::success("logical node text resolved");
            result.source_text = Some(text.to_string());
            canonical_v5_terminal(&ReceiptTerminalOutcome::Completed {
                result: Box::new(result),
            })
            .unwrap()
        };
        let first = terminal("Процедура А()\nКонецПроцедуры\n");
        let same = terminal("Процедура А()\nКонецПроцедуры\n");
        let edited = terminal("Процедура А()\r\nКонецПроцедуры\n");
        assert_eq!(first.digest(), same.digest());
        assert_ne!(first.digest(), edited.digest());
    }

    #[test]
    fn task_result_with_source_text_follows_the_client_budget() {
        let mut result = DomainResult::success("logical node text resolved");
        result.at = Some("main:CommonModule.Пример.Method.А.Body".into());
        result.source_text = Some(format!("{}\n", "a".repeat(39)).repeat(100));
        result.data = Some(
            json!({"at":result.at,"kind":"Body","title":"Body","can":[{"op":"code.replace"}]}),
        );
        result.rev = Some("revision".into());
        result.source_text_read_hint =
            Some(Box::new(crate::domain::invocation::SourceTextReadHint {
                from_line: 97,
                to_line: 196,
                prefix: None,
            }));
        let canonical_before = serde_json::to_vec(&result).unwrap();
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
            result: Box::new(result.clone()),
        };
        let embedded = |budget: usize| {
            let detailed =
                super::detailed_task_v5(&completed, ResponseBudget::from_bytes(budget)).unwrap();
            let rmcp::model::TaskPayload::Completed { result } = detailed.payload else {
                panic!("completed task must embed its result");
            };
            Value::Object(result)
        };
        let compatibility = crate::application::v13::task_tools::CompatibilityTaskSnapshot::new(
            task_id,
            crate::domain::invocation::InvocationStatus::Completed,
            Some(result.clone()),
            None,
            false,
            1_777_012_345_678,
            1_777_012_346_789,
            3_600_000,
            250,
        );
        let compatibility = crate::application::v13::task_tools::project_task_snapshot(
            &compatibility,
            crate::application::v13::task_tools::CompatibilityProjection::TerminalResult,
        )
        .unwrap();
        for budget in [16_384, 4_096, 3_000] {
            let direct = match super::project_direct_terminal(
                &crate::application::receipt_ledger::ReceiptTerminalOutcome::Completed {
                    result: Box::new(result.clone()),
                },
                ResponseBudget::from_bytes(budget),
            )
            .unwrap()
            {
                super::DirectProjection::Result(value) => serde_json::to_value(value).unwrap(),
                super::DirectProjection::Error(error) => panic!("{error}"),
            };
            let compat = serde_json::to_value(
                super::call_tool_result(&compatibility, ResponseBudget::from_bytes(budget))
                    .unwrap(),
            )
            .unwrap();
            assert_eq!(embedded(budget), direct);
            assert_eq!(compat, direct);
            assert!(!serde_json::to_string(&direct)
                .unwrap()
                .contains("sourceTextReadHint"));
            assert!(!serde_json::to_string(&direct).unwrap().contains("fromLine"));
        }
        let wide = embedded(16_384);
        assert_eq!(
            wide["content"][1]["text"].as_str().map(str::len),
            Some(4_000)
        );
        let narrow = embedded(4_096);
        assert_eq!(narrow["isError"], true, "{narrow}");
        assert_eq!(
            narrow["structuredContent"]["diagnostics"][0]["code"],
            "result_too_large"
        );
        assert!(narrow["structuredContent"]["next"][0]["args"]["at"]
            .as_str()
            .unwrap()
            .contains("Body#L97-L"));
        assert_eq!(
            embedded(16_384),
            wide,
            "a narrower reader never changes the saved result"
        );
        assert_eq!(serde_json::to_vec(&result).unwrap(), canonical_before);
    }

    #[test]
    fn source_text_result_is_an_envelope_then_raw_text_without_structured_content() {
        let text =
            "// Документ \"метода\".\r\n&НаСервере\nПроцедура А()\n\tВозврат;\nКонецПроцедуры\n";
        let mut result = DomainResult::success("logical node text resolved");
        result.at = Some("main:CommonModule.Пример.Method.А.Body".into());
        result.data = Some(json!({"at": "main:CommonModule.Пример.Method.А.Body", "kind": "Body"}));
        result.source_text = Some(text.to_string());
        result.source_text_read_hint =
            Some(Box::new(crate::domain::invocation::SourceTextReadHint {
                from_line: 1,
                to_line: 5,
                prefix: None,
            }));
        let direct = match super::project_direct_terminal(
            &crate::application::receipt_ledger::ReceiptTerminalOutcome::Completed {
                result: Box::new(result.clone()),
            },
            ResponseBudget::default(),
        )
        .expect("project direct terminal")
        {
            super::DirectProjection::Result(projected) => projected,
            super::DirectProjection::Error(error) => panic!("completed terminal errored: {error}"),
        };
        let wire = serde_json::to_value(&direct).unwrap();
        assert!(wire.get("structuredContent").is_none(), "{wire:#}");
        assert_eq!(wire["isError"], false);
        let blocks = wire["content"].as_array().unwrap();
        assert_eq!(blocks.len(), 2);
        let envelope: Value = serde_json::from_str(blocks[0]["text"].as_str().unwrap()).unwrap();
        let mut expected = result.clone();
        expected.source_text = None;
        expected.source_text_read_hint = None;
        assert_eq!(envelope, serde_json::to_value(&expected).unwrap());
        assert!(envelope.get("sourceText").is_none());
        // Текст идёт байт в байт, без экранирования JSON.
        assert_eq!(blocks[1]["type"], "text");
        assert_eq!(blocks[1]["text"].as_str().unwrap(), text);

        // Завершённое задание встраивает тот же ответ.
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
            result: Box::new(result.clone()),
        };
        let detailed =
            super::detailed_task_v5(&completed, ResponseBudget::default()).expect("completed task");
        let rmcp::model::TaskPayload::Completed { result: embedded } = detailed.payload else {
            panic!("completed task must embed the original call result");
        };
        assert_eq!(Value::Object(embedded), wire);

        // Отказ текста не несёт и остаётся структурным.
        result.ok = false;
        let refused = serde_json::to_value(
            super::call_tool_result(&result, ResponseBudget::default()).unwrap(),
        )
        .unwrap();
        assert!(refused["structuredContent"].is_object());
        assert!(!serde_json::to_string(&refused)
            .unwrap()
            .contains("sourceTextReadHint"));
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
                cancel_requested: true,
                terminal_epoch_ms: 1_777_012_346_789,
                terminal_digest: v5_terminal_digest(),
                reason,
            };
            let detailed =
                super::detailed_task_v5(&failed, ResponseBudget::default()).expect("failed task");
            assert_eq!(
                serde_json::to_value(&detailed).unwrap()["statusMessage"],
                "Cancellation was requested; check task status and result for the actual outcome"
            );
            let rmcp::model::TaskPayload::Failed { error } = detailed.payload else {
                panic!("failed task must embed a JSON-RPC error");
            };
            assert_eq!(error["code"], -32603);
            assert_eq!(error["message"], message);
            assert_eq!(error["data"], json!({"code": code}));
            let direct = super::project_direct_terminal(
                &crate::application::receipt_ledger::ReceiptTerminalOutcome::Failed { reason },
                ResponseBudget::default(),
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
            ResponseBudget::default(),
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
        let detailed = super::detailed_task_v5(&cancelled_task, ResponseBudget::default()).unwrap();
        assert_eq!(
            serde_json::to_value(&detailed).unwrap()["statusMessage"],
            "Cancellation was requested; check task status and result for the actual outcome"
        );
        assert!(matches!(
            detailed.payload,
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
        assert!(super::detailed_task_v5(&reversed, ResponseBudget::default()).is_err());

        let oversized = DomainResult::success("x".repeat(MAX_CANONICAL_RESULT_BYTES + 1));
        assert!(matches!(
            super::project_direct_terminal(
                &crate::application::receipt_ledger::ReceiptTerminalOutcome::Completed {
                    result: Box::new(oversized),
                },
                ResponseBudget::default(),
            ),
            Err(super::TaskProjectionError::ResultTooLarge)
        ));
    }
}
