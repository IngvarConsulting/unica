//! Exact mutation-result facts shared by writing tools.
use std::path::Path;
/// Typed answer shared by the writing tools (ADR-0023). Each tool builds it
/// from the paths it actually touched, never by re-reading its own report: the
/// point of the decision is to stop parsing prose, including our own.
#[derive(Debug, serde::Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MutationData {
    /// `false` when the call was a preview and nothing was written.
    pub applied: bool,
    pub created: Vec<String>,
    pub updated: Vec<String>,
    pub removed: Vec<String>,
}
impl MutationData {
    pub fn new(applied: bool) -> Self {
        Self {
            applied,
            ..Default::default()
        }
    }

    pub fn created(mut self, path: &Path) -> Self {
        self.created.push(path.display().to_string());
        self
    }

    pub fn updated(mut self, path: &Path) -> Self {
        self.updated.push(path.display().to_string());
        self
    }

    pub fn removed(mut self, path: &Path) -> Self {
        self.removed.push(path.display().to_string());
        self
    }
}
