use serde::{Deserialize, Serialize};

/// Transport-facing view of the project source topology. Filesystem and YAML
/// discovery deliberately live in infrastructure.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSourceMap {
    pub workspace_root: String,
    pub config_path: Option<String>,
    pub source_sets: Vec<ProjectSourceSet>,
    #[serde(skip_serializing)]
    pub(crate) configured_format_raw: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSourceSet {
    pub name: String,
    pub kind: SourceSetKind,
    pub path: String,
    pub source_format: SourceFormat,
    pub format_evidence: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceSetKind {
    Configuration,
    Extension,
    ExternalProcessor,
    ExternalReport,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceFormat {
    PlatformXml,
    Edt,
    Unknown,
    Invalid,
}
