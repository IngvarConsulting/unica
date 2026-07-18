use std::path::{Path, PathBuf};

use unica_coder::domain::project_sources::{
    discover_project_source_map, ProjectSourceMap, ProjectSourceSet,
};
use unica_coder::domain::source_roots::{
    normalize_path_identity, resolve_source_root, select_default_source_set, ResolvedSourceRoot,
};

#[test]
fn source_root_compatibility_paths_remain_public() {
    let _discover: fn(&Path) -> Result<ProjectSourceMap, String> = discover_project_source_map;
    let _normalize: fn(&Path) -> Result<PathBuf, String> = normalize_path_identity;
    let _resolved = ResolvedSourceRoot {
        source_set: None,
        path: PathBuf::new(),
    };
    let no_source_sets: &[ProjectSourceSet] = &[];
    assert!(select_default_source_set(no_source_sets).is_err());

    fn accepts_resolver<F>(_resolver: F) {}
    accepts_resolver(resolve_source_root);
}
