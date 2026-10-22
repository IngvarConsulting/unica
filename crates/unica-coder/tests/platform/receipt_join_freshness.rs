mod tests {

    use std::time::{Duration, SystemTime};
    use unica_execution::receipt_runtime::test_support::TIMESTAMP_MARGIN;
    fn sources_unchanged_since(workspace_hint: &str, since: SystemTime) -> bool {
        unica_execution::receipt_runtime::test_support::sources_unchanged_since(
            workspace_hint,
            since,
            &crate::infrastructure::daemon::server::discover_join_workspace_root,
            &crate::infrastructure::daemon::server::discover_join_source_set_paths,
        )
    }

    const MANIFEST: &str =
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n";

    fn workspace() -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("v8project.yaml"), MANIFEST).unwrap();
        std::fs::create_dir_all(root.path().join("src/CommonModules/Общий/Ext")).unwrap();
        std::fs::write(
            root.path().join("src/CommonModules/Общий/Ext/Module.bsl"),
            "Процедура А() КонецПроцедуры\n",
        )
        .unwrap();
        root
    }

    fn hint(root: &tempfile::TempDir) -> String {
        root.path().to_string_lossy().into_owned()
    }

    /// The walk started after every file of the fixture was written: the
    /// fixture's own timestamps lie before it by more than the margin.
    fn walk_started_after_fixture() -> SystemTime {
        SystemTime::now() + TIMESTAMP_MARGIN + Duration::from_secs(1)
    }

    #[test]
    fn unchanged_sources_allow_the_join() {
        let root = workspace();
        assert!(sources_unchanged_since(
            &hint(&root),
            walk_started_after_fixture()
        ));
    }

    #[test]
    fn a_file_edited_after_the_walk_started_forbids_the_join() {
        let root = workspace();
        let since = walk_started_after_fixture();
        let module = root.path().join("src/CommonModules/Общий/Ext/Module.bsl");
        std::fs::File::options()
            .write(true)
            .open(&module)
            .unwrap()
            .set_modified(since + Duration::from_secs(1))
            .unwrap();
        assert!(!sources_unchanged_since(&hint(&root), since));
    }

    #[test]
    fn a_file_edited_within_the_margin_before_the_walk_forbids_the_join() {
        let root = workspace();
        let since = walk_started_after_fixture();
        let module = root.path().join("src/CommonModules/Общий/Ext/Module.bsl");
        std::fs::File::options()
            .write(true)
            .open(&module)
            .unwrap()
            .set_modified(since - TIMESTAMP_MARGIN / 2)
            .unwrap();
        assert!(!sources_unchanged_since(&hint(&root), since));
    }

    #[test]
    fn an_edited_project_file_forbids_the_join() {
        let root = workspace();
        let since = walk_started_after_fixture();
        std::fs::File::options()
            .write(true)
            .open(root.path().join("v8project.yaml"))
            .unwrap()
            .set_modified(since + Duration::from_secs(1))
            .unwrap();
        assert!(!sources_unchanged_since(&hint(&root), since));
    }

    #[test]
    fn the_unica_cache_directory_inside_a_source_set_is_not_a_source() {
        let root = workspace();
        let since = walk_started_after_fixture();
        let cache = root.path().join("src/.build/unica");
        std::fs::create_dir_all(&cache).unwrap();
        let index = cache.join("index.bin");
        std::fs::write(&index, b"cache").unwrap();
        std::fs::File::options()
            .write(true)
            .open(&index)
            .unwrap()
            .set_modified(since + Duration::from_secs(1))
            .unwrap();
        assert!(sources_unchanged_since(&hint(&root), since));
    }
}
