use serde_json::{Map, Value};
use unica_runner::admission::{runtime_risk_notice, RuntimeRiskOutcome};
#[test]
fn canonical_runtime_surface_has_an_explicit_risk_classification() {
    for operation in super::tool_contracts::RUNTIME_OPERATIONS {
        let mut modes: Vec<Option<&str>> = vec![None];
        if *operation == "syntax" {
            modes = super::tool_contracts::RUNTIME_SYNTAX_MODES
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

            let outcome = runtime_risk_notice("unica.runtime.execute", &args)
                .expect("canonical runtime operation is valid");
            assert!(
                !matches!(outcome, RuntimeRiskOutcome::Refused(_)),
                "{operation} {mode:?} needs a reviewed risk classification"
            );
        }
    }
}
