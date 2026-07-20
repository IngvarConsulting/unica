use crate::domain::cancellation::cancelled_error;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub(crate) struct AdapterOutcome {
    pub(crate) ok: bool,
    pub(crate) summary: String,
    pub(crate) changes: Vec<String>,
    pub(crate) warnings: Vec<String>,
    pub(crate) errors: Vec<String>,
    pub(crate) artifacts: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) stdout: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) stderr: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) command: Option<Vec<String>>,
}

impl AdapterOutcome {
    pub(crate) fn ok(summary: impl Into<String>) -> Self {
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

    pub(crate) fn cancelled(detail: impl AsRef<str>) -> Self {
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
