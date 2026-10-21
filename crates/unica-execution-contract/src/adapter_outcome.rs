use serde::Serialize;
use unica_work_control::cancellation::cancelled_error;

#[derive(Debug, Clone, Serialize)]
pub struct AdapterOutcome {
    pub ok: bool,
    pub summary: String,
    pub changes: Vec<String>,
    pub warnings: Vec<String>,
    pub errors: Vec<String>,
    pub artifacts: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stdout: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stderr: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<Vec<String>>,
}

impl AdapterOutcome {
    pub fn ok(summary: impl Into<String>) -> Self {
        Self {
            ok: true,
            summary: summary.into(),
            changes: Vec::new(),
            warnings: Vec::new(),
            errors: Vec::new(),
            artifacts: Vec::new(),
            stdout: None,
            stderr: None,
            command: None,
        }
    }

    pub fn cancelled(detail: impl AsRef<str>) -> Self {
        let error = cancelled_error(detail);
        Self {
            ok: false,
            summary: error.clone(),
            changes: Vec::new(),
            warnings: Vec::new(),
            errors: vec![error],
            artifacts: Vec::new(),
            stdout: None,
            stderr: None,
            command: None,
        }
    }
}

pub fn cancelled_process_outcome(
    tool_name: &str,
    stdout: String,
    stderr: String,
    command: Option<Vec<String>>,
) -> AdapterOutcome {
    let mut outcome = AdapterOutcome::cancelled(format!("{tool_name} process stopped"));
    outcome.stdout = Some(stdout);
    outcome.stderr = Some(stderr);
    outcome.command = command;
    outcome
}

pub fn merge_warnings(mut left: Vec<String>, right: Vec<String>) -> Vec<String> {
    left.extend(right);
    left
}
