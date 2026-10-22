use super::actor_capacity_tests::{direct_v5, subsystem_picture_workspace};
use super::*;
use std::path::Path;
use unica_execution::invocation_store::ToolIdentity;

fn tree_entries(root: &Path) -> Vec<std::path::PathBuf> {
    fn visit(root: &Path, path: &Path, entries: &mut Vec<std::path::PathBuf>) {
        entries.push(path.strip_prefix(root).unwrap().to_path_buf());
        if path.is_dir() {
            for entry in std::fs::read_dir(path).unwrap() {
                visit(root, &entry.unwrap().path(), entries);
            }
        }
    }
    let mut entries = Vec::new();
    if root.exists() {
        visit(root, root, &mut entries);
    }
    entries.sort();
    entries
}

#[test]
fn canonical_apply_preserves_external_cache_authority_from_preview_to_execution() {
    const CHILD: &str = "UNICA_EXTERNAL_CACHE_APPLY_TEST_CHILD";
    const TEST: &str = "infrastructure::daemon::server::external_cache_apply_tests::canonical_apply_preserves_external_cache_authority_from_preview_to_execution";
    let Ok(mode) = std::env::var(CHILD) else {
        // Only the selected child sees this cache override. Concurrent tests
        // cannot inherit the fixture's environment.
        for mode in [
            "default",
            "external-missing",
            "external-existing",
            "relative",
        ] {
            let cache_parent = tempfile::tempdir().unwrap();
            let cache = unica_workspace::test_support::canonical_path(cache_parent.path())
                .join("missing-parent/cache");
            if mode == "external-existing" {
                std::fs::create_dir_all(cache.join("keep-empty")).unwrap();
                std::fs::write(cache.join("keep.bin"), b"foreign cache content").unwrap();
            }
            let mut child = std::process::Command::new(std::env::current_exe().unwrap());
            child
                .args(["--exact", TEST, "--nocapture"])
                .env(CHILD, mode);
            if mode == "default" {
                child.env_remove("UNICA_CACHE_DIR");
            } else if mode == "relative" {
                child
                    .current_dir(cache_parent.path())
                    .env("UNICA_CACHE_DIR", "missing-parent/cache");
            } else {
                child.env("UNICA_CACHE_DIR", &cache);
            }
            let output = child.output().unwrap();
            assert!(
                output.status.success(),
                "{mode}:\n{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
        return;
    };

    let workspace = subsystem_picture_workspace("", true);
    let root = unica_workspace::test_support::canonical_path(workspace.path());
    let cache = std::env::var_os("UNICA_CACHE_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| root.join(".build/unica"));
    let descriptor = root.join("src/Subsystems/Sales.xml");
    let bytes_before = std::fs::read(&descriptor).unwrap();
    let source_before = unica_workspace::test_support::tree_snapshot(&root);
    let cache_entries_before = tree_entries(&cache);
    let cache_before = cache
        .exists()
        .then(|| unica_workspace::test_support::tree_snapshot(&cache));
    let runtime = V5CanonicalInvocationRuntime::new(
        Arc::new(crate::infrastructure::daemon::v13_service::CanonicalV13ReadService::default()),
        Arc::new(TokioClock),
    );
    let call = |arguments| {
        let request = InvocationRequest::new(
            ToolIdentity::Apply,
            arguments,
            root.to_string_lossy(),
            7_000,
        )
        .unwrap();
        direct_v5(&runtime, request).unwrap()
    };
    let preview = call(serde_json::json!({
        "at": "main:Subsystem.Sales",
        "ops": [{"op":"props.set", "args":{"values":{"Comment":"external cache"}}}]
    }));
    assert!(preview.ok, "{mode} preview: {preview:?}");
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&root),
        source_before
    );
    assert_eq!(tree_entries(&cache), cache_entries_before);
    assert_eq!(
        cache
            .exists()
            .then(|| unica_workspace::test_support::tree_snapshot(&cache)),
        cache_before
    );
    let token = &preview.data.as_ref().unwrap()["executionToken"];
    assert!(token.is_string(), "{preview:?}");
    let published = call(serde_json::json!({"executionToken":token}));
    assert!(published.ok, "{mode} execution: {published:?}");
    let expected = String::from_utf8(bytes_before)
        .unwrap()
        .replace("<Comment/>", "<Comment>external cache</Comment>");
    assert_eq!(std::fs::read(&descriptor).unwrap(), expected.as_bytes());
    for key in ["planHash", "operations", "effects"] {
        assert_eq!(
            preview.data.as_ref().unwrap()[key],
            published.data.as_ref().unwrap()[key]
        );
    }
    assert!(tree_entries(&cache)
        .iter()
        .any(|path| path.file_name().is_some_and(|name| name == "state.json")));
    if mode != "default" {
        assert!(
            !root.join(".build/unica").exists(),
            "override silently fell back to default"
        );
    }
    if mode == "external-existing" {
        assert_eq!(
            std::fs::read(cache.join("keep.bin")).unwrap(),
            b"foreign cache content"
        );
        assert!(cache.join("keep-empty").is_dir());
    }
    // The existing runner projection store must still select this cache after
    // apply created it; this opens the real retained store without a provider.
    let projection =
        super::super::runner_015::lock_project(&root, &CancellationToken::new()).unwrap();
    let cache_identity = unica_workspace::paths::normalize_path_identity(&cache).unwrap();
    assert!(projection
        .directory()
        .starts_with(cache_identity.join("runner-project")));
    drop(projection);
    let after = unica_workspace::test_support::tree_snapshot(&root);
    let replayed = call(serde_json::json!({"executionToken":token}));
    assert!(replayed.ok, "{replayed:?}");
    assert_eq!(unica_workspace::test_support::tree_snapshot(&root), after);
}
