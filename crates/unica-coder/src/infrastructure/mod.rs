pub(crate) mod bundled_tools;
pub mod internal_adapters;
pub mod native_operations;
pub mod path_policy;
pub mod plugin_runtime;
pub(crate) mod shadow_dump;
pub mod source_sync;
pub mod workspace_index;
pub mod workspace_services;
pub mod workspace_state;

use serde::Serialize;

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
}
