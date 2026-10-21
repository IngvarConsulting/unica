//! Trusted construction of the shared Designer discovery and workspace readers.
use std::path::Path;
use unica_source_contract::ProjectSourceMap;
use unica_workspace::discovery::{ProjectDiscoveryReader, ProjectSourceMapProvenance};

pub(crate) fn discover_project_source_map(
    workspace_root: &Path,
) -> Result<ProjectSourceMap, String> {
    unica_source_designer::project_sources::discover_project_source_map(
        &ProjectDiscoveryReader::new(workspace_root),
    )
}
pub(crate) fn discover_project_source_map_controlled(
    workspace_root: &Path,
    checkpoint: &mut dyn FnMut() -> Result<(), String>,
) -> Result<ProjectSourceMap, String> {
    unica_source_designer::project_sources::discover_project_source_map_controlled(
        &ProjectDiscoveryReader::new(workspace_root),
        checkpoint,
    )
}
pub(crate) fn discover_project_source_map_with_provenance(
    workspace_root: &Path,
) -> Result<(ProjectSourceMap, ProjectSourceMapProvenance), String> {
    let reader = ProjectDiscoveryReader::new(workspace_root);
    let mut provenance = ProjectSourceMapProvenance::default();
    let map = unica_source_designer::project_sources::discover_project_source_map_with_provenance(
        &reader,
        &mut provenance,
    )?;
    Ok((map, provenance))
}
