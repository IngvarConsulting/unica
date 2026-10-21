use super::{inspect_project_health_with_runner, SourceRootOwnerIndex};
use crate::{ProjectCheckId, ProjectCheckOutcome, ProjectHealthFact};
use unica_source_contract::{ProjectSourceSet, SourceFormat, SourceSetKind};
use unica_work_control::cancellation::CancellationToken;
use unica_work_control::provider_deadline::ProviderDeadline;

use std::cell::RefCell;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tempfile::TempDir;
use unica_process::runner::{ProcessCommand, ProcessOutput, ProcessRunner};
use unica_source_contract::workspace::WorkspaceContext;

fn cancel_before_snapshot<T>(action: impl FnOnce() -> T) -> T {
    struct Reset(bool);

    impl Drop for Reset {
        fn drop(&mut self) {
            super::CANCEL_BEFORE_SNAPSHOT.with(|slot| slot.set(self.0));
        }
    }

    let previous = super::CANCEL_BEFORE_SNAPSHOT.with(|slot| slot.replace(true));
    let _reset = Reset(previous);
    action()
}

#[test]
fn final_infrastructure_cancellation_wins_before_snapshot_return() {
    let temp = TempDir::new().unwrap();
    fs::create_dir_all(temp.path().join("src")).unwrap();
    fs::write(
        temp.path().join("src/Configuration.xml"),
        "<MetaDataObject/>",
    )
    .unwrap();
    fs::write(
        temp.path().join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
    )
    .unwrap();
    let context = WorkspaceContext {
        cwd: temp.path().to_path_buf(),
        workspace_root: temp.path().to_path_buf(),
        cache_root: temp.path().join(".build/unica"),
        workspace_epoch: 1,
    };
    let cancellation = CancellationToken::new();
    let runner = RecordingRunner {
        repository_root: temp.path().to_path_buf(),
        commands: RefCell::new(Vec::new()),
    };

    let result = cancel_before_snapshot(|| {
        inspect_project_health_with_runner(
            &context,
            &cancellation,
            ProviderDeadline::from_budget(Duration::from_secs(2)),
            &runner,
        )
    });

    assert!(matches!(
        result,
        Err(crate::ProjectHealthInspectionError::Cancelled)
    ));
}

#[test]
fn source_owner_index_keeps_repository_root_and_equal_root_ambiguity() {
    let repository_root = Path::new("/repository");
    let root = |name: &str| crate::inspection::layout::InspectedSourceRoot {
        source_set: ProjectSourceSet {
            name: name.into(),
            kind: SourceSetKind::Configuration,
            path: ".".into(),
            source_format: SourceFormat::PlatformXml,
            source_state: unica_source_contract::SourceSetState::Supported,
            format_evidence: Vec::new(),
            format_probe_error: None,
        },
        path: repository_root.to_path_buf(),
    };
    let roots = [root("first"), root("second")];
    let one = SourceRootOwnerIndex::new_with_case_policy(
        repository_root,
        roots.iter().take(1),
        &mut || Ok::<_, ()>(()),
        &|_| Ok(true),
    )
    .unwrap();
    let one_owners = one
        .deepest_owners_with_checkpoint("ConfigDumpInfo.xml", &mut || Ok::<_, ()>(()))
        .unwrap()
        .unwrap();
    assert_eq!(one_owners.0[0].source_set.name, "first");

    let ambiguous = SourceRootOwnerIndex::new_with_case_policy(
        repository_root,
        &roots,
        &mut || Ok::<_, ()>(()),
        &|_| Ok(true),
    )
    .unwrap();
    let owners = ambiguous
        .deepest_owners_with_checkpoint("ConfigDumpInfo.xml", &mut || Ok::<_, ()>(()))
        .unwrap()
        .unwrap();
    assert_eq!(owners.0.len(), 2);
}

#[test]
fn source_owner_index_uses_the_host_volume_case_policy() {
    let temp = TempDir::new().unwrap();
    let repository_root = temp.path();
    fs::create_dir(repository_root.join("src")).unwrap();
    let repository_identity =
        unica_workspace::paths::normalize_path_identity(repository_root).unwrap();
    if unica_workspace::paths::host_filesystem_case_sensitive(&repository_identity).unwrap() {
        return;
    }
    let root = crate::inspection::layout::InspectedSourceRoot {
        source_set: ProjectSourceSet {
            name: "main".into(),
            kind: SourceSetKind::Configuration,
            path: "src".into(),
            source_format: SourceFormat::PlatformXml,
            source_state: unica_source_contract::SourceSetState::Supported,
            format_evidence: Vec::new(),
            format_probe_error: None,
        },
        path: repository_root.join("src"),
    };
    let index = SourceRootOwnerIndex::new(repository_root, [&root]).unwrap();

    let owners = index
        .deepest_owners_with_checkpoint("SRC/Hidden.xml", &mut || Ok::<_, ()>(()))
        .unwrap()
        .unwrap();

    assert_eq!(owners.0[0].source_set.name, "main");
}

#[test]
fn source_owner_index_applies_case_policy_per_parent_directory() {
    let temp = TempDir::new().unwrap();
    let repository_root = temp.path();
    let repository_identity =
        unica_workspace::paths::normalize_path_identity(repository_root).unwrap();
    if !unica_workspace::paths::host_filesystem_case_sensitive(&repository_identity).unwrap() {
        return;
    }
    fs::create_dir_all(repository_root.join("modules")).unwrap();
    let root = crate::inspection::layout::InspectedSourceRoot {
        source_set: ProjectSourceSet {
            name: "upper".into(),
            kind: SourceSetKind::Configuration,
            path: "modules/MISSING/SRC".into(),
            source_format: SourceFormat::PlatformXml,
            source_state: unica_source_contract::SourceSetState::Supported,
            format_evidence: Vec::new(),
            format_probe_error: None,
        },
        path: repository_root.join("modules/MISSING/SRC"),
    };
    let index = SourceRootOwnerIndex::new_with_case_policy(
        repository_root,
        [&root],
        &mut || Ok::<_, ()>(()),
        &|path| Ok(path.ends_with("modules")),
    )
    .unwrap();

    assert!(index
        .deepest_owners_with_checkpoint("modules/missing/SRC/Hidden.xml", &mut || Ok::<_, ()>(()),)
        .unwrap()
        .is_none());
    assert_eq!(
        index
            .deepest_owners_with_checkpoint("modules/MISSING/SRC/Hidden.xml", &mut || Ok::<_, ()>(
                ()
            ),)
            .unwrap()
            .unwrap()
            .0[0]
            .source_set
            .name,
        "upper"
    );
}

#[test]
fn source_owner_index_fails_closed_when_case_policy_is_unavailable() {
    let repository_root = Path::new("/repository");
    let root = crate::inspection::layout::InspectedSourceRoot {
        source_set: ProjectSourceSet {
            name: "main".into(),
            kind: SourceSetKind::Configuration,
            path: "src".into(),
            source_format: SourceFormat::PlatformXml,
            source_state: unica_source_contract::SourceSetState::Supported,
            format_evidence: Vec::new(),
            format_probe_error: None,
        },
        path: repository_root.join("src"),
    };

    let result = SourceRootOwnerIndex::new_with_case_policy(
        repository_root,
        [&root],
        &mut || Ok::<_, ()>(()),
        &|_| {
            Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "case policy denied",
            ))
        },
    );

    assert!(matches!(
        result,
        Err(super::SourceRootOwnerIndexError::Path(reason))
            if reason.contains("failed to determine filesystem path identity")
                && reason.contains("case policy denied")
    ));
}

#[test]
fn source_owner_index_inherits_case_policy_for_a_missing_declared_root() {
    let temp = TempDir::new().unwrap();
    let repository_root = temp.path();
    let root = crate::inspection::layout::InspectedSourceRoot {
        source_set: ProjectSourceSet {
            name: "main".into(),
            kind: SourceSetKind::Configuration,
            path: "missing/nested".into(),
            source_format: SourceFormat::PlatformXml,
            source_state: unica_source_contract::SourceSetState::Supported,
            format_evidence: Vec::new(),
            format_probe_error: None,
        },
        path: repository_root.join("missing/nested"),
    };

    let index = SourceRootOwnerIndex::new(repository_root, [&root]).unwrap();
    let owners = index
        .deepest_owners_with_checkpoint("missing/nested/Configuration.xml", &mut || Ok::<_, ()>(()))
        .unwrap()
        .unwrap();

    assert_eq!(owners.0[0].source_set.name, "main");
}

#[test]
fn failed_index_does_not_run_or_pass_dependent_resource_checks() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().to_path_buf();
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("src/Configuration.xml"), "<MetaDataObject/>").unwrap();
    fs::write(
        root.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
    )
    .unwrap();
    let context = WorkspaceContext {
        cwd: root.clone(),
        workspace_root: root.clone(),
        cache_root: root.join(".build/unica"),
        workspace_epoch: 1,
    };
    let runner = RecordingRunner {
        repository_root: root,
        commands: RefCell::new(Vec::new()),
    };

    let snapshot = inspect_project_health_with_runner(
        &context,
        &CancellationToken::new(),
        ProviderDeadline::from_budget(Duration::from_secs(2)),
        &runner,
    )
    .unwrap();

    assert_eq!(runner.commands.borrow().len(), 2, "{:?}", runner.commands);
    assert!(snapshot.facts.iter().any(|fact| matches!(
        fact,
        ProjectHealthFact::GitInspectionIncomplete {
            check: ProjectCheckId::RepositoryIndex,
            ..
        }
    )));
    for check in [
        ProjectCheckId::RepositoryAttributes,
        ProjectCheckId::RepositoryIndexEol,
        ProjectCheckId::RepositoryWorkingEol,
        ProjectCheckId::RepositoryLfs,
    ] {
        assert!(snapshot.observations.iter().any(|observation| {
            observation.id == check
                && matches!(observation.outcome, ProjectCheckOutcome::NotRun { .. })
        }));
    }
}

#[test]
fn incomplete_source_targets_do_not_pass_source_dependent_repository_checks() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().to_path_buf();
    for path in ["src/a", "src/b"] {
        fs::create_dir_all(root.join(path)).unwrap();
        fs::write(
            root.join(path).join("Configuration.xml"),
            "<MetaDataObject/>",
        )
        .unwrap();
    }
    fs::write(
        root.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src/a\n  - name: main\n    type: CONFIGURATION\n    path: src/b\n",
    )
    .unwrap();
    let context = WorkspaceContext {
        cwd: root.clone(),
        workspace_root: root.clone(),
        cache_root: root.join(".build/unica"),
        workspace_epoch: 1,
    };
    let runner = RecordingRunner {
        repository_root: root,
        commands: RefCell::new(Vec::new()),
    };

    let snapshot = inspect_project_health_with_runner(
        &context,
        &CancellationToken::new(),
        ProviderDeadline::from_budget(Duration::from_secs(2)),
        &runner,
    )
    .unwrap();

    assert!(!snapshot.source_targets_complete);
    assert_eq!(runner.commands.borrow().len(), 2, "{:?}", runner.commands);
    for check in [
        ProjectCheckId::RepositoryIgnore,
        ProjectCheckId::RepositoryGeneratedPaths,
        ProjectCheckId::RepositoryConfigDumpInfo,
        ProjectCheckId::RepositoryAttributes,
        ProjectCheckId::RepositoryIndexEol,
        ProjectCheckId::RepositoryWorkingEol,
        ProjectCheckId::RepositoryLfs,
    ] {
        assert!(
            snapshot.observations.iter().any(|observation| {
                observation.id == check
                    && observation.source_set.is_none()
                    && matches!(observation.outcome, ProjectCheckOutcome::NotRun { .. })
            }),
            "{check:?}: {:?}",
            snapshot.observations
        );
        assert!(
            !snapshot.observations.iter().any(|observation| {
                observation.id == check
                    && matches!(observation.outcome, ProjectCheckOutcome::Completed)
            }),
            "{check:?}: {:?}",
            snapshot.observations
        );
    }
}

#[test]
fn edt_without_git_marks_platform_xml_policy_checks_not_applicable() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().to_path_buf();
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("src/.project"), "<projectDescription/>").unwrap();
    fs::write(
        root.join("v8project.yaml"),
        "format: EDT\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
    )
    .unwrap();
    let context = WorkspaceContext {
        cwd: root.clone(),
        workspace_root: root.clone(),
        cache_root: root.join(".build/unica"),
        workspace_epoch: 1,
    };

    let snapshot = inspect_project_health_with_runner(
        &context,
        &CancellationToken::new(),
        ProviderDeadline::from_budget(Duration::from_secs(2)),
        &NoGitRunner,
    )
    .unwrap();

    assert!(
        snapshot.observations.iter().all(|observation| {
            observation.id != ProjectCheckId::RepositoryIndex || observation.source_set.is_none()
        }),
        "repository.index is repository-wide: {:?}",
        snapshot.observations
    );

    for check in [
        ProjectCheckId::RepositoryAttributes,
        ProjectCheckId::RepositoryIndexEol,
        ProjectCheckId::RepositoryWorkingEol,
        ProjectCheckId::RepositoryLfs,
    ] {
        assert!(
            snapshot.observations.iter().any(|observation| {
                observation.id == check
                    && observation.source_set.as_deref() == Some("main")
                    && matches!(
                        observation.outcome,
                        ProjectCheckOutcome::NotApplicable { .. }
                    )
            }),
            "{check:?}: {:?}",
            snapshot.observations
        );
    }
}

#[test]
fn mixed_source_sets_without_git_publish_complete_per_set_repository_matrix() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().to_path_buf();
    fs::create_dir_all(root.join("designer")).unwrap();
    fs::create_dir_all(root.join("edt")).unwrap();
    fs::write(root.join("designer/Configuration.xml"), "<MetaDataObject/>").unwrap();
    fs::write(root.join("edt/.project"), "<projectDescription/>").unwrap();
    fs::write(
        root.join("v8project.yaml"),
        "source-set:\n  - name: designer\n    type: CONFIGURATION\n    path: designer\n  - name: edt\n    type: CONFIGURATION\n    path: edt\n",
    )
    .unwrap();
    let context = WorkspaceContext {
        cwd: root.clone(),
        workspace_root: root.clone(),
        cache_root: root.join(".build/unica"),
        workspace_epoch: 1,
    };

    let snapshot = inspect_project_health_with_runner(
        &context,
        &CancellationToken::new(),
        ProviderDeadline::from_budget(Duration::from_secs(2)),
        &NoGitRunner,
    )
    .unwrap();

    for check in [
        ProjectCheckId::RepositoryIgnore,
        ProjectCheckId::RepositoryGeneratedPaths,
        ProjectCheckId::RepositoryConfigDumpInfo,
    ] {
        for source_set in ["designer", "edt"] {
            assert!(
                snapshot.observations.iter().any(|observation| {
                    observation.id == check
                        && observation.source_set.as_deref() == Some(source_set)
                        && matches!(observation.outcome, ProjectCheckOutcome::NotRun { .. })
                }),
                "{check:?}/{source_set}: {:?}",
                snapshot.observations
            );
        }
    }
    for check in [
        ProjectCheckId::RepositoryAttributes,
        ProjectCheckId::RepositoryIndexEol,
        ProjectCheckId::RepositoryWorkingEol,
        ProjectCheckId::RepositoryLfs,
    ] {
        assert!(
            snapshot.observations.iter().any(|observation| {
                observation.id == check
                    && observation.source_set.as_deref() == Some("designer")
                    && matches!(observation.outcome, ProjectCheckOutcome::NotRun { .. })
            }),
            "{check:?}/designer: {:?}",
            snapshot.observations
        );
        assert!(
            snapshot.observations.iter().any(|observation| {
                observation.id == check
                    && observation.source_set.as_deref() == Some("edt")
                    && matches!(
                        observation.outcome,
                        ProjectCheckOutcome::NotApplicable { .. }
                    )
            }),
            "{check:?}/edt: {:?}",
            snapshot.observations
        );
    }
}

#[test]
fn known_unsafe_source_set_without_git_still_has_a_complete_repository_matrix() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().to_path_buf();
    fs::write(root.join("src"), "not a directory").unwrap();
    fs::write(
        root.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
    )
    .unwrap();
    let context = WorkspaceContext {
        cwd: root.clone(),
        workspace_root: root.clone(),
        cache_root: root.join(".build/unica"),
        workspace_epoch: 1,
    };

    let snapshot = inspect_project_health_with_runner(
        &context,
        &CancellationToken::new(),
        ProviderDeadline::from_budget(Duration::from_secs(2)),
        &NoGitRunner,
    )
    .unwrap();

    assert_eq!(snapshot.source_sets.as_ref().unwrap()[0].name, "main");
    for check in [
        ProjectCheckId::RepositoryIgnore,
        ProjectCheckId::RepositoryGeneratedPaths,
        ProjectCheckId::RepositoryConfigDumpInfo,
        ProjectCheckId::RepositoryAttributes,
        ProjectCheckId::RepositoryIndexEol,
        ProjectCheckId::RepositoryWorkingEol,
        ProjectCheckId::RepositoryLfs,
    ] {
        assert!(
            snapshot.observations.iter().any(|observation| {
                observation.id == check
                    && observation.source_set.as_deref() == Some("main")
                    && matches!(observation.outcome, ProjectCheckOutcome::NotRun { .. })
            }),
            "{check:?}: {:?}",
            snapshot.observations
        );
    }
}

#[test]
fn duplicate_source_set_names_do_not_invent_an_addressable_repository_row() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().to_path_buf();
    fs::create_dir_all(root.join("a")).unwrap();
    fs::create_dir_all(root.join("b")).unwrap();
    fs::write(
        root.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: a\n  - name: main\n    type: EXTENSION\n    path: b\n",
    )
    .unwrap();
    let context = WorkspaceContext {
        cwd: root.clone(),
        workspace_root: root.clone(),
        cache_root: root.join(".build/unica"),
        workspace_epoch: 1,
    };

    let snapshot = inspect_project_health_with_runner(
        &context,
        &CancellationToken::new(),
        ProviderDeadline::from_budget(Duration::from_secs(2)),
        &NoGitRunner,
    )
    .unwrap();

    assert!(snapshot.observations.iter().all(|observation| {
        observation.source_set.as_deref() != Some("main")
            || !matches!(observation.id.scope(), crate::DiagnosticScope::Repository)
    }));
    let report = crate::evaluate_project_health(snapshot).unwrap();
    let diagnostics: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|item| item.code == "source_set.name_ambiguous")
        .collect();
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].scope, crate::DiagnosticScope::Workspace);
    assert_eq!(diagnostics[0].count, 2);
    assert!(diagnostics[0].source_set.is_none());
}

#[test]
fn old_git_partial_clone_does_not_run_blob_dependent_resource_commands() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().to_path_buf();
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("src/Configuration.xml"), "<MetaDataObject/>").unwrap();
    fs::write(
        root.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
    )
    .unwrap();
    let context = WorkspaceContext {
        cwd: root.clone(),
        workspace_root: root.clone(),
        cache_root: root.join(".build/unica"),
        workspace_epoch: 1,
    };
    let runner = OldPartialCloneRunner {
        repository_root: root,
        commands: RefCell::new(Vec::new()),
        index_paths: vec!["src/Configuration.xml".into()],
    };

    let snapshot = inspect_project_health_with_runner(
        &context,
        &CancellationToken::new(),
        ProviderDeadline::from_budget(Duration::from_secs(2)),
        &runner,
    )
    .unwrap();

    assert!(runner.commands.borrow().iter().all(|command| {
        command.args.first().map(String::as_str) != Some("check-attr")
            && !(command.args.first().map(String::as_str) == Some("ls-files")
                && command.args.iter().any(|argument| argument == "--eol"))
    }));
    for check in [
        ProjectCheckId::RepositoryAttributes,
        ProjectCheckId::RepositoryIndexEol,
        ProjectCheckId::RepositoryWorkingEol,
        ProjectCheckId::RepositoryLfs,
    ] {
        assert!(snapshot.observations.iter().any(|observation| {
            observation.id == check
                && matches!(observation.outcome, ProjectCheckOutcome::NotRun { .. })
        }));
    }
}

#[test]
fn edt_only_old_partial_clone_keeps_resource_checks_not_applicable() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().to_path_buf();
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("src/.project"), "<projectDescription/>").unwrap();
    fs::write(root.join(".gitignore"), "**/.build/\n").unwrap();
    fs::write(
        root.join("v8project.yaml"),
        "format: EDT\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
    )
    .unwrap();
    let context = WorkspaceContext {
        cwd: root.clone(),
        workspace_root: root.clone(),
        cache_root: root.join(".build/unica"),
        workspace_epoch: 1,
    };
    let runner = OldPartialCloneRunner {
        repository_root: root,
        commands: RefCell::new(Vec::new()),
        index_paths: vec!["src/.project".into(), ".gitignore".into()],
    };

    let snapshot = inspect_project_health_with_runner(
        &context,
        &CancellationToken::new(),
        ProviderDeadline::from_budget(Duration::from_secs(2)),
        &runner,
    )
    .unwrap();
    assert!(snapshot.observations.iter().any(|observation| {
        observation.id == ProjectCheckId::RepositoryIgnore
            && matches!(observation.outcome, ProjectCheckOutcome::NotRun { .. })
    }));
    let report = crate::evaluate_project_health(snapshot).unwrap();

    for check in [
        ProjectCheckId::RepositoryAttributes,
        ProjectCheckId::RepositoryIndexEol,
        ProjectCheckId::RepositoryWorkingEol,
        ProjectCheckId::RepositoryLfs,
    ] {
        assert!(report.checks.iter().any(|reported| {
            reported.id == check.as_str()
                && reported.source_set.as_deref() == Some("main")
                && reported.status == crate::ProjectCheckStatus::NotApplicable
        }));
    }
}

#[test]
fn staged_platform_marker_makes_old_partial_clone_resource_checks_not_run() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().to_path_buf();
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("src/.project"), "<projectDescription/>").unwrap();
    fs::write(root.join(".gitignore"), "**/.build/\n").unwrap();
    fs::write(
        root.join("v8project.yaml"),
        "format: EDT\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
    )
    .unwrap();
    let context = WorkspaceContext {
        cwd: root.clone(),
        workspace_root: root.clone(),
        cache_root: root.join(".build/unica"),
        workspace_epoch: 1,
    };
    let runner = OldPartialCloneRunner {
        repository_root: root,
        commands: RefCell::new(Vec::new()),
        index_paths: vec![
            "src/.project".into(),
            "src/Configuration.xml".into(),
            ".gitignore".into(),
        ],
    };

    let snapshot = inspect_project_health_with_runner(
        &context,
        &CancellationToken::new(),
        ProviderDeadline::from_budget(Duration::from_secs(2)),
        &runner,
    )
    .unwrap();
    let report = crate::evaluate_project_health(snapshot).unwrap();

    for check in [
        ProjectCheckId::RepositoryAttributes,
        ProjectCheckId::RepositoryIndexEol,
        ProjectCheckId::RepositoryWorkingEol,
        ProjectCheckId::RepositoryLfs,
    ] {
        assert!(
            report.checks.iter().any(|reported| {
                reported.id == check.as_str()
                    && reported.source_set.as_deref() == Some("main")
                    && reported.status == crate::ProjectCheckStatus::NotRun
            }),
            "{check:?}: {:?}",
            report.checks
        );
    }
}

#[test]
fn staged_platform_marker_without_policy_files_still_probes_old_partial_clone() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().to_path_buf();
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("src/.project"), "<projectDescription/>").unwrap();
    fs::write(
        root.join("v8project.yaml"),
        "format: EDT\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
    )
    .unwrap();
    let context = WorkspaceContext {
        cwd: root.clone(),
        workspace_root: root.clone(),
        cache_root: root.join(".build/unica"),
        workspace_epoch: 1,
    };
    let runner = OldPartialCloneRunner {
        repository_root: root,
        commands: RefCell::new(Vec::new()),
        index_paths: vec!["src/Configuration.xml".into()],
    };

    let snapshot = inspect_project_health_with_runner(
        &context,
        &CancellationToken::new(),
        ProviderDeadline::from_budget(Duration::from_secs(2)),
        &runner,
    )
    .unwrap();
    let report = crate::evaluate_project_health(snapshot).unwrap();

    assert!(runner.commands.borrow().iter().any(|command| command
        .args
        .first()
        .map(String::as_str)
        == Some("config")));
    for check in [
        ProjectCheckId::RepositoryAttributes,
        ProjectCheckId::RepositoryIndexEol,
        ProjectCheckId::RepositoryWorkingEol,
        ProjectCheckId::RepositoryLfs,
    ] {
        assert!(
            report.checks.iter().any(|reported| {
                reported.id == check.as_str()
                    && reported.source_set.as_deref() == Some("main")
                    && reported.status == crate::ProjectCheckStatus::NotRun
            }),
            "{check:?}: {:?}",
            report.checks
        );
    }
}

#[test]
fn unclassified_staged_config_dump_marker_keeps_old_partial_resource_checks_not_run() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().to_path_buf();
    fs::create_dir_all(root.join("reports")).unwrap();
    fs::write(root.join(".gitignore"), "**/.build/\n").unwrap();
    fs::write(
        root.join("v8project.yaml"),
        "format: EDT\nsource-set:\n  - name: reports\n    type: EXTERNAL_REPORTS\n    path: reports\n",
    )
    .unwrap();
    let context = WorkspaceContext {
        cwd: root.clone(),
        workspace_root: root.clone(),
        cache_root: root.join(".build/unica"),
        workspace_epoch: 1,
    };
    let runner = OldPartialCloneRunner {
        repository_root: root,
        commands: RefCell::new(Vec::new()),
        index_paths: vec!["reports/ConfigDumpInfo.xml".into(), ".gitignore".into()],
    };

    let snapshot = inspect_project_health_with_runner(
        &context,
        &CancellationToken::new(),
        ProviderDeadline::from_budget(Duration::from_secs(2)),
        &runner,
    )
    .unwrap();
    let report = crate::evaluate_project_health(snapshot).unwrap();

    for check in [
        ProjectCheckId::RepositoryAttributes,
        ProjectCheckId::RepositoryIndexEol,
        ProjectCheckId::RepositoryWorkingEol,
        ProjectCheckId::RepositoryLfs,
    ] {
        assert!(
            report.checks.iter().any(|reported| {
                reported.id == check.as_str()
                    && reported.source_set.as_deref() == Some("reports")
                    && reported.status == crate::ProjectCheckStatus::NotRun
            }),
            "{check:?}: {:?}",
            report.checks
        );
    }
}

struct RecordingRunner {
    repository_root: PathBuf,
    commands: RefCell<Vec<ProcessCommand>>,
}

struct NoGitRunner;

struct OldPartialCloneRunner {
    repository_root: PathBuf,
    commands: RefCell<Vec<ProcessCommand>>,
    index_paths: Vec<String>,
}

impl ProcessRunner for OldPartialCloneRunner {
    fn run(&self, command: &ProcessCommand) -> Result<ProcessOutput, String> {
        self.commands.borrow_mut().push(command.clone());
        match command.args.first().map(String::as_str) {
            Some("rev-parse") => Ok(output(
                true,
                &format!("{}\ntrue\n", self.repository_root.display()),
            )),
            Some("ls-files") if command.args.iter().any(|arg| arg == "--stage") => {
                let stdout = self
                    .index_paths
                    .iter()
                    .map(|path| format!("100644 {} 0\t{path}\0", "a".repeat(40)))
                    .collect::<String>();
                Ok(output(true, &stdout))
            }
            Some("config") => Ok(output(true, "remote.origin.promisor\ntrue\0")),
            Some("version") => Ok(output(true, "git version 2.40.1\n")),
            Some(arg) if arg.starts_with("--work-tree=") => Ok(ProcessOutput {
                status_success: false,
                status: "exit status: 1".into(),
                ..output(false, "")
            }),
            Some("check-ignore") => Ok(ProcessOutput {
                status_success: false,
                status: "exit status: 1".into(),
                ..output(false, "")
            }),
            Some("check-attr") => Err("blob-dependent resource inspection ran".into()),
            Some("ls-files") if command.args.iter().any(|arg| arg == "--eol") => {
                Err("blob-dependent resource inspection ran".into())
            }
            _ => Err(format!("unexpected Git command: {:?}", command.args)),
        }
    }

    fn run_with_input(
        &self,
        command: &ProcessCommand,
        _input: &[u8],
    ) -> Result<ProcessOutput, String> {
        self.run(command)
    }
}

impl ProcessRunner for NoGitRunner {
    fn run(&self, _command: &ProcessCommand) -> Result<ProcessOutput, String> {
        Ok(ProcessOutput {
            status_success: false,
            status: "exit status: 128".into(),
            stdout: String::new(),
            stderr: "fatal: not a git repository".into(),
            timed_out: false,
            cancelled: false,
            stdout_truncated: false,
            stderr_truncated: false,
            stdout_had_invalid_utf8: false,
            stderr_had_invalid_utf8: false,
        })
    }

    fn run_with_input(
        &self,
        command: &ProcessCommand,
        _input: &[u8],
    ) -> Result<ProcessOutput, String> {
        self.run(command)
    }
}

impl ProcessRunner for RecordingRunner {
    fn run(&self, command: &ProcessCommand) -> Result<ProcessOutput, String> {
        self.commands.borrow_mut().push(command.clone());
        if command.args.first().map(String::as_str) == Some("rev-parse") {
            return Ok(output(
                true,
                &format!("{}\ntrue\n", self.repository_root.display()),
            ));
        }
        if command.args.first().map(String::as_str) == Some("ls-files")
            && command.args.contains(&"--stage".to_string())
        {
            let index = if self.repository_root.join("src/a").exists() {
                output(true, "")
            } else {
                ProcessOutput {
                    stdout_truncated: true,
                    ..output(true, "")
                }
            };
            return Ok(index);
        }
        Err("resource inspection must not run after an incomplete index".into())
    }

    fn run_with_input(
        &self,
        command: &ProcessCommand,
        _input: &[u8],
    ) -> Result<ProcessOutput, String> {
        self.run(command)
    }
}

fn output(status_success: bool, stdout: &str) -> ProcessOutput {
    ProcessOutput {
        status_success,
        status: "exit status: 0".into(),
        stdout: stdout.into(),
        stderr: String::new(),
        timed_out: false,
        cancelled: false,
        stdout_truncated: false,
        stderr_truncated: false,
        stdout_had_invalid_utf8: false,
        stderr_had_invalid_utf8: false,
    }
}
