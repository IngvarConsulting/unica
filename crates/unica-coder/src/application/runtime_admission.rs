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

/// One named applied-risk reason carried into the warning and the receipt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RuntimeRiskNotice {
    pub(crate) code: &'static str,
    pub(crate) message: String,
}

/// ADR-0074: a classified applied operation is warned about and executed; an
/// unclassified one still fails closed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RuntimeRiskOutcome {
    Warned(RuntimeRiskNotice),
    Refused(RuntimeAdmissionFailure),
}

pub(crate) fn runtime_risk_notice(
    tool_name: &str,
    args: &Map<String, Value>,
) -> Result<RuntimeRiskOutcome, String> {
    let operation = args
        .get("operation")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{tool_name} requires string `operation` argument"))?;
    let capability = runtime_completion_capability(operation, args);

    let (code, message) = match capability {
        RuntimeCompletionCapability::CriticalNonAbortable => (
            "runtime_risk_critical_non_abortable",
            format!(
                "operation `{operation}` contains a CriticalNonAbortable runner phase, so cancellation is deferred until that phase ends"
            ),
        ),
        RuntimeCompletionCapability::PublicationWithoutBoundedRecovery => (
            "runtime_risk_publication_without_bounded_recovery",
            format!(
                "operation `{operation}` writes or publishes persistent state without a bounded recovery contract"
            ),
        ),
        RuntimeCompletionCapability::UnprovenExternalProcessOwnership => (
            "runtime_risk_unproven_process_ownership",
            format!(
                "operation `{operation}` may create a separately grouped platform process whose ownership and cleanup are not proved for every runner failure path"
            ),
        ),
        RuntimeCompletionCapability::Detached => (
            "runtime_risk_detached_child",
            format!(
                "operation `{operation}` would detach a child process, so this call cannot observe its exit"
            ),
        ),
        RuntimeCompletionCapability::Unclassified => {
            return Ok(RuntimeRiskOutcome::Refused(
                runtime_receipt_admission_failure(tool_name, args)?,
            ));
        }
    };

    Ok(RuntimeRiskOutcome::Warned(RuntimeRiskNotice {
        code,
        message,
    }))
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
    fn every_classified_applied_runtime_operation_is_warned_with_its_reason() {
        for (args, code, reason) in [
            (
                json!({"operation": "config-init"}),
                "runtime_risk_publication_without_bounded_recovery",
                "persistent state",
            ),
            (
                json!({"operation": "init"}),
                "runtime_risk_critical_non_abortable",
                "CriticalNonAbortable",
            ),
            (
                json!({"operation": "build"}),
                "runtime_risk_critical_non_abortable",
                "CriticalNonAbortable",
            ),
            (
                json!({"operation": "dump", "mode": "full"}),
                "runtime_risk_publication_without_bounded_recovery",
                "persistent state",
            ),
            (
                json!({"operation": "convert"}),
                "runtime_risk_publication_without_bounded_recovery",
                "persistent state",
            ),
            (
                json!({"operation": "make"}),
                "runtime_risk_publication_without_bounded_recovery",
                "persistent state",
            ),
            (
                json!({"operation": "load"}),
                "runtime_risk_critical_non_abortable",
                "CriticalNonAbortable",
            ),
            (
                json!({"operation": "syntax", "mode": "designer-config"}),
                "runtime_risk_unproven_process_ownership",
                "separately grouped platform process",
            ),
            (
                json!({"operation": "syntax", "mode": "edt"}),
                "runtime_risk_unproven_process_ownership",
                "separately grouped platform process",
            ),
            (
                json!({"operation": "test"}),
                "runtime_risk_critical_non_abortable",
                "CriticalNonAbortable",
            ),
            (
                json!({"operation": "extensions"}),
                "runtime_risk_critical_non_abortable",
                "CriticalNonAbortable",
            ),
            (
                json!({"operation": "tools-download"}),
                "runtime_risk_publication_without_bounded_recovery",
                "persistent state",
            ),
            (
                json!({"operation": "launch", "waitForExit": true}),
                "runtime_risk_unproven_process_ownership",
                "separately grouped platform process",
            ),
            (
                json!({"operation": "launch", "waitForExit": false}),
                "runtime_risk_detached_child",
                "detach a child process",
            ),
        ] {
            let outcome =
                runtime_risk_notice("unica.runtime.execute", args.as_object().unwrap()).unwrap();

            match outcome {
                RuntimeRiskOutcome::Warned(notice) => {
                    assert_eq!(notice.code, code, "{args}");
                    assert!(notice.message.contains(reason), "{notice:?}");
                }
                RuntimeRiskOutcome::Refused(failure) => {
                    panic!("{args} must be warned, not refused: {failure:?}")
                }
            }
        }
    }

    #[test]
    fn unclassified_applied_operation_still_fails_closed() {
        let args = json!({"operation": "syntax"});

        let outcome =
            runtime_risk_notice("unica.runtime.execute", args.as_object().unwrap()).unwrap();

        match outcome {
            RuntimeRiskOutcome::Refused(failure) => {
                assert_eq!(failure.code, "runtime_operation_unbounded");
                assert!(
                    failure.message.contains("no reviewed terminal-receipt"),
                    "{failure:?}"
                );
            }
            RuntimeRiskOutcome::Warned(notice) => {
                panic!("an unclassified operation must fail closed: {notice:?}")
            }
        }
    }

    #[test]
    fn canonical_runtime_surface_has_an_explicit_risk_classification() {
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
                    "{operation} {mode:?} needs a reviewed risk classification"
                );
            }
        }
    }
}
