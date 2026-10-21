//! Discovery and the actual named-root resolver must agree on physical identity.
use super::project_sources::discover_project_source_map;
use super::source_roots::resolve_source_root;
use super::workspace::discover_workspace;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};
static TEMP_WORKSPACE_NONCE: AtomicU64 = AtomicU64::new(0);

fn temp_workspace(prefix: &str) -> PathBuf {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let nonce = TEMP_WORKSPACE_NONCE.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!(
        "{prefix}-{}-{timestamp}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    root
}

fn write(path: &Path, text: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, text).unwrap();
}
#[test]
fn effective_source_root_uses_resolver_path_identity() {
    let root = temp_workspace("unica-source-map-normalized");
    write(
        &root.join("v8project.yaml"),
        "source-set:\n  - name: main\n    type: CONFIGURATION\n    path: src/../src/cf\n",
    );
    fs::create_dir_all(root.join("src/cf")).unwrap();
    let context = discover_workspace(Some(root.clone())).unwrap();

    let map = discover_project_source_map(&root).unwrap();
    let resolved = resolve_source_root(&context, None).unwrap();

    assert_eq!(map.effective_source_set.as_deref(), Some("main"));
    assert_eq!(
        map.effective_source_root.as_deref(),
        Some(resolved.path.to_string_lossy().as_ref())
    );
    fs::remove_dir_all(root).unwrap();
}
