use super::*;

#[allow(dead_code)] // legacy surface test support; production dispatches through daemon
pub fn call_tool_result_observed(
    app: &UnicaApplication,
    name: &str,
    args: &Map<String, Value>,
    cancellation: CancellationToken,
    progress: Arc<dyn ProgressSink>,
) -> Result<OperationResult, (i32, String)> {
    if let Some(result) = role_edit_argument_failure_result(name, args) {
        return Ok(result);
    }
    if let Some(result) = metadata_argument_failure_result(name, args) {
        return Ok(result);
    }
    app.call_tool_observed(name, args, cancellation, progress)
        .map_err(|message| (TOOL_EXECUTION_ERROR, message))
}

#[cfg(feature = "test-support")]
pub fn call_tool_text(
    app: &UnicaApplication,
    name: &str,
    args: &Map<String, Value>,
    cancellation: CancellationToken,
) -> Result<String, (i32, String)> {
    let result = call_tool_result(app, name, args, cancellation)?;
    serde_json::to_string_pretty(&result)
        .map_err(|error| (ErrorCode::INTERNAL_ERROR.0, error.to_string()))
}
