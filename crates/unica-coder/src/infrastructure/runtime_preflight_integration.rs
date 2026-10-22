#[cfg(test)]
mod tests {
    use crate::infrastructure::runtime_composition::workspace_observer;
    use crate::infrastructure::workspace::discover_workspace;

    use unica_adapter_v8_runner::preflight::*;
    use unica_source_contract::workspace::WorkspaceContext;
    use unica_workspace::paths::normalize_path_identity;

    use serde_json::{json, Map};
    use std::fs;
    use tempfile::TempDir;
    use unica_workspace::test_support::{
        create_file_link_fixture_for_test, FileLinkFixtureOutcome,
    };

    fn workspace() -> (TempDir, WorkspaceContext) {
        let directory = tempfile::tempdir().unwrap();
        fs::write(
            directory.path().join("v8project.yaml"),
            "format: DESIGNER\nsource-set: []\n",
        )
        .unwrap();
        let context = discover_workspace(Some(directory.path().to_path_buf())).unwrap();
        (directory, context)
    }

    #[test]
    fn default_build_is_not_forced_full_and_does_not_read_support() {
        let (_directory, context) = workspace();
        let args = Map::from_iter([("operation".to_string(), json!("build"))]);

        let plan = plan_runtime_invocation(&args, &context, workspace_observer()).unwrap();

        assert_eq!(plan.args.get("fullRebuild"), None);
        assert!(plan.build_preflight.is_some());
        assert!(plan.warnings[0].contains("retry once"));
    }

    #[test]
    fn explicit_full_build_has_no_fallback_preflight() {
        let (_directory, context) = workspace();
        let args = Map::from_iter([
            ("operation".to_string(), json!("build")),
            ("fullRebuild".to_string(), json!(true)),
        ]);

        let plan = plan_runtime_invocation(&args, &context, workspace_observer()).unwrap();

        assert_eq!(plan.args.get("fullRebuild"), Some(&json!(true)));
        assert!(plan.build_preflight.is_none());
        assert!(plan.warnings.is_empty());
    }

    #[test]
    fn build_preflight_rejects_a_changed_config_without_reading_support() {
        let (_directory, context) = workspace();
        let args = Map::from_iter([("operation".to_string(), json!("build"))]);
        let preflight =
            RuntimeBuildPreflight::capture(&args, &context, workspace_observer()).unwrap();
        fs::write(
            context.workspace_root.join("v8project.yaml"),
            "format: EDT\nsource-set: []\n",
        )
        .unwrap();

        let error = preflight
            .reauthorize_current_workspace(workspace_observer())
            .expect_err("changed config must fail closed");

        assert!(error.contains("identity changed") || error.contains("config changed"));
    }

    #[test]
    fn build_preflight_rejects_a_changed_local_config_overlay() {
        let (_directory, context) = workspace();
        let local_config = context.workspace_root.join("v8project.local.yaml");
        fs::write(&local_config, "workPath: first\n").unwrap();
        let args = Map::from_iter([("operation".to_string(), json!("build"))]);
        let preflight =
            RuntimeBuildPreflight::capture(&args, &context, workspace_observer()).unwrap();
        preflight
            .reauthorize_current_workspace(workspace_observer())
            .expect("an unchanged local overlay must stay authorized");
        fs::write(&local_config, "workPath: other\n").unwrap();

        let error = preflight
            .reauthorize_current_workspace(workspace_observer())
            .expect_err("a changed local overlay must invalidate fallback authorization");

        assert!(error.contains("local project config changed"), "{error}");
    }

    #[test]
    fn build_preflight_rejects_a_created_or_removed_local_config_overlay() {
        let (_directory, context) = workspace();
        let local_config = context.workspace_root.join("v8project.local.yaml");
        let args = Map::from_iter([("operation".to_string(), json!("build"))]);
        let absent_preflight =
            RuntimeBuildPreflight::capture(&args, &context, workspace_observer()).unwrap();
        fs::write(&local_config, "workPath: local\n").unwrap();

        let created_error = absent_preflight
            .reauthorize_current_workspace(workspace_observer())
            .expect_err("a new local overlay must invalidate fallback authorization");
        assert!(
            created_error.contains("local project config changed"),
            "{created_error}"
        );

        let present_preflight =
            RuntimeBuildPreflight::capture(&args, &context, workspace_observer()).unwrap();
        fs::remove_file(&local_config).unwrap();

        let removed_error = present_preflight
            .reauthorize_current_workspace(workspace_observer())
            .expect_err("a removed local overlay must invalidate fallback authorization");
        assert!(
            removed_error.contains("local project config changed"),
            "{removed_error}"
        );
    }

    #[test]
    fn build_preflight_rejects_a_non_regular_local_config_overlay() {
        let (_directory, context) = workspace();
        fs::create_dir(context.workspace_root.join("v8project.local.yaml")).unwrap();
        let args = Map::from_iter([("operation".to_string(), json!("build"))]);

        let error = RuntimeBuildPreflight::capture(&args, &context, workspace_observer())
            .expect_err("a directory must not become a runtime local config overlay");

        assert!(error.contains("regular file"), "{error}");
    }

    #[test]
    fn build_preflight_rejects_a_linked_local_config_overlay() {
        let (_directory, context) = workspace();
        let outside = tempfile::NamedTempFile::new().unwrap();
        let outcome = create_file_link_fixture_for_test(
            outside.path(),
            context.workspace_root.join("v8project.local.yaml"),
        )
        .expect("unexpected file-link fixture error must fail the test");
        match outcome {
            FileLinkFixtureOutcome::Created => {}
            FileLinkFixtureOutcome::Unsupported
            | FileLinkFixtureOutcome::WindowsPrivilegeUnavailable => {
                unica_process::test_support::skip_without_host_capability(
                    unica_process::test_support::HostCapability::Symlink,
                    "creating a link fixture needs the symlink privilege",
                );
                return;
            }
        }
        let args = Map::from_iter([("operation".to_string(), json!("build"))]);

        let error = RuntimeBuildPreflight::capture(&args, &context, workspace_observer())
            .expect_err("a linked local overlay must not escape the workspace binding");

        assert!(error.contains("regular file"), "{error}");
    }

    #[test]
    fn build_preflight_rejects_a_changed_workspace_epoch() {
        let (directory, _) = workspace();
        fs::create_dir_all(directory.path().join(".git")).unwrap();
        let head = directory.path().join(".git/HEAD");
        fs::write(&head, "ref: refs/heads/feature-a\n").unwrap();
        let context = discover_workspace(Some(directory.path().to_path_buf())).unwrap();
        let args = Map::from_iter([("operation".to_string(), json!("build"))]);
        let preflight =
            RuntimeBuildPreflight::capture(&args, &context, workspace_observer()).unwrap();
        fs::write(&head, "ref: refs/heads/feature-b\n").unwrap();

        let error = preflight
            .reauthorize_current_workspace(workspace_observer())
            .expect_err("a changed workspace epoch must invalidate fallback authorization");

        assert!(error.contains("workspace identity changed"), "{error}");
    }

    #[test]
    fn build_preflight_rejects_a_config_outside_the_workspace() {
        let (_directory, context) = workspace();
        let outside = tempfile::tempdir().unwrap();
        let outside_config = outside.path().join("v8project.yaml");
        fs::write(&outside_config, "format: DESIGNER\nsource-set: []\n").unwrap();
        let args = Map::from_iter([
            ("operation".to_string(), json!("build")),
            (
                "config".to_string(),
                json!(outside_config.to_string_lossy().into_owned()),
            ),
        ]);

        let error = match plan_runtime_invocation(&args, &context, workspace_observer()) {
            Ok(_) => panic!("an external project config must not escape the workspace lock"),
            Err(error) => error,
        };

        assert!(error.contains("outside the workspace"), "{error}");
    }

    #[test]
    fn build_preflight_accepts_an_alternate_config_inside_the_workspace() {
        let (_directory, context) = workspace();
        let alternate = context.workspace_root.join("configs/alternate.yaml");
        fs::create_dir_all(alternate.parent().unwrap()).unwrap();
        fs::write(&alternate, "format: DESIGNER\nsource-set: []\n").unwrap();
        let args = Map::from_iter([
            ("operation".to_string(), json!("build")),
            ("config".to_string(), json!("configs/alternate.yaml")),
        ]);

        let plan = plan_runtime_invocation(&args, &context, workspace_observer()).unwrap();

        assert_eq!(
            plan.args.get("config"),
            Some(&json!(normalize_path_identity(&alternate)
                .unwrap()
                .to_string_lossy()
                .into_owned()))
        );
        assert!(plan.build_preflight.is_some());
    }

    #[test]
    fn build_preflight_binds_the_local_overlay_next_to_an_alternate_config() {
        let (_directory, context) = workspace();
        let alternate = context.workspace_root.join("configs/alternate.yaml");
        let local_config = alternate.parent().unwrap().join("v8project.local.yaml");
        fs::create_dir_all(alternate.parent().unwrap()).unwrap();
        fs::write(&alternate, "format: DESIGNER\nsource-set: []\n").unwrap();
        fs::write(&local_config, "workPath: first\n").unwrap();
        let args = Map::from_iter([
            ("operation".to_string(), json!("build")),
            ("config".to_string(), json!("configs/alternate.yaml")),
        ]);
        let preflight =
            RuntimeBuildPreflight::capture(&args, &context, workspace_observer()).unwrap();
        fs::write(&local_config, "workPath: other\n").unwrap();

        let error = preflight
            .reauthorize_current_workspace(workspace_observer())
            .expect_err("the selected config's sibling overlay must stay bound");

        assert!(error.contains("local project config changed"), "{error}");
    }

    #[test]
    fn build_preflight_rejects_a_non_regular_config() {
        let (_directory, context) = workspace();
        let directory_path = context.workspace_root.join("runtime-config-directory");
        fs::create_dir(&directory_path).unwrap();
        let args = Map::from_iter([
            ("operation".to_string(), json!("build")),
            (
                "config".to_string(),
                json!(directory_path.to_string_lossy().into_owned()),
            ),
        ]);

        let error = match plan_runtime_invocation(&args, &context, workspace_observer()) {
            Ok(_) => panic!("a directory must not become a runtime project config"),
            Err(error) => error,
        };

        assert!(error.contains("regular file"), "{error}");
    }
}
