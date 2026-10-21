//! Application publication and mapping for provider-neutral diagnostics.

use super::ports::{ApplicationPorts, HandlerOutcome};
use super::AdapterOutcome;
use serde_json::{Map, Value};
use unica_configuration::operational::OperationalConfig;
use unica_diagnostics::coordinator::{
    parse_diagnostic_request, DiagnosticCoordinator, DiagnosticMapping,
};
use unica_diagnostics::model::*;
use unica_source_contract::workspace::WorkspaceContext;
use unica_work_control::cancellation::CancellationToken;

pub(crate) fn invoke(
    ports: &dyn ApplicationPorts,
    args: &Map<String, Value>,
    workspace: &WorkspaceContext,
    operational_config: Option<&OperationalConfig>,
    cancellation: &CancellationToken,
) -> Result<HandlerOutcome, String> {
    let mut request = parse_diagnostic_request(args)
        .map_err(|error| format!("{}: {}", error.code, error.message))?;
    if request.action == DiagnosticAction::Analyze {
        request.timeout = operational_config
            .ok_or_else(|| "diagnostics analyze call is missing operational config".to_string())?
            .code_diagnostics()
            .analyze_timeout();
    }
    let mapping = ApplicationDiagnosticMapping(ports);
    let result = DiagnosticCoordinator::new(ports.diagnostic_provider_registry()?, &mapping)
        .execute(&request, workspace, cancellation)
        .map_err(|error| format!("{}: {}", error.code, error.message))?;
    let ok = result.ok;
    let summary = match result.state {
        DiagnosticResultState::Completed => {
            "unica.code.diagnostics completed through provider-neutral diagnostics"
        }
        DiagnosticResultState::Partial => {
            "unica.code.diagnostics returned a partial provider-neutral result"
        }
        DiagnosticResultState::Failed => {
            "unica.code.diagnostics failed because no provider produced a useful result"
        }
    }
    .to_string();
    let warnings = (result.state == DiagnosticResultState::Partial)
        .then(|| "one or more diagnostic providers returned an incomplete result".to_string())
        .into_iter()
        .collect();
    let errors = (!ok)
        .then(|| "diagnostics_failed: no provider produced a useful result".to_string())
        .into_iter()
        .collect();
    let data = serde_json::to_value(result)
        .map_err(|error| format!("failed to serialize diagnostic result: {error}"))?;
    Ok(HandlerOutcome::with_data(
        AdapterOutcome {
            ok,
            summary,
            changes: Vec::new(),
            warnings,
            errors,
            artifacts: Vec::new(),
            stdout: None,
            stderr: None,
            command: None,
        },
        data,
    ))
}

struct ApplicationDiagnosticMapping<'a>(&'a dyn ApplicationPorts);

impl DiagnosticMapping for ApplicationDiagnosticMapping<'_> {
    fn resolve_context(
        &self,
        request: &DiagnosticRequest,
        workspace: &WorkspaceContext,
        cancellation: &CancellationToken,
    ) -> Result<DiagnosticContext, DiagnosticRequestError> {
        ApplicationPorts::resolve_diagnostic_context(self.0, request, workspace, cancellation)
    }

    fn map_observation(
        &self,
        observation: DiagnosticObservation,
        context: &DiagnosticContext,
        cancellation: &CancellationToken,
    ) -> Result<DiagnosticItem, DiagnosticMapError> {
        ApplicationPorts::map_diagnostic_observation(self.0, observation, context, cancellation)
    }

    fn map_observations(
        &self,
        observations: Vec<DiagnosticObservation>,
        context: &DiagnosticContext,
        cancellation: &CancellationToken,
    ) -> Vec<Result<DiagnosticItem, DiagnosticMapError>> {
        ApplicationPorts::map_diagnostic_observations(self.0, observations, context, cancellation)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn live_diagnostics_schema_only_names_the_shipped_provider() {
        let diagnostics_spec = crate::application::tools()
            .into_iter()
            .find(|tool| tool.name == "unica.code.diagnostics")
            .unwrap();
        let schema = crate::application::input_schema_for_tool(&diagnostics_spec);
        let schema_text = serde_json::to_string(&schema).unwrap();
        assert!(schema_text.contains("bsl-analyzer"));
        assert!(!schema_text.contains("bsl-language-server"));
        assert!(!schema_text.contains("metadata-validator"));
    }
}
