use crate::daemon_router::*;
use crate::*;
use rmcp::model::{CallToolResult, ErrorData};
use serde_json::{Map, Value};
use std::time::{Duration, Instant};
use unica_execution::receipt_ledger::V5ToolIdentity;
use unica_execution::receipt_runtime::response::V5DaemonTaskSnapshot;

use serde_json::json;

pub fn arguments() -> Map<String, Value> {
    json!({"at": "main:Catalog.Товары"})
        .as_object()
        .expect("argument object")
        .clone()
}

pub fn deadline(host_remaining: Option<Duration>) -> FrontendInvocationDeadline {
    FrontendInvocationDeadline::new(Instant::now(), host_remaining)
}

pub fn call(
    router: &CanonicalDaemonRouter,
    host_remaining: Option<Duration>,
) -> Result<CanonicalCallOutcome, ErrorData> {
    (router.call)(
        V5ToolIdentity::View,
        &arguments(),
        &unica_bootstrap::HostRequest::default(),
        deadline(host_remaining),
        CancellationToken::new().into(),
    )
}

pub fn direct_result(outcome: Result<CanonicalCallOutcome, ErrorData>) -> CallToolResult {
    match outcome {
        Ok(CanonicalCallOutcome::Direct(result)) => result,
        Ok(CanonicalCallOutcome::Task(snapshot)) => {
            panic!("unexpected Task handoff: {snapshot:?}")
        }
        Err(error) => panic!("unexpected refusal: {error}"),
    }
}

/// A lost submit answers with the queued Task reserved for the call.
pub fn lost_task(outcome: Result<CanonicalCallOutcome, ErrorData>) -> V5DaemonTaskSnapshot {
    let task = match outcome {
        Ok(CanonicalCallOutcome::Task(task)) => task,
        Ok(CanonicalCallOutcome::Direct(result)) => panic!("unexpected direct: {result:?}"),
        Err(error) => panic!("unexpected refusal: {error}"),
    };
    assert_eq!(
        task.status(),
        unica_execution_contract::invocation::InvocationStatus::Queued
    );
    task
}
