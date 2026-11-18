//! Feature-only integration facade for the protocol-v5 ReceiptLedger contract.
//!
//! The facade owns no scenario logic: every scenario runs through the
//! production scenario runner, and a shape or action the runner does not know
//! is a scenario error, never evidence of anything.

use crate::infrastructure::receipt_scenario::run_supported_receipt_scenario_for_test;
use serde::de::DeserializeOwned;
use std::fmt;
use std::str::FromStr;
use unica_execution::receipt_ledger::{
    canonical_v5_terminal, receipt_key_digest, request_scope_hash, task_link_digest,
    CoreIdentityDigest, ReceiptKey, ReceiptKeyDigest, ReceiptTerminalOutcome, RequestIdentity,
    RequestScopeHash, TaskLinkIdentity, V5ToolIdentity,
};
use unica_execution_contract::invocation::{
    InvocationId, NormalizedArgumentsHash, SafeIdentityHash, TaskId,
};

pub fn execute_scenario_json(request: &str) -> Result<String, String> {
    run_supported_receipt_scenario_for_test(request)
}

pub fn request_scope_hash_for_test(workspace_hint: &str) -> String {
    request_scope_hash(workspace_hint)
        .unwrap_or_else(|error| panic!("invalid request scope supplied by test: {error}"))
        .to_string()
}

pub fn receipt_key_digest_for_test(
    invocation_id: &str,
    reserved_task_id: &str,
    core_identity_digest: &str,
    tool_wire_name: &str,
    normalized_arguments_hash: &str,
    request_scope_hash_value: &str,
) -> String {
    let core_identity_digest: CoreIdentityDigest =
        parse_from_str(core_identity_digest, "core identity digest");
    let normalized_arguments_hash: NormalizedArgumentsHash =
        parse_application_json_string(normalized_arguments_hash, "normalized arguments hash");
    let request_scope_hash_value: RequestScopeHash =
        parse_from_str(request_scope_hash_value, "request scope hash");
    let request_identity = RequestIdentity::new(
        core_identity_digest,
        V5ToolIdentity::from_wire_name(tool_wire_name)
            .unwrap_or_else(|| panic!("invalid v5 tool identity supplied by test")),
        normalized_arguments_hash,
        request_scope_hash_value,
    );
    let invocation_id: InvocationId = parse_from_str(invocation_id, "invocation id");
    let reserved_task_id: TaskId = parse_from_str(reserved_task_id, "reserved task id");
    let key = ReceiptKey::new(invocation_id, reserved_task_id, request_identity);
    receipt_key_digest(&key).to_string()
}

pub fn task_link_digest_for_test(
    receipt_key_digest_value: &str,
    task_id: &str,
    invocation_id: &str,
    workspace_identity_hash: &str,
) -> String {
    let receipt_key_digest_value: ReceiptKeyDigest =
        parse_from_str(receipt_key_digest_value, "receipt key digest");
    let task_id: TaskId = parse_from_str(task_id, "task id");
    let invocation_id: InvocationId = parse_from_str(invocation_id, "invocation id");
    let workspace_identity_hash: SafeIdentityHash =
        parse_application_json_string(workspace_identity_hash, "workspace identity hash");
    let identity = TaskLinkIdentity::new(
        receipt_key_digest_value,
        task_id,
        invocation_id,
        workspace_identity_hash,
    );
    task_link_digest(&identity).to_string()
}

pub fn canonical_v5_terminal_for_test(terminal_json: &str) -> (Vec<u8>, String) {
    let outcome: ReceiptTerminalOutcome = serde_json::from_str(terminal_json)
        .unwrap_or_else(|error| panic!("invalid strict v5 terminal supplied by test: {error}"));
    let terminal = canonical_v5_terminal(&outcome, unica_bootstrap::MAX_RESPONSE_BUDGET)
        .unwrap_or_else(|error| panic!("v5 terminal cannot be canonicalized: {error}"));
    (terminal.payload().to_vec(), terminal.digest().to_string())
}

fn parse_from_str<T>(value: &str, label: &str) -> T
where
    T: FromStr,
    T::Err: fmt::Display,
{
    value
        .parse()
        .unwrap_or_else(|error| panic!("invalid {label} supplied by test: {error}"))
}

fn parse_application_json_string<T>(value: &str, label: &str) -> T
where
    T: DeserializeOwned,
{
    serde_json::from_value(serde_json::Value::String(value.to_owned()))
        .unwrap_or_else(|error| panic!("invalid {label} supplied by test: {error}"))
}
