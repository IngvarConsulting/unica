use serde_json::{Map, Value};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RuntimeAdmissionFailure {
    pub(crate) code: &'static str,
    pub(crate) message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RuntimeCompletionCapability {
    CriticalNonAbortable,
    PublicationWithoutBoundedRecovery,
    UnprovenExternalProcessOwnership,
    Detached,
    Unclassified,
}

pub(crate) fn runtime_receipt_admission_failure(
    tool_name: &str,
    args: &Map<String, Value>,
) -> Result<RuntimeAdmissionFailure, String> {
    let operation = args
        .get("operation")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{tool_name} requires string `operation` argument"))?;
    let capability = runtime_completion_capability(operation, args);

    // Every capability in the current table is fail-closed independently of
    // host configuration. A host budget becomes meaningful only when a future
    // capability is actually admitted, so do not let a missing marker mask the
    // more fundamental operation-level refusal.
    let (code, message) = match capability {
        RuntimeCompletionCapability::CriticalNonAbortable => (
            "runtime_operation_unbounded",
            format!(
                "operation `{operation}` contains a CriticalNonAbortable runner phase and has no bounded terminal-receipt contract"
            ),
        ),
        RuntimeCompletionCapability::PublicationWithoutBoundedRecovery => (
            "runtime_operation_unbounded",
            format!(
                "operation `{operation}` writes or publishes persistent state without a bounded recovery contract"
            ),
        ),
        RuntimeCompletionCapability::UnprovenExternalProcessOwnership => (
            "runtime_operation_unbounded",
            format!(
                "operation `{operation}` may create a separately grouped platform process whose ownership and cleanup are not proved for every runner failure path"
            ),
        ),
        RuntimeCompletionCapability::Detached => (
            "runtime_operation_unbounded",
            format!(
                "operation `{operation}` would detach a child process and cannot belong to one tools/call lifecycle"
            ),
        ),
        RuntimeCompletionCapability::Unclassified => (
            "runtime_operation_unbounded",
            format!(
                "operation `{operation}` has no reviewed terminal-receipt capability classification"
            ),
        ),
    };
    Ok(RuntimeAdmissionFailure { code, message })
}

fn runtime_completion_capability(
    operation: &str,
    args: &Map<String, Value>,
) -> RuntimeCompletionCapability {
    match operation {
        "syntax"
            if matches!(
                args.get("mode").and_then(Value::as_str),
                Some("designer-config" | "designer-modules" | "edt")
            ) =>
        {
            RuntimeCompletionCapability::UnprovenExternalProcessOwnership
        }
        "syntax" => RuntimeCompletionCapability::Unclassified,
        "launch" if args.get("waitForExit").and_then(Value::as_bool) == Some(true) => {
            RuntimeCompletionCapability::UnprovenExternalProcessOwnership
        }
        "launch" => RuntimeCompletionCapability::Detached,
        "init" | "build" | "load" | "test" | "extensions" => {
            RuntimeCompletionCapability::CriticalNonAbortable
        }
        "config-init" | "dump" | "convert" | "make" | "tools-download" => {
            RuntimeCompletionCapability::PublicationWithoutBoundedRecovery
        }
        _ => RuntimeCompletionCapability::Unclassified,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn every_current_applied_runtime_operation_fails_closed_without_a_host_budget() {
        for (args, reason) in [
            (json!({"operation": "config-init"}), "persistent state"),
            (json!({"operation": "init"}), "CriticalNonAbortable"),
            (json!({"operation": "build"}), "CriticalNonAbortable"),
            (
                json!({"operation": "dump", "mode": "full"}),
                "persistent state",
            ),
            (json!({"operation": "convert"}), "persistent state"),
            (json!({"operation": "make"}), "persistent state"),
            (json!({"operation": "load"}), "CriticalNonAbortable"),
            (
                json!({"operation": "syntax", "mode": "designer-config"}),
                "separately grouped platform process",
            ),
            (
                json!({"operation": "syntax", "mode": "designer-modules"}),
                "separately grouped platform process",
            ),
            (
                json!({"operation": "syntax", "mode": "edt"}),
                "separately grouped platform process",
            ),
            (json!({"operation": "test"}), "CriticalNonAbortable"),
            (json!({"operation": "extensions"}), "CriticalNonAbortable"),
            (json!({"operation": "tools-download"}), "persistent state"),
            (
                json!({"operation": "launch", "waitForExit": true}),
                "separately grouped platform process",
            ),
            (
                json!({"operation": "launch", "waitForExit": false}),
                "detach a child process",
            ),
        ] {
            let failure = runtime_receipt_admission_failure(
                "unica.runtime.execute",
                args.as_object().unwrap(),
            )
            .unwrap();

            assert_eq!(failure.code, "runtime_operation_unbounded");
            assert!(failure.message.contains(reason), "{failure:?}");
        }
    }

    #[test]
    fn canonical_runtime_surface_has_an_explicit_refusal_reason() {
        for operation in super::super::tool_contracts::RUNTIME_OPERATIONS {
            let mut modes: Vec<Option<&str>> = vec![None];
            if *operation == "syntax" {
                modes = super::super::tool_contracts::RUNTIME_SYNTAX_MODES
                    .iter()
                    .copied()
                    .map(Some)
                    .collect();
            }
            for mode in modes {
                let mut args = Map::from_iter([(
                    "operation".to_string(),
                    Value::String((*operation).to_string()),
                )]);
                if let Some(mode) = mode {
                    args.insert("mode".to_string(), Value::String(mode.to_string()));
                }

                let capability = runtime_completion_capability(operation, &args);
                assert_ne!(
                    capability,
                    RuntimeCompletionCapability::Unclassified,
                    "{operation} {mode:?} needs a reviewed refusal reason"
                );
            }
        }
    }
}
