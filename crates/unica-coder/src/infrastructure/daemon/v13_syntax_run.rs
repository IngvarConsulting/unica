use crate::application::AdapterOutcome;
use crate::domain::cancellation::CancellationToken;
use crate::domain::invocation::DomainResult;
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::internal_adapters::{RuntimeAdapter, RuntimeInvocation};
use serde_json::{json, Map, Value};
use std::time::Duration;

const SYNTAX_MODES: &[&str] = &["designer-config", "designer-modules", "edt"];
const SYNTAX_PROCESS_TIMEOUT: Duration = Duration::from_secs(300);
const SYNTAX_STDOUT_LIMIT: usize = 1024 * 1024;
const SYNTAX_STDERR_LIMIT: usize = 256 * 1024;
const BOOLEAN_FIELDS: &[&str] = &[
    "server",
    "thinClient",
    "webClient",
    "mobileClient",
    "externalConnection",
    "externalConnectionServer",
    "thickClientManagedApplication",
    "thickClientServerManagedApplication",
    "thickClientOrdinaryApplication",
    "thickClientServerOrdinaryApplication",
    "mobileAppClient",
    "mobileAppServer",
    "mobileClientDigiSign",
    "distributiveModules",
    "unreferenceProcedures",
    "handlersExistence",
    "emptyHandlers",
    "extendedModulesCheck",
    "checkUseSynchronousCalls",
    "checkUseModality",
    "unsupportedFunctional",
    "configLogIntegrity",
    "incorrectReferences",
    "allExtensions",
];

pub(super) fn canonical_syntax_invocation_args(
    args: &Map<String, Value>,
) -> Result<Map<String, Value>, String> {
    let mut mapped = Map::new();
    mapped.insert("operation".to_string(), json!("syntax"));
    for (key, value) in args {
        match key.as_str() {
            "mode" => {
                let mode = value
                    .as_str()
                    .ok_or_else(|| "syntax.check mode must be a string".to_string())?;
                if !SYNTAX_MODES.contains(&mode) {
                    return Err(format!(
                        "syntax.check mode must be one of: {}",
                        SYNTAX_MODES.join(", ")
                    ));
                }
                mapped.insert(key.clone(), value.clone());
            }
            "extension" => {
                if value.as_str().is_none_or(|value| value.trim().is_empty()) {
                    return Err("syntax.check extension must be non-empty text".to_string());
                }
                mapped.insert(key.clone(), value.clone());
            }
            "projects" => {
                let projects = value
                    .as_array()
                    .ok_or_else(|| "syntax.check projects must be an array".to_string())?;
                if projects.is_empty()
                    || projects.iter().any(|project| {
                        project
                            .as_str()
                            .is_none_or(|project| project.trim().is_empty())
                    })
                {
                    return Err("syntax.check projects must contain non-empty strings".to_string());
                }
                mapped.insert(key.clone(), value.clone());
            }
            key if BOOLEAN_FIELDS.contains(&key) => {
                if !value.is_boolean() {
                    return Err(format!("syntax.check {key} must be a boolean"));
                }
                mapped.insert(key.to_string(), value.clone());
            }
            _ => return Err(format!("syntax.check does not accept argument `{key}`")),
        }
    }
    if !mapped.contains_key("mode") {
        return Err("syntax.check requires string argument `mode`".to_string());
    }
    let mode = mapped
        .get("mode")
        .and_then(Value::as_str)
        .expect("validated mode exists");
    if mode == "edt"
        && BOOLEAN_FIELDS
            .iter()
            .any(|key| mapped.get(*key).and_then(Value::as_bool) == Some(true))
    {
        return Err("syntax.check EDT mode does not accept Designer flags".to_string());
    }
    if mode != "edt" && mapped.contains_key("projects") {
        return Err("syntax.check projects are accepted only in EDT mode".to_string());
    }
    Ok(mapped)
}

pub(super) fn execute_syntax_check(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
    cancellation: &CancellationToken,
) -> DomainResult {
    let mapped = match canonical_syntax_invocation_args(args) {
        Ok(mapped) => mapped,
        Err(error) => return DomainResult::canonical_rejection(None, "bad_value", error),
    };
    match RuntimeAdapter::new().invoke_cancellable_bounded(
        RuntimeInvocation {
            tool_name: "unica.run",
            args: &mapped,
            context,
            dry_run: false,
            mutating: false,
        },
        cancellation,
        SYNTAX_PROCESS_TIMEOUT,
        (SYNTAX_STDOUT_LIMIT, SYNTAX_STDERR_LIMIT),
    ) {
        Ok(outcome) => runtime_outcome_result(outcome.outcome, outcome.data),
        Err(error) => DomainResult::canonical_rejection(None, "provider_unavailable", error),
    }
}

pub(super) fn runtime_outcome_result(
    outcome: AdapterOutcome,
    _adapter_data: Option<Value>,
) -> DomainResult {
    if !outcome.ok {
        let cancelled = outcome
            .errors
            .iter()
            .any(|error| error.to_ascii_lowercase().contains("cancel"));
        let code = if cancelled {
            "cancelled"
        } else {
            "runtime_failed"
        };
        let message = if cancelled {
            "syntax check was cancelled".to_string()
        } else {
            "syntax check failed in the bounded runtime provider".to_string()
        };
        let mut result = DomainResult::canonical_rejection(None, code, message);
        if !outcome.warnings.is_empty() {
            result.warnings = vec![json!({"message": "syntax provider reported warnings"})];
        }
        return result;
    }
    let mut result = DomainResult::success(outcome.summary);
    let mut data = Map::new();
    data.insert("status".to_string(), json!("completed"));
    result.data = Some(Value::Object(data));
    if !outcome.warnings.is_empty() {
        result.warnings = vec![json!({"message": "syntax provider reported warnings"})];
    }
    result
}

#[cfg(test)]
mod tests {
    use super::{canonical_syntax_invocation_args, execute_syntax_check, runtime_outcome_result};
    use crate::application::AdapterOutcome;
    use crate::domain::cancellation::CancellationToken;
    use crate::infrastructure::workspace::discover_workspace;
    use serde_json::{json, Map};

    #[test]
    fn canonical_syntax_args_are_closed_and_map_without_raw_command_fields() {
        let args = Map::from_iter([
            ("mode".to_string(), json!("designer-modules")),
            ("server".to_string(), json!(true)),
            ("thinClient".to_string(), json!(true)),
        ]);
        let mapped = canonical_syntax_invocation_args(&args).unwrap();
        assert_eq!(mapped["operation"], "syntax");
        assert_eq!(mapped["mode"], "designer-modules");
        assert_eq!(mapped["server"], true);
        assert!(!mapped.contains_key("command"));
        assert!(canonical_syntax_invocation_args(&Map::from_iter([(
            "query".to_string(),
            json!("select 1")
        )]))
        .is_err());
    }

    #[test]
    fn runtime_outcome_is_mapped_without_exposing_the_command_line() {
        let result = runtime_outcome_result(
            AdapterOutcome {
                ok: true,
                summary: "syntax completed".to_string(),
                changes: Vec::new(),
                warnings: vec!["warning".to_string()],
                errors: Vec::new(),
                artifacts: Vec::new(),
                stdout: Some("checked".to_string()),
                stderr: Some(String::new()),
                command: Some(vec!["secret-runner-path".to_string()]),
            },
            None,
        );
        assert!(result.ok);
        assert_eq!(result.data.as_ref().unwrap()["status"], "completed");
        assert!(result.data.as_ref().unwrap().get("stdout").is_none());
        assert!(!result
            .data
            .as_ref()
            .unwrap()
            .to_string()
            .contains("secret-runner-path"));
        assert_eq!(
            result.warnings,
            vec![json!({"message": "syntax provider reported warnings"})]
        );
        assert!(result.artifacts.is_empty());
    }

    #[test]
    fn syntax_cancellation_and_provider_failure_publish_only_closed_results() {
        let context = discover_workspace(Some(std::env::current_dir().unwrap())).unwrap();
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let cancelled = execute_syntax_check(
            &Map::from_iter([("mode".to_string(), json!("designer-config"))]),
            &context,
            &cancellation,
        );
        assert!(!cancelled.ok);
        assert_eq!(cancelled.diagnostics[0]["code"], "cancelled");
        assert!(!serde_json::to_string(&cancelled)
            .unwrap()
            .contains(&context.workspace_root.to_string_lossy().into_owned()));

        let failed = runtime_outcome_result(
            AdapterOutcome {
                ok: false,
                summary: "raw /private/workspace provider failure".to_string(),
                changes: Vec::new(),
                warnings: vec!["raw /private/workspace warning".to_string()],
                errors: vec!["raw /private/workspace error".to_string()],
                artifacts: vec!["/private/workspace/report.log".to_string()],
                stdout: Some("raw /private/workspace stdout".to_string()),
                stderr: Some("raw /private/workspace stderr".to_string()),
                command: Some(vec!["/private/runner".to_string()]),
            },
            Some(json!({"path": "/private/workspace/result"})),
        );
        let serialized = serde_json::to_string(&failed).unwrap();
        assert!(!failed.ok);
        assert_eq!(failed.diagnostics[0]["code"], "runtime_failed");
        assert!(!serialized.contains("/private"), "{serialized}");
        assert!(failed.artifacts.is_empty());
    }
}
