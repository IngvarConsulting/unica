use crate::application::{ApplicationPorts, CodeIntelligenceOperation, ToolHandler, ToolSpec};
use crate::domain::operational_config::{OperationalConfig, OperationalConfigDiagnostic};
use crate::domain::workspace::WorkspaceContext;
use serde_json::{Map, Value};
use std::time::Duration;

pub(crate) fn resolve_for_call(
    ports: &dyn ApplicationPorts,
    spec: ToolSpec,
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<Option<OperationalConfig>, OperationalConfigDiagnostic> {
    if !requires_snapshot(spec, args) {
        return Ok(None);
    }

    let config = ports.load_operational_config(context)?;
    let Some(seconds) = diagnostics_analyze_override(spec, args) else {
        return Ok(Some(config));
    };
    config
        .with_diagnostics_analyze_timeout(Duration::from_secs(seconds))
        .map(Some)
}

pub(crate) fn requires_snapshot(spec: ToolSpec, args: &Map<String, Value>) -> bool {
    match spec.handler {
        ToolHandler::CodeIntelligence { operation } => match operation {
            CodeIntelligenceOperation::Search
            | CodeIntelligenceOperation::Definition
            | CodeIntelligenceOperation::Outline => true,
        },
        ToolHandler::CodeAdapter { .. } if spec.name == "unica.code.diagnostics" => {
            args.get("mode")
                .and_then(Value::as_str)
                .unwrap_or("analyze")
                == "analyze"
        }
        _ => false,
    }
}

fn diagnostics_analyze_override(spec: ToolSpec, args: &Map<String, Value>) -> Option<u64> {
    (spec.name == "unica.code.diagnostics")
        .then(|| args.get("timeoutSeconds").and_then(Value::as_u64))
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::requires_snapshot;
    use crate::application::{tools, CodeIntelligenceOperation, ToolHandler};
    use serde_json::{json, Map};

    #[test]
    fn snapshot_scope_is_closed_to_code_deadline_consumers() {
        let tools = tools();
        for spec in tools {
            let expected = matches!(
                spec.handler,
                ToolHandler::CodeIntelligence {
                    operation: CodeIntelligenceOperation::Search
                        | CodeIntelligenceOperation::Definition
                        | CodeIntelligenceOperation::Outline
                }
            ) || spec.name == "unica.code.diagnostics";
            assert_eq!(
                requires_snapshot(spec, &Map::new()),
                expected,
                "{}",
                spec.name
            );
        }
    }

    #[test]
    fn non_analyze_diagnostics_modes_do_not_resolve_config() {
        let spec = tools()
            .into_iter()
            .find(|spec| spec.name == "unica.code.diagnostics")
            .unwrap();
        for mode in ["status", "catalog", "file", "workspace"] {
            let mut args = Map::new();
            args.insert("mode".to_string(), json!(mode));
            assert!(!requires_snapshot(spec, &args), "{mode}");
        }
    }
}
