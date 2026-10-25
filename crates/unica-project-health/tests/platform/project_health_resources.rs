use super::{
    classify_platform_xml_relative_path, inspect_working_eol, parse_attribute_records,
    parse_eol_records, resource_check_ids, RepositoryResourceKind, SourceResourcePolicyInspector,
    LFS_SINGLE_FILE_THRESHOLD_BYTES,
};
use crate::{ProjectCheckId, ProjectCheckOutcome, ProjectHealthFact, ProjectHealthInspectionError};
use unica_source_contract::{ProjectSourceSet, SourceFormat, SourceSetKind};
use unica_work_control::cancellation::CancellationToken;
use unica_work_control::provider_deadline::ProviderDeadline;

use crate::inspection::git::{GitIndexEntry, GitRepositoryInspector};
use crate::inspection::layout::{InspectedSourceRoot, SourceLayoutInspector};
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use tempfile::TempDir;
use unica_process::runner::{ProcessCommand, ProcessOutput, ProcessRunner};
use unica_source_contract::workspace::WorkspaceContext;
use unica_workspace::test_support::{
    create_directory_link_fixture_for_test, create_file_link_fixture_for_test,
    FileLinkFixtureOutcome,
};

#[test]
fn project_health_repository_policy_classifies_platform_xml_roles_exactly() {
    assert_eq!(
        classify_platform_xml_relative_path("XDTOPackages/Sales/Ext/Package.bin"),
        Some(RepositoryResourceKind::Text)
    );
    assert_eq!(
        classify_platform_xml_relative_path("Templates/Blob/Ext/Template.bin"),
        Some(RepositoryResourceKind::Binary)
    );
    assert_eq!(
        classify_platform_xml_relative_path("Other/XDTOPackages/Sales/Ext/Package.bin"),
        Some(RepositoryResourceKind::Binary)
    );
    assert_eq!(
        classify_platform_xml_relative_path("CommonModules/Sales/Ext/Module.bsl"),
        Some(RepositoryResourceKind::Text)
    );
    assert_eq!(classify_platform_xml_relative_path("unknown.dat"), None);
}

#[test]
fn resource_owner_lookup_uses_deepest_path_ancestor() {
    let fixture = policy_fixture();
    let cancellation = CancellationToken::new();
    let layout = SourceLayoutInspector::inspect(
        &fixture.context,
        &cancellation,
        ProviderDeadline::from_budget(Duration::from_secs(2)),
        &unica_workspace::discovery::ProjectDiscoveryReader::new(&fixture.context.workspace_root),
    )
    .unwrap();
    let resources = SourceResourcePolicyInspector::classify(
        &fixture.root,
        &layout.roots,
        &[GitIndexEntry {
            repo_path: "src/Configuration.xml".into(),
            blob_oid: Some("a".repeat(40)),
            mode: Some("100644".into()),
        }],
        &Default::default(),
    )
    .unwrap();

    assert_eq!(resources.len(), 1);
    assert_eq!(resources[0].source_set, "main");
}

#[test]
fn resource_owner_names_past_the_former_expansion_budget_are_classified() {
    // 256 resources of a 40 000-byte owner name retain over 10 MB of owner
    // names, past the former 8 MiB refusal. Every resource is classified.
    let temp = TempDir::new().unwrap();
    let root = temp.path().to_path_buf();
    fs::create_dir_all(root.join("src")).unwrap();
    let owner = "owner-".to_string() + &"x".repeat(40_000);
    let roots = vec![InspectedSourceRoot {
        source_set: ProjectSourceSet {
            name: owner.clone(),
            kind: SourceSetKind::Configuration,
            path: "src".into(),
            source_format: SourceFormat::PlatformXml,
            source_state: unica_source_contract::SourceSetState::Supported,
            format_evidence: Vec::new(),
            format_probe_error: None,
        },
        path: root.join("src"),
    }];
    let entries = (0..256)
        .map(|index| GitIndexEntry {
            repo_path: format!("src/Module{index:03}.bsl"),
            blob_oid: Some(format!("{index:040x}")),
            mode: Some("100644".into()),
        })
        .collect::<Vec<_>>();

    let resources =
        SourceResourcePolicyInspector::classify(&root, &roots, &entries, &Default::default())
            .unwrap();

    assert_eq!(resources.len(), 256);
    assert!(resources
        .iter()
        .all(|resource| resource.source_set == owner));
}

#[test]
fn nested_edt_root_shields_resources_from_outer_platform_xml_profile() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().to_path_buf();
    fs::create_dir_all(root.join("src/edt")).unwrap();
    let roots = vec![
        InspectedSourceRoot {
            source_set: ProjectSourceSet {
                name: "designer".into(),
                kind: SourceSetKind::Configuration,
                path: "src".into(),
                source_format: SourceFormat::PlatformXml,
                source_state: unica_source_contract::SourceSetState::Supported,
                format_evidence: Vec::new(),
                format_probe_error: None,
            },
            path: root.join("src"),
        },
        InspectedSourceRoot {
            source_set: ProjectSourceSet {
                name: "edt".into(),
                kind: SourceSetKind::Configuration,
                path: "src/edt".into(),
                source_format: SourceFormat::Edt,
                source_state: unica_source_contract::SourceSetState::Unsupported,
                format_evidence: Vec::new(),
                format_probe_error: None,
            },
            path: root.join("src/edt"),
        },
    ];

    let resources = SourceResourcePolicyInspector::classify(
        &root,
        &roots,
        &[GitIndexEntry {
            repo_path: "src/edt/Foo.xml".into(),
            blob_oid: Some("a".repeat(40)),
            mode: Some("100644".into()),
        }],
        &Default::default(),
    )
    .unwrap();

    assert!(resources.is_empty(), "{resources:?}");
}

#[test]
fn platform_probe_failure_does_not_overwrite_edt_not_applicable() {
    let fixture = policy_fixture();
    fs::create_dir_all(fixture.root.join("src/edt")).unwrap();
    fs::write(
        fixture.root.join("src/edt/.project"),
        "<projectDescription/>",
    )
    .unwrap();
    fs::write(
        fixture.root.join("v8project.yaml"),
        "source-set:\n  - name: designer\n    type: CONFIGURATION\n    path: src\n  - name: edt\n    type: CONFIGURATION\n    path: src/edt\n",
    )
    .unwrap();
    let cancellation = CancellationToken::new();
    let layout = SourceLayoutInspector::inspect(
        &fixture.context,
        &cancellation,
        ProviderDeadline::from_budget(Duration::from_secs(2)),
        &unica_workspace::discovery::ProjectDiscoveryReader::new(&fixture.context.workspace_root),
    )
    .unwrap();

    let inspection = SourceResourcePolicyInspector::with_process_runner(
        &FailingRunner,
        crate::inspection::TEST_RESPONSE_HANDOFF_WINDOW,
    )
    .inspect(
        &fixture.root,
        &layout.roots,
        &[GitIndexEntry {
            repo_path: "src/Configuration.xml".into(),
            blob_oid: Some("a".repeat(40)),
            mode: Some("100644".into()),
        }],
        &cancellation,
        ProviderDeadline::from_budget(Duration::from_secs(2)),
    )
    .unwrap();

    for check in resource_check_ids() {
        assert!(
            inspection.observations.iter().any(|observation| {
                observation.id == check
                    && observation.source_set.as_deref() == Some("designer")
                    && matches!(observation.outcome, ProjectCheckOutcome::NotRun { .. })
            }),
            "{check:?}: {:?}",
            inspection.observations
        );
        assert!(
            inspection.observations.iter().any(|observation| {
                observation.id == check
                    && observation.source_set.as_deref() == Some("edt")
                    && matches!(
                        observation.outcome,
                        ProjectCheckOutcome::NotApplicable { .. }
                    )
            }),
            "{check:?}: {:?}",
            inspection.observations
        );
    }
}

#[test]
fn project_health_rejects_nonregular_staged_resource() {
    let fixture = policy_fixture();
    let cancellation = CancellationToken::new();
    let layout = SourceLayoutInspector::inspect(
        &fixture.context,
        &cancellation,
        ProviderDeadline::from_budget(Duration::from_secs(2)),
        &unica_workspace::discovery::ProjectDiscoveryReader::new(&fixture.context.workspace_root),
    )
    .unwrap();

    let inspection = SourceResourcePolicyInspector::with_process_runner(
        &FailingRunner,
        crate::inspection::TEST_RESPONSE_HANDOFF_WINDOW,
    )
    .inspect(
        &fixture.root,
        &layout.roots,
        &[GitIndexEntry {
            repo_path: "src/Picture.bin".into(),
            blob_oid: None,
            mode: None,
        }],
        &cancellation,
        ProviderDeadline::from_budget(Duration::from_secs(2)),
    )
    .unwrap();

    assert!(inspection.facts.iter().any(|fact| matches!(
        fact,
        ProjectHealthFact::GitInspectionIncompleteCounted {
            check: ProjectCheckId::RepositoryAttributes,
            reason,
            count: 1,
            ..
        } if reason.contains("regular stage-0 blob")
    )));
}

#[test]
fn project_health_repository_policy_parsers_require_complete_nul_protocols() {
    let attributes = parse_attribute_records(
        "src/A.xml\0text\0set\0src/A.xml\0eol\0lf\0src/A.xml\0filter\0unspecified\0",
        &["src/A.xml".into()],
    )
    .unwrap();
    assert_eq!(attributes["src/A.xml"].text, "set");
    assert_eq!(attributes["src/A.xml"].eol, "lf");
    assert_eq!(attributes["src/A.xml"].filter, "unspecified");
    assert!(parse_attribute_records("src/A.xml\0text\0set\0", &["src/A.xml".into()]).is_err());

    let expected = vec!["src/A.xml".to_string()];
    let eol = parse_eol_records("i/lf w/crlf attr/text eol=crlf\tsrc/A.xml\0", &expected).unwrap();
    assert_eq!(eol["src/A.xml"].index, "lf");
    assert_eq!(eol["src/A.xml"].worktree, "crlf");
    assert!(parse_eol_records("i/lf w/lf without-tab\0", &expected).is_err());
    assert!(parse_eol_records("i/unknown w/lf attr/text\tsrc/A.xml\0", &expected).is_err());
}

#[test]
fn repository_policy_parsers_honor_cooperative_checkpoint() {
    let attributes = "src/A.xml\0text\0set\0src/A.xml\0eol\0lf\0src/A.xml\0filter\0unspecified\0";
    let mut attribute_checkpoints = 0;
    let attribute_error = super::parse_attribute_records_with_checkpoint(
        attributes,
        &["src/A.xml".into()],
        &mut || {
            attribute_checkpoints += 1;
            Err(super::ResourceProtocolParseError::TimedOut)
        },
    )
    .unwrap_err();
    assert!(matches!(
        attribute_error,
        super::ResourceProtocolParseError::TimedOut
    ));
    assert_eq!(attribute_checkpoints, 1);

    let mut eol_checkpoints = 0;
    let eol_error = super::parse_eol_records_with_checkpoint(
        "i/lf w/lf attr/text\tsrc/A.xml\0",
        &["src/A.xml".into()],
        &mut || {
            eol_checkpoints += 1;
            Err(super::ResourceProtocolParseError::Cancelled)
        },
    )
    .unwrap_err();
    assert!(matches!(
        eol_error,
        super::ResourceProtocolParseError::Cancelled
    ));
    assert_eq!(eol_checkpoints, 1);

    let attribute_error =
        super::parse_attribute_records_with_checkpoint("unterminated", &[], &mut || {
            Err(super::ResourceProtocolParseError::Cancelled)
        })
        .unwrap_err();
    assert!(matches!(
        attribute_error,
        super::ResourceProtocolParseError::Cancelled
    ));

    let eol_error = super::parse_eol_records_with_checkpoint("unterminated", &[], &mut || {
        Err(super::ResourceProtocolParseError::TimedOut)
    })
    .unwrap_err();
    assert!(matches!(
        eol_error,
        super::ResourceProtocolParseError::TimedOut
    ));
}

#[test]
fn repository_policy_parsers_checkpoint_final_expected_path_validation() {
    let attributes = "src/A.xml\0text\0set\0src/A.xml\0eol\0lf\0src/A.xml\0filter\0unspecified\0";
    let mut attribute_checkpoints = 0;
    let attribute_error = super::parse_attribute_records_with_checkpoint(
        attributes,
        &["src/A.xml".into()],
        &mut || {
            attribute_checkpoints += 1;
            if attribute_checkpoints == 4 {
                Err(super::ResourceProtocolParseError::TimedOut)
            } else {
                Ok(())
            }
        },
    )
    .unwrap_err();
    assert!(matches!(
        attribute_error,
        super::ResourceProtocolParseError::TimedOut
    ));

    let mut eol_checkpoints = 0;
    let eol_error = super::parse_eol_records_with_checkpoint(
        "i/lf w/lf attr/text\tsrc/A.xml\0",
        &["src/A.xml".into()],
        &mut || {
            eol_checkpoints += 1;
            if eol_checkpoints == 3 {
                Err(super::ResourceProtocolParseError::Cancelled)
            } else {
                Ok(())
            }
        },
    )
    .unwrap_err();
    assert!(matches!(
        eol_error,
        super::ResourceProtocolParseError::Cancelled
    ));
}

#[test]
fn index_eol_evaluation_is_all_or_nothing_on_timeout() {
    let resources = vec![
        super::RepositoryResource {
            source_set: "main".into(),
            repo_path: "src/A.xml".into(),
            kind: RepositoryResourceKind::Text,
            blob_oid: "a".repeat(40),
        },
        super::RepositoryResource {
            source_set: "main".into(),
            repo_path: "src/B.xml".into(),
            kind: RepositoryResourceKind::Text,
            blob_oid: "b".repeat(40),
        },
    ];
    let eol = std::collections::BTreeMap::from([
        (
            "src/A.xml".into(),
            super::EolValues {
                index: "lf".into(),
                worktree: "lf".into(),
            },
        ),
        (
            "src/B.xml".into(),
            super::EolValues {
                index: "crlf".into(),
                worktree: "crlf".into(),
            },
        ),
    ]);
    let mut checkpoints = 0;

    let result =
        super::index_eol_facts_with_checkpoint(&resources, &eol, &Default::default(), &mut || {
            checkpoints += 1;
            if checkpoints == 2 {
                Err(super::ResourceProtocolParseError::TimedOut)
            } else {
                Ok(())
            }
        });

    assert!(matches!(
        result,
        Err(super::ResourceProtocolParseError::TimedOut)
    ));
}

#[test]
fn index_eol_evaluation_checks_deadline_for_fully_filtered_resources() {
    let resources = vec![super::RepositoryResource {
        source_set: "main".into(),
        repo_path: "src/Picture.bin".into(),
        kind: RepositoryResourceKind::Binary,
        blob_oid: "a".repeat(40),
    }];

    let result = super::index_eol_facts_with_checkpoint(
        &resources,
        &Default::default(),
        &Default::default(),
        &mut || Err(super::ResourceProtocolParseError::TimedOut),
    );

    assert!(matches!(
        result,
        Err(super::ResourceProtocolParseError::TimedOut)
    ));
}

#[test]
fn attribute_policy_evaluation_is_all_or_nothing_on_timeout() {
    let resources = vec![super::RepositoryResource {
        source_set: "main".into(),
        repo_path: "src/A.xml".into(),
        kind: RepositoryResourceKind::Text,
        blob_oid: "a".repeat(40),
    }];
    let attributes = std::collections::BTreeMap::from([(
        "src/A.xml".into(),
        super::AttributeValues {
            text: "unspecified".into(),
            eol: "unspecified".into(),
            filter: "unspecified".into(),
        },
    )]);

    let result = super::evaluate_attribute_policy_with_checkpoint(
        &resources,
        &attributes,
        &attributes,
        &mut || Err(super::ResourceProtocolParseError::TimedOut),
    );

    assert!(matches!(
        result,
        Err(super::ResourceProtocolParseError::TimedOut)
    ));
}

#[test]
fn lfs_aggregation_is_all_or_nothing_on_timeout() {
    let mut aggregate = super::LfsAggregate::default();
    aggregate.add("src/Picture.bin", 12 * 1024 * 1024).unwrap();
    let binaries = std::collections::BTreeMap::from([("main".to_string(), aggregate)]);

    let result = super::lfs_facts_with_checkpoint(binaries, &mut || {
        Err(super::ResourceProtocolParseError::TimedOut)
    });

    assert!(matches!(
        result,
        Err(super::ResourceProtocolParseError::TimedOut)
    ));
}
#[test]
fn project_health_repository_policy_does_not_pass_checks_that_were_not_run() {
    let fixture = policy_fixture();
    fs::write(fixture.root.join("src/A.xml"), "<A/>\n").unwrap();
    let cancellation = CancellationToken::new();
    let layout = SourceLayoutInspector::inspect(
        &fixture.context,
        &cancellation,
        ProviderDeadline::from_budget(Duration::from_secs(2)),
        &unica_workspace::discovery::ProjectDiscoveryReader::new(&fixture.context.workspace_root),
    )
    .unwrap();
    let entries = vec![GitIndexEntry {
        repo_path: "src/A.xml".into(),
        blob_oid: Some("a".repeat(40)),
        mode: Some("100644".into()),
    }];

    let inspection = SourceResourcePolicyInspector::with_process_runner(
        &FailingRunner,
        crate::inspection::TEST_RESPONSE_HANDOFF_WINDOW,
    )
    .inspect(
        &fixture.root,
        &layout.roots,
        &entries,
        &cancellation,
        ProviderDeadline::from_budget(Duration::from_secs(2)),
    )
    .unwrap();

    for check in [
        ProjectCheckId::RepositoryAttributes,
        ProjectCheckId::RepositoryIndexEol,
        ProjectCheckId::RepositoryWorkingEol,
        ProjectCheckId::RepositoryLfs,
    ] {
        assert!(
            inspection.observations.iter().any(|observation| {
                observation.id == check
                    && observation.source_set.is_none()
                    && matches!(
                        observation.outcome,
                        crate::ProjectCheckOutcome::NotRun { .. }
                    )
            }),
            "{check:?}: {:?}",
            inspection.observations
        );
    }
}

#[test]
fn edt_only_repository_policy_remains_not_applicable_when_no_resources_exist() {
    let fixture = policy_fixture();
    fs::remove_file(fixture.root.join("src/Configuration.xml")).unwrap();
    fs::write(fixture.root.join("src/.project"), "<projectDescription/>").unwrap();
    fs::write(
        fixture.root.join("v8project.yaml"),
        "format: EDT\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
    )
    .unwrap();
    let cancellation = CancellationToken::new();
    let layout = SourceLayoutInspector::inspect(
        &fixture.context,
        &cancellation,
        ProviderDeadline::from_budget(Duration::from_secs(2)),
        &unica_workspace::discovery::ProjectDiscoveryReader::new(&fixture.context.workspace_root),
    )
    .unwrap();

    let inspection = SourceResourcePolicyInspector::with_process_runner(
        &FailingRunner,
        crate::inspection::TEST_RESPONSE_HANDOFF_WINDOW,
    )
    .inspect(
        &fixture.root,
        &layout.roots,
        &[],
        &cancellation,
        ProviderDeadline::from_budget(Duration::from_secs(2)),
    )
    .unwrap();

    assert!(inspection.facts.is_empty());
    assert!(
        inspection.observations.iter().all(|observation| matches!(
            observation.outcome,
            crate::ProjectCheckOutcome::NotApplicable { .. }
        )),
        "{:?}",
        inspection.observations
    );
}

#[test]
fn project_health_attribute_probe_does_not_require_check_attr_source() {
    let fixture = policy_fixture();
    fs::write(fixture.root.join("src/A.xml"), "<A/>\n").unwrap();
    let cancellation = CancellationToken::new();
    let layout = SourceLayoutInspector::inspect(
        &fixture.context,
        &cancellation,
        ProviderDeadline::from_budget(Duration::from_secs(2)),
        &unica_workspace::discovery::ProjectDiscoveryReader::new(&fixture.context.workspace_root),
    )
    .unwrap();
    let entries = vec![GitIndexEntry {
        repo_path: "src/A.xml".into(),
        blob_oid: Some("a".repeat(40)),
        mode: Some("100644".into()),
    }];
    let runner = AttributeProbeRunner::default();

    SourceResourcePolicyInspector::with_process_runner(
        &runner,
        crate::inspection::TEST_RESPONSE_HANDOFF_WINDOW,
    )
    .inspect(
        &fixture.root,
        &layout.roots,
        &entries,
        &cancellation,
        ProviderDeadline::from_budget(Duration::from_secs(2)),
    )
    .unwrap();

    assert!(
        runner
            .commands
            .borrow()
            .iter()
            .all(|command| { !command.args.iter().any(|arg| arg.starts_with("--source=")) }),
        "{:?}",
        runner.commands.borrow()
    );
}

#[test]
fn late_cancellation_during_staged_attribute_probe_cancels_inspection() {
    let fixture = policy_fixture();
    fs::write(fixture.root.join("src/A.xml"), "<A/>\n").unwrap();
    let cancellation = CancellationToken::new();
    let layout = SourceLayoutInspector::inspect(
        &fixture.context,
        &cancellation,
        ProviderDeadline::from_budget(Duration::from_secs(2)),
        &unica_workspace::discovery::ProjectDiscoveryReader::new(&fixture.context.workspace_root),
    )
    .unwrap();
    let runner = LateCancellingAttributeRunner {
        inner: AttributeProbeRunner::default(),
        cancellation: cancellation.clone(),
    };

    let result = SourceResourcePolicyInspector::with_process_runner(
        &runner,
        crate::inspection::TEST_RESPONSE_HANDOFF_WINDOW,
    )
    .inspect(
        &fixture.root,
        &layout.roots,
        &[GitIndexEntry {
            repo_path: "src/A.xml".into(),
            blob_oid: Some("a".repeat(40)),
            mode: Some("100644".into()),
        }],
        &cancellation,
        ProviderDeadline::from_budget(Duration::from_secs(2)),
    );

    assert!(matches!(
        result,
        Err(ProjectHealthInspectionError::Cancelled)
    ));
}

#[test]
fn late_cancellation_during_eol_index_creation_cancels_inspection() {
    let fixture = policy_fixture();
    fs::write(fixture.root.join("src/A.xml"), "<A/>\n").unwrap();
    let cancellation = CancellationToken::new();
    let layout = SourceLayoutInspector::inspect(
        &fixture.context,
        &cancellation,
        ProviderDeadline::from_budget(Duration::from_secs(2)),
        &unica_workspace::discovery::ProjectDiscoveryReader::new(&fixture.context.workspace_root),
    )
    .unwrap();
    let runner = LateCancellingEolIndexRunner {
        inner: AttributeProbeRunner::default(),
        cancellation: cancellation.clone(),
        read_tree_calls: Cell::new(0),
    };

    let result = SourceResourcePolicyInspector::with_process_runner(
        &runner,
        crate::inspection::TEST_RESPONSE_HANDOFF_WINDOW,
    )
    .inspect(
        &fixture.root,
        &layout.roots,
        &[GitIndexEntry {
            repo_path: "src/A.xml".into(),
            blob_oid: Some("a".repeat(40)),
            mode: Some("100644".into()),
        }],
        &cancellation,
        ProviderDeadline::from_budget(Duration::from_secs(2)),
    );

    assert!(matches!(
        result,
        Err(ProjectHealthInspectionError::Cancelled)
    ));
}

#[test]
fn project_health_index_eol_probe_never_uses_the_real_worktree() {
    let fixture = policy_fixture();
    fs::write(fixture.root.join("src/A.xml"), "<A/>\n").unwrap();
    let cancellation = CancellationToken::new();
    let layout = SourceLayoutInspector::inspect(
        &fixture.context,
        &cancellation,
        ProviderDeadline::from_budget(Duration::from_secs(2)),
        &unica_workspace::discovery::ProjectDiscoveryReader::new(&fixture.context.workspace_root),
    )
    .unwrap();
    let entries = vec![GitIndexEntry {
        repo_path: "src/A.xml".into(),
        blob_oid: Some("a".repeat(40)),
        mode: Some("100644".into()),
    }];
    let runner = AttributeProbeRunner::default();

    SourceResourcePolicyInspector::with_process_runner(
        &runner,
        crate::inspection::TEST_RESPONSE_HANDOFF_WINDOW,
    )
    .inspect(
        &fixture.root,
        &layout.roots,
        &entries,
        &cancellation,
        ProviderDeadline::from_budget(Duration::from_secs(2)),
    )
    .unwrap();

    let commands = runner.commands.borrow();
    let eol_command = commands
        .iter()
        .find(|command| command.args.iter().any(|arg| arg == "--eol"))
        .expect("index EOL probe was executed");
    assert!(
        eol_command
            .env
            .iter()
            .any(|(name, _)| name == "GIT_INDEX_FILE"),
        "{eol_command:?}"
    );
    assert!(
        eol_command
            .args
            .iter()
            .any(|arg| arg.starts_with("--work-tree=")),
        "{eol_command:?}"
    );
}

#[test]
fn project_health_repository_policy_accepts_tracked_portable_attributes() {
    let fixture = policy_fixture();
    fs::create_dir_all(fixture.root.join("src/XDTOPackages/Sales/Ext")).unwrap();
    fs::write(fixture.root.join("src/A.xml"), "<A/>\n").unwrap();
    fs::write(
        fixture.root.join("src/XDTOPackages/Sales/Ext/Package.bin"),
        "<Package/>\n",
    )
    .unwrap();
    fs::write(fixture.root.join("src/Picture.bin"), b"binary").unwrap();
    fs::write(
        fixture.root.join(".gitattributes"),
        concat!(
            "*.xml text eol=lf\n",
            "*.bin -text\n",
            "XDTOPackages/**/Ext/Package.bin text eol=lf\n",
            "src/XDTOPackages/**/Ext/Package.bin text eol=lf\n",
        ),
    )
    .unwrap();
    git(&fixture.root, &["add", ".gitattributes", "src"]);

    let inspection = inspect_policy(&fixture);

    assert!(
        !inspection.facts.iter().any(|fact| matches!(
            fact,
            ProjectHealthFact::TextPolicyMissing { .. }
                | ProjectHealthFact::BinaryPolicyMissing { .. }
                | ProjectHealthFact::TextResourceMarkedBinary { .. }
                | ProjectHealthFact::AttributesLocalOnly { .. }
        )),
        "{:?}",
        inspection.facts
    );
}

#[test]
fn duplicate_local_attributes_do_not_invalidate_sufficient_staged_policy() {
    let fixture = policy_fixture();
    fs::write(fixture.root.join("src/A.xml"), "<A/>\n").unwrap();
    fs::write(fixture.root.join(".gitattributes"), "*.xml text eol=lf\n").unwrap();
    git(&fixture.root, &["add", ".gitattributes", "src"]);
    fs::write(
        fixture.root.join(".git/info/attributes"),
        "*.xml text eol=lf\n",
    )
    .unwrap();

    let inspection = inspect_policy(&fixture);

    assert!(
        !inspection.facts.iter().any(|fact| matches!(
            fact,
            ProjectHealthFact::AttributesLocalOnly { path, .. } if path == "src/A.xml"
        )),
        "{:?}",
        inspection.facts
    );
}

#[test]
fn project_health_repository_policy_reports_missing_and_local_only_attributes() {
    let fixture = policy_fixture();
    fs::write(fixture.root.join("src/A.xml"), "<A/>\n").unwrap();
    fs::write(fixture.root.join("src/Picture.bin"), b"binary").unwrap();
    git(&fixture.root, &["add", "src"]);

    let missing = inspect_policy(&fixture);

    assert!(
        missing.facts.iter().any(|fact| matches!(
            fact,
            ProjectHealthFact::TextPolicyMissing { path, .. } if path == "src/A.xml"
        )),
        "{:?}",
        missing.facts
    );
    assert!(
        missing.facts.iter().any(|fact| matches!(
            fact,
            ProjectHealthFact::BinaryPolicyMissing { path, .. } if path == "src/Picture.bin"
        )),
        "{:?}",
        missing.facts
    );

    fs::write(
        fixture.root.join(".git/info/attributes"),
        "*.xml text eol=lf\n*.bin -text\n",
    )
    .unwrap();
    let local = inspect_policy(&fixture);
    assert!(
        local.facts.iter().any(|fact| matches!(
            fact,
            ProjectHealthFact::AttributesLocalOnly { path, .. } if path == "src/A.xml"
        )),
        "{:?}",
        local.facts
    );
    assert!(!local.facts.iter().any(|fact| matches!(
        fact,
        ProjectHealthFact::TextPolicyMissing { path, .. } if path == "src/A.xml"
    )));
}

#[test]
fn text_marked_binary_suppresses_derivative_index_eol_fact() {
    let fixture = policy_fixture();
    fs::write(fixture.root.join("src/A.xml"), "<A/>\n").unwrap();
    fs::write(fixture.root.join(".gitattributes"), "*.xml -text\n").unwrap();
    git(&fixture.root, &["add", ".gitattributes", "src/A.xml"]);

    let inspection = inspect_policy(&fixture);

    assert!(
        inspection.facts.iter().any(|fact| matches!(
            fact,
            ProjectHealthFact::TextResourceMarkedBinary { path, .. } if path == "src/A.xml"
        )),
        "{:?}",
        inspection.facts
    );
    assert!(
        !inspection.facts.iter().any(|fact| matches!(
            fact,
            ProjectHealthFact::IndexEolNotLf { path, .. } if path == "src/A.xml"
        )),
        "{:?}",
        inspection.facts
    );
}

#[test]
fn local_eol_cannot_mask_staged_text_marked_binary_policy() {
    let fixture = policy_fixture();
    fs::write(fixture.root.join("src/A.xml"), "<A/>\n").unwrap();
    fs::write(fixture.root.join(".gitattributes"), "*.xml -text\n").unwrap();
    git(&fixture.root, &["add", ".gitattributes", "src/A.xml"]);
    fs::write(fixture.root.join(".git/info/attributes"), "*.xml eol=lf\n").unwrap();

    let inspection = inspect_policy(&fixture);

    assert!(inspection.facts.iter().any(|fact| matches!(
        fact,
        ProjectHealthFact::TextResourceMarkedBinary { path, .. } if path == "src/A.xml"
    )));
    assert!(!inspection.facts.iter().any(|fact| matches!(
        fact,
        ProjectHealthFact::AttributesLocalOnly { path, .. } if path == "src/A.xml"
    )));
}

#[test]
fn text_marked_binary_suppresses_derivative_working_eol_facts() {
    let fixture = policy_fixture();
    fs::write(fixture.root.join("src/A.xml"), "<A/>\n<B/>\n").unwrap();
    fs::write(fixture.root.join(".gitattributes"), "*.xml -text\n").unwrap();
    git(&fixture.root, &["add", ".gitattributes", "src/A.xml"]);
    fs::write(fixture.root.join("src/A.xml"), "<A/>\r\n<B/>\n").unwrap();

    let inspection = inspect_policy(&fixture);

    assert!(inspection.facts.iter().any(|fact| matches!(
        fact,
        ProjectHealthFact::TextResourceMarkedBinary { path, .. } if path == "src/A.xml"
    )));
    assert!(
        !inspection.facts.iter().any(|fact| matches!(
            fact,
            ProjectHealthFact::MixedEol { path, .. }
                | ProjectHealthFact::WorkingEolUnsupported { path, .. }
                if path == "src/A.xml"
        )),
        "{:?}",
        inspection.facts
    );
}

#[test]
fn empty_index_eol_metadata_is_incomplete_protocol() {
    let text = vec!["src/A.xml".to_string()];
    assert!(parse_eol_records("i/ w/lf attr/text\tsrc/A.xml\0", &text).is_err());
    assert!(parse_eol_records("i/lf w/ attr/text\tsrc/A.xml\0", &text).is_ok());
    assert!(parse_eol_records("i/ w/ attr/\tsrc/Linked.bin\0", &[]).is_ok());
}

#[test]
fn project_health_repository_policy_detects_mixed_worktree_eol() {
    let fixture = policy_fixture();
    fs::write(fixture.root.join(".gitattributes"), "*.xml text\n").unwrap();
    fs::write(fixture.root.join("src/A.xml"), "<A/>\n<B/>\n").unwrap();
    git(&fixture.root, &["add", ".gitattributes", "src/A.xml"]);
    fs::write(fixture.root.join("src/A.xml"), "<A/>\r\n<B/>\n").unwrap();

    let inspection = inspect_policy(&fixture);

    assert!(
        inspection.facts.iter().any(|fact| matches!(
            fact,
            ProjectHealthFact::MixedEol { path, .. } if path == "src/A.xml"
        )),
        "{:?}",
        inspection.facts
    );
}

#[test]
fn project_health_worktree_eol_rejects_a_symlink_at_open() {
    let temp = TempDir::new().unwrap();
    let target = temp.path().join("target.xml");
    let link = temp.path().join("link.xml");
    fs::write(&target, "<A/>\n").unwrap();
    match create_file_link_fixture_for_test(&target, &link).unwrap() {
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
    let physical_root = fs::canonicalize(temp.path()).unwrap();

    let error = inspect_working_eol(
        &physical_root,
        Path::new("link.xml"),
        &CancellationToken::new(),
        ProviderDeadline::from_budget(Duration::from_secs(1)),
    )
    .unwrap_err();
    let error = error.to_string();

    assert!(
        error.contains("link") || error.contains("reparse"),
        "{error}"
    );
}

thread_local! {
    static DEADLINE_CLOCK_TICKS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
static DEADLINE_CLOCK_ORIGIN: OnceLock<Instant> = OnceLock::new();

fn advancing_deadline_clock() -> Instant {
    let tick = DEADLINE_CLOCK_TICKS.with(|ticks| {
        let tick = ticks.get();
        ticks.set(tick + 1);
        tick as u64
    });
    *DEADLINE_CLOCK_ORIGIN.get_or_init(Instant::now) + Duration::from_millis(tick)
}

#[test]
fn project_health_worktree_eol_checks_deadline_between_chunks() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("large.xml");
    fs::write(&path, vec![b'x'; 128 * 1024]).unwrap();
    let physical_root = fs::canonicalize(temp.path()).unwrap();
    DEADLINE_CLOCK_TICKS.with(|ticks| ticks.set(0));
    let deadline = ProviderDeadline::with_clock(
        advancing_deadline_clock() + Duration::from_millis(2),
        advancing_deadline_clock,
    );

    let error = inspect_working_eol(
        &physical_root,
        Path::new("large.xml"),
        &CancellationToken::new(),
        deadline,
    )
    .unwrap_err();
    let error = error.to_string();

    assert!(error.contains("deadline"), "{error}");
}

#[test]
fn project_health_worktree_eol_streams_past_the_former_file_limit_and_reads_the_tail() {
    struct LargeTailReader {
        remaining: u64,
        tail: std::io::Cursor<&'static [u8]>,
    }

    impl std::io::Read for LargeTailReader {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            assert!(buffer.len() <= 64 * 1024, "whole-file buffer requested");
            let prefix = self.remaining.min(buffer.len() as u64) as usize;
            buffer[..prefix].fill(b'x');
            self.remaining -= prefix as u64;
            let suffix = std::io::Read::read(&mut self.tail, &mut buffer[prefix..])?;
            Ok(prefix + suffix)
        }
    }

    // The CR ends a 64 KiB block at the former 32 MiB ceiling; its LF
    // and the other significant bytes must still be classified.
    for (tail, expected) in [
        (b"\r\n".as_slice(), super::WorkingEol::Supported),
        (b"\r\nTAIL\n".as_slice(), super::WorkingEol::Mixed),
        (b"\rTAIL".as_slice(), super::WorkingEol::BareCr),
        (b"TAIL\r".as_slice(), super::WorkingEol::BareCr),
    ] {
        let reader = LargeTailReader {
            remaining: 32 * 1024 * 1024 - 1,
            tail: std::io::Cursor::new(tail),
        };
        let result = super::inspect_working_eol_reader(
            reader,
            &CancellationToken::new(),
            ProviderDeadline::from_budget(Duration::from_secs(30)),
        )
        .unwrap();
        assert_eq!(result, Some(expected));
    }
}

fn large_working_eol_file() -> TempDir {
    use std::io::{Read, Write};

    let temp = TempDir::new().unwrap();
    let mut file = fs::File::create(temp.path().join("large.xml")).unwrap();
    file.write_all(b"<A>").unwrap();
    std::io::copy(
        &mut std::io::repeat(b'x').take(32 * 1024 * 1024 - 4),
        &mut file,
    )
    .unwrap();
    file.write_all(b"\r\nTAIL\n</A>").unwrap();
    assert!(file.metadata().unwrap().len() > 32 * 1024 * 1024);
    temp
}

#[test]
fn project_health_worktree_eol_opens_a_file_past_the_former_metadata_limit() {
    let temp = large_working_eol_file();
    let root = fs::canonicalize(temp.path()).unwrap();
    let result = super::inspect_working_eol(
        &root,
        Path::new("large.xml"),
        &CancellationToken::new(),
        ProviderDeadline::from_budget(Duration::from_secs(30)),
    )
    .unwrap();
    assert_eq!(result, Some(super::WorkingEol::Mixed));
}

#[test]
fn continued_working_eol_opens_and_caches_a_file_past_the_former_metadata_limit() {
    let temp = large_working_eol_file();
    let root = fs::canonicalize(temp.path()).unwrap();
    let mut checkpoint = WorkingCheckpoint::default();
    let cancellation = CancellationToken::new();
    let deadline = ProviderDeadline::from_budget(Duration::from_secs(30));
    let result = checkpoint
        .inspect(&root, "large.xml", &cancellation, deadline)
        .unwrap();
    assert_eq!(result, Some(super::WorkingEol::Mixed));
    let evidence = checkpoint.evidence;
    assert!(matches!(
        evidence,
        Some(super::WorkingEvidence::Complete { .. })
    ));
    let retained_before = checkpoint.progress.working_eol_retained;
    let repeated = checkpoint
        .inspect(&root, "large.xml", &cancellation, deadline)
        .unwrap();
    assert_eq!(repeated, result);
    assert_eq!(checkpoint.evidence, evidence);
    assert_eq!(
        checkpoint.progress.working_eol_retained,
        retained_before + 1
    );
}
#[test]
fn project_health_worktree_eol_scans_bounded_files_past_former_total_limit() {
    use std::io::Read;

    let file_bytes = 31 * 1024 * 1024_u64;
    assert!(file_bytes * 9 > 256 * 1024 * 1024);
    for _ in 0..9 {
        let result = super::inspect_working_eol_reader(
            std::io::repeat(b'x').take(file_bytes),
            &CancellationToken::new(),
            ProviderDeadline::from_budget(Duration::from_secs(30)),
        )
        .unwrap();
        assert_eq!(result, Some(super::WorkingEol::Supported));
    }
}

#[test]
fn project_health_index_eol_accepts_large_aggregate_of_bounded_blobs() {
    struct LargeBlobMetadataRunner;

    impl ProcessRunner for LargeBlobMetadataRunner {
        fn run(&self, command: &ProcessCommand) -> Result<ProcessOutput, String> {
            AttributeProbeRunner::default().run(command)
        }

        fn run_with_input(
            &self,
            command: &ProcessCommand,
            input: &[u8],
        ) -> Result<ProcessOutput, String> {
            if command.args.iter().any(|arg| arg == "cat-file") {
                let rows = std::str::from_utf8(input)
                    .unwrap()
                    .lines()
                    .map(|oid| format!("{oid} blob {}\n", 31 * 1024 * 1024))
                    .collect::<String>();
                return Ok(success_output(&rows));
            }
            AttributeProbeRunner::default().run_with_input(command, input)
        }
    }

    let root = TempDir::new().unwrap();
    let resources = (0..9)
        .map(|index| super::RepositoryResource {
            source_set: "main".into(),
            repo_path: format!("src/Module{index}.bsl"),
            kind: RepositoryResourceKind::Text,
            blob_oid: format!("{index:040x}"),
        })
        .collect::<Vec<_>>();
    SourceResourcePolicyInspector::with_process_runner(
        &LargeBlobMetadataRunner,
        crate::inspection::TEST_RESPONSE_HANDOFF_WINDOW,
    )
    .resource_blob_sizes(
        root.path(),
        &resources.iter().collect::<Vec<_>>(),
        &CancellationToken::new(),
        ProviderDeadline::from_budget(Duration::from_secs(2)),
    )
    .expect("nine 31 MiB blobs are measured without a total-memory bound");
}

#[test]
fn continued_repository_eol_resumes_after_a_staged_timeout_and_rechecks_working_bytes() {
    let fixture = policy_fixture();
    fs::write(fixture.root.join("src/A.xml"), "<A>first</A>\n").unwrap();
    fs::write(fixture.root.join(".gitattributes"), "*.xml text eol=lf\n").unwrap();
    git(&fixture.root, &["add", ".gitattributes", "src"]);
    let cancellation = CancellationToken::new();
    let layout = SourceLayoutInspector::inspect(
        &fixture.context,
        &cancellation,
        ProviderDeadline::from_budget(Duration::from_secs(5)),
        &unica_workspace::discovery::ProjectDiscoveryReader::new(&fixture.context.workspace_root),
    )
    .unwrap();
    let repository = GitRepositoryInspector::new()
        .inspect_base(
            &fixture.context,
            &layout,
            &cancellation,
            ProviderDeadline::from_budget(Duration::from_secs(5)),
        )
        .unwrap();
    let root = repository.repository_root.as_ref().unwrap();
    let mut continuation = super::ResourceContinuation::new(&fixture.continuation_file());
    super::stop_staged_eol_after_for_test(1);
    let first = SourceResourcePolicyInspector::new()
        .inspect_excluding_continued(
            root,
            &layout.roots,
            &repository.entries,
            &Default::default(),
            &cancellation,
            ProviderDeadline::from_budget(Duration::from_secs(5)),
            Some(&mut continuation),
        )
        .unwrap();
    assert!(first.observations.iter().any(|observation| {
        observation.id == ProjectCheckId::RepositoryIndexEol
            && matches!(observation.outcome, ProjectCheckOutcome::NotRun { .. })
    }));
    assert_eq!(continuation.progress.staged_eol_retained, 1);

    continuation.progress = Default::default();
    let second = SourceResourcePolicyInspector::new()
        .inspect_excluding_continued(
            root,
            &layout.roots,
            &repository.entries,
            &Default::default(),
            &cancellation,
            ProviderDeadline::from_budget(Duration::from_secs(5)),
            Some(&mut continuation),
        )
        .unwrap();
    assert!(second.observations.iter().all(|observation| {
        observation.id != ProjectCheckId::RepositoryIndexEol
            || !matches!(observation.outcome, ProjectCheckOutcome::NotRun { .. })
    }));
    assert_eq!(continuation.progress.staged_eol_retained, 2);
    assert_eq!(continuation.progress.working_eol_retained, 2);

    fs::write(
        fixture.root.join("src/A.xml"),
        b"<A>mixed\r\nsecond\n</A>\n",
    )
    .unwrap();
    continuation.progress = Default::default();
    let changed = SourceResourcePolicyInspector::new()
        .inspect_excluding_continued(
            root,
            &layout.roots,
            &repository.entries,
            &Default::default(),
            &cancellation,
            ProviderDeadline::from_budget(Duration::from_secs(5)),
            Some(&mut continuation),
        )
        .unwrap();
    assert!(changed.facts.iter().any(|fact| {
        matches!(fact, ProjectHealthFact::MixedEol { path, .. } if path == "src/A.xml")
    }));

    let old_keys = continuation.index_eol_keys_for_test();
    git(&fixture.root, &["add", "src/A.xml"]);
    let restaged = GitRepositoryInspector::new()
        .inspect_base(
            &fixture.context,
            &layout,
            &cancellation,
            ProviderDeadline::from_budget(Duration::from_secs(5)),
        )
        .unwrap();
    continuation.progress = Default::default();
    SourceResourcePolicyInspector::new()
        .inspect_excluding_continued(
            root,
            &layout.roots,
            &restaged.entries,
            &Default::default(),
            &cancellation,
            ProviderDeadline::from_budget(Duration::from_secs(5)),
            Some(&mut continuation),
        )
        .unwrap();
    assert_ne!(continuation.index_eol_keys_for_test(), old_keys);
    assert_eq!(continuation.progress.staged_eol_retained, 2);

    let current_oid = restaged
        .entries
        .iter()
        .find(|entry| entry.repo_path == "src/A.xml")
        .unwrap()
        .blob_oid
        .clone();
    let old_keys = continuation.index_eol_keys_for_test();
    fs::write(fixture.root.join(".gitattributes"), "*.xml text eol=crlf\n").unwrap();
    git(&fixture.root, &["add", ".gitattributes"]);
    let changed_attributes = GitRepositoryInspector::new()
        .inspect_base(
            &fixture.context,
            &layout,
            &cancellation,
            ProviderDeadline::from_budget(Duration::from_secs(5)),
        )
        .unwrap();
    assert_eq!(
        changed_attributes
            .entries
            .iter()
            .find(|entry| entry.repo_path == "src/A.xml")
            .unwrap()
            .blob_oid,
        current_oid
    );
    continuation.progress = Default::default();
    SourceResourcePolicyInspector::new()
        .inspect_excluding_continued(
            root,
            &layout.roots,
            &changed_attributes.entries,
            &Default::default(),
            &cancellation,
            ProviderDeadline::from_budget(Duration::from_secs(5)),
            Some(&mut continuation),
        )
        .unwrap();
    assert_ne!(continuation.index_eol_keys_for_test(), old_keys);
    assert_eq!(continuation.progress.staged_eol_retained, 2);
}

#[test]
fn continued_repository_eol_uses_bounded_git_batches() {
    #[derive(Default)]
    struct BatchCountingRunner {
        eol_batch_sizes: RefCell<Vec<usize>>,
    }

    impl ProcessRunner for BatchCountingRunner {
        fn run(&self, command: &ProcessCommand) -> Result<ProcessOutput, String> {
            let output = unica_process::runner::system_process_runner().run(command)?;
            if command.args.iter().any(|arg| arg == "--eol") {
                self.eol_batch_sizes
                    .borrow_mut()
                    .push(output.stdout.matches('\0').count());
            }
            Ok(output)
        }

        fn run_with_input(
            &self,
            command: &ProcessCommand,
            input: &[u8],
        ) -> Result<ProcessOutput, String> {
            unica_process::runner::system_process_runner().run_with_input(command, input)
        }
    }

    let fixture = policy_fixture();
    fs::write(fixture.root.join(".gitattributes"), "*.xml text eol=lf\n").unwrap();
    for number in 0..129 {
        fs::write(
            fixture.root.join(format!("src/File{number:03}.xml")),
            format!("<Object name=\"{number}\"/>\n"),
        )
        .unwrap();
    }
    git(&fixture.root, &["add", ".gitattributes", "src"]);
    let cancellation = CancellationToken::new();
    let deadline = ProviderDeadline::from_budget(Duration::from_secs(15));
    let layout = SourceLayoutInspector::inspect(
        &fixture.context,
        &cancellation,
        deadline,
        &unica_workspace::discovery::ProjectDiscoveryReader::new(&fixture.context.workspace_root),
    )
    .unwrap();
    let repository = GitRepositoryInspector::new()
        .inspect_base(&fixture.context, &layout, &cancellation, deadline)
        .unwrap();
    let runner = BatchCountingRunner::default();
    let mut continuation = super::ResourceContinuation::new(&fixture.continuation_file());
    let inspection = SourceResourcePolicyInspector::with_process_runner(
        &runner,
        crate::inspection::TEST_RESPONSE_HANDOFF_WINDOW,
    )
    .inspect_excluding_continued(
        repository.repository_root.as_ref().unwrap(),
        &layout.roots,
        &repository.entries,
        &Default::default(),
        &cancellation,
        deadline,
        Some(&mut continuation),
    )
    .unwrap();
    assert!(inspection.observations.iter().any(|observation| {
        observation.id == ProjectCheckId::RepositoryIndexEol
            && !matches!(observation.outcome, ProjectCheckOutcome::NotRun { .. })
    }));
    assert_eq!(*runner.eol_batch_sizes.borrow(), vec![64, 64, 2]);
    assert_eq!(continuation.index_eol_keys_for_test().len(), 130);
}

#[test]
fn continued_repository_eol_resumes_on_disk_past_the_former_evidence_capacity() {
    // 22 full pages of staged text resources and Configuration.xml: about 90
    // 000 paths, past the roughly 80 000 text resources whose evidence fitted
    // the former 32 MiB in memory. Working files are absent.
    const PAGES: usize = 22;
    const FIRST_RUN: usize = 11 * super::RESOURCE_PAGE_PATHS;

    #[derive(Default)]
    struct EolBatchCountingRunner {
        eol_batches: Cell<usize>,
    }

    impl ProcessRunner for EolBatchCountingRunner {
        fn run(&self, command: &ProcessCommand) -> Result<ProcessOutput, String> {
            if command.args.iter().any(|arg| arg == "--eol") {
                self.eol_batches.set(self.eol_batches.get() + 1);
            }
            unica_process::runner::system_process_runner().run(command)
        }

        fn run_with_input(
            &self,
            command: &ProcessCommand,
            input: &[u8],
        ) -> Result<ProcessOutput, String> {
            unica_process::runner::system_process_runner().run_with_input(command, input)
        }
    }

    let fixture = policy_fixture();
    fs::write(fixture.root.join(".gitattributes"), "*.xml text eol=lf\n").unwrap();
    git(&fixture.root, &["add", ".gitattributes", "src"]);
    let files = PAGES * super::RESOURCE_PAGE_PATHS;
    stage_synthetic_paths(
        &fixture.root,
        (0..files).map(|number| format!("src/Files/{number:05}.xml")),
    );
    let text_resources = files + 1;
    let cancellation = CancellationToken::new();
    let deadline = ProviderDeadline::no_deadline();
    let layout = SourceLayoutInspector::inspect(
        &fixture.context,
        &cancellation,
        deadline,
        &unica_workspace::discovery::ProjectDiscoveryReader::new(&fixture.context.workspace_root),
    )
    .unwrap();
    let repository = GitRepositoryInspector::new()
        .inspect_base(&fixture.context, &layout, &cancellation, deadline)
        .unwrap();
    let root = repository.repository_root.as_ref().unwrap();
    let inspect = |runner: &EolBatchCountingRunner,
                   continuation: &mut super::ResourceContinuation| {
        SourceResourcePolicyInspector::with_process_runner(
            runner,
            crate::inspection::TEST_RESPONSE_HANDOFF_WINDOW,
        )
        .inspect_excluding_continued(
            root,
            &layout.roots,
            &repository.entries,
            &Default::default(),
            &cancellation,
            deadline,
            Some(continuation),
        )
        .unwrap()
    };
    let index_eol_not_run = |inspection: &super::RepositoryPolicyInspection| {
        inspection.observations.iter().any(|observation| {
            observation.id == ProjectCheckId::RepositoryIndexEol
                && matches!(observation.outcome, ProjectCheckOutcome::NotRun { .. })
        })
    };

    let mut continuation = super::ResourceContinuation::new(&fixture.continuation_file());
    let first = EolBatchCountingRunner::default();
    super::stop_staged_eol_after_for_test(FIRST_RUN);
    assert!(index_eol_not_run(&inspect(&first, &mut continuation)));
    assert_eq!(first.eol_batches.get(), FIRST_RUN / 64);
    assert_eq!(continuation.index_eol_keys_for_test().len(), FIRST_RUN);

    // A new continuation over the same cache finds the evidence on disk and
    // inspects only the paths the first call did not reach.
    let mut continued = super::ResourceContinuation::new(&fixture.continuation_file());
    let second = EolBatchCountingRunner::default();
    assert!(!index_eol_not_run(&inspect(&second, &mut continued)));
    assert_eq!(
        second.eol_batches.get(),
        (text_resources - FIRST_RUN).div_ceil(64)
    );
    assert_eq!(continued.progress.staged_eol_retained, text_resources);
    assert_eq!(continued.index_eol_keys_for_test().len(), text_resources);
}

#[test]
fn inspection_without_continuation_rejects_an_index_changed_between_pages() {
    // A check that holds no continuation still ends with the index fence:
    // the index changes after the first of two pages was inspected.
    struct IndexChangingRunner {
        root: PathBuf,
        attribute_probes: Cell<usize>,
    }

    impl ProcessRunner for IndexChangingRunner {
        fn run(&self, command: &ProcessCommand) -> Result<ProcessOutput, String> {
            unica_process::runner::system_process_runner().run(command)
        }

        fn run_with_input(
            &self,
            command: &ProcessCommand,
            input: &[u8],
        ) -> Result<ProcessOutput, String> {
            if command.args.iter().any(|arg| arg == "check-attr") {
                let probe = self.attribute_probes.get() + 1;
                self.attribute_probes.set(probe);
                // Probes 1 and 2 belong to the first page.
                if probe == 3 {
                    stage_synthetic_paths(&self.root, std::iter::once("late.dat".to_string()));
                }
            }
            unica_process::runner::system_process_runner().run_with_input(command, input)
        }
    }

    let fixture = policy_fixture();
    fs::write(
        fixture.root.join(".gitattributes"),
        "*.xml text eol=lf\n*.bin -text\n",
    )
    .unwrap();
    git(&fixture.root, &["add", ".gitattributes", "src"]);
    stage_synthetic_paths(
        &fixture.root,
        (0..super::RESOURCE_PAGE_PATHS).map(|number| format!("src/Blobs/{number:05}.bin")),
    );
    let cancellation = CancellationToken::new();
    let deadline = ProviderDeadline::no_deadline();
    let layout = SourceLayoutInspector::inspect(
        &fixture.context,
        &cancellation,
        deadline,
        &unica_workspace::discovery::ProjectDiscoveryReader::new(&fixture.context.workspace_root),
    )
    .unwrap();
    let repository = GitRepositoryInspector::new()
        .inspect_base(&fixture.context, &layout, &cancellation, deadline)
        .unwrap();
    let runner = IndexChangingRunner {
        root: fixture.root.clone(),
        attribute_probes: Cell::new(0),
    };

    let inspection = SourceResourcePolicyInspector::with_process_runner(
        &runner,
        crate::inspection::TEST_RESPONSE_HANDOFF_WINDOW,
    )
    .inspect(
        repository.repository_root.as_ref().unwrap(),
        &layout.roots,
        &repository.entries,
        &cancellation,
        deadline,
    )
    .unwrap();

    assert!(
        runner.attribute_probes.get() > 3,
        "both pages were inspected"
    );
    assert!(inspection.observations.iter().any(|observation| {
        observation.id == ProjectCheckId::RepositoryAttributes
            && matches!(observation.outcome, ProjectCheckOutcome::NotRun { .. })
    }));
    assert!(inspection.facts.iter().any(|fact| matches!(
        fact,
        ProjectHealthFact::GitInspectionIncomplete {
            check: ProjectCheckId::RepositoryAttributes,
            reason,
            ..
        } if reason == "Git index changed during resource inspection"
    )));
}

#[test]
fn index_eol_scanner_matches_git_ls_files_eol_at_every_piece_boundary() {
    let mut cases: Vec<(&str, Vec<u8>)> = vec![
        ("empty", Vec::new()),
        ("lf", b"a\nb\n".to_vec()),
        ("crlf", b"a\r\nb\r\n".to_vec()),
        ("mixed", b"a\r\nb\n".to_vec()),
        ("lone-cr", b"a\rb".to_vec()),
        ("trailing-cr", b"a\r".to_vec()),
        ("only-cr", b"\r".to_vec()),
        ("cr-cr-lf", b"a\r\r\n".to_vec()),
        ("lf-crlf", b"\n\r\n".to_vec()),
        ("nul", b"a\0b\n".to_vec()),
        ("eof-only", b"\x1a".to_vec()),
        ("eof-tail", b"abc\n\x1a".to_vec()),
        ("eof-twice", b"\x1a\x1a".to_vec()),
        ("eof-inside", b"a\x1ab\n".to_vec()),
        ("del", b"a\x7f\n".to_vec()),
        ("printable-controls", b"\x1b\t\x08\x0c ok\n".to_vec()),
        ("high-bytes", (0x80..=0xff_u8).chain(*b"\n").collect()),
        ("no-eol", b"plain".to_vec()),
    ];
    for (name, nonprintable) in [("ratio-at", 1_usize), ("ratio-over", 2)] {
        let mut bytes = vec![b'x'; 128];
        bytes.extend(std::iter::repeat_n(1_u8, nonprintable));
        bytes.push(b'\n');
        cases.push((name, bytes));
    }
    let mut bytes = vec![b'x'; 127];
    bytes.extend_from_slice(b"\x01\n");
    cases.push(("ratio-under", bytes));

    let repository = TempDir::new().unwrap();
    git(repository.path(), &["init"]);
    for (name, bytes) in &cases {
        fs::write(repository.path().join(name), bytes).unwrap();
    }
    git(
        repository.path(),
        &["-c", "core.autocrlf=false", "add", "."],
    );
    let listing = Command::new("git")
        .args(["ls-files", "--eol", "-z"])
        .current_dir(repository.path())
        .output()
        .unwrap();
    assert!(listing.status.success());
    let expected = String::from_utf8(listing.stdout)
        .unwrap()
        .split_terminator('\0')
        .map(|record| {
            let (metadata, path) = record.split_once('\t').unwrap();
            let index = metadata.split_whitespace().next().unwrap();
            (
                path.to_owned(),
                index.strip_prefix("i/").unwrap().to_owned(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    for (name, bytes) in &cases {
        // Every split point, so a CR, LF or ^Z lands on a piece boundary.
        for split in 0..=bytes.len() {
            let mut scanner = super::IndexEolScanner::default();
            scanner.push(&bytes[..split]);
            scanner.push(&bytes[split..]);
            assert_eq!(
                scanner.finish().as_git(),
                expected[*name],
                "{name} split at {split}"
            );
        }
        let mut scanner = super::IndexEolScanner::default();
        for byte in bytes {
            scanner.push(std::slice::from_ref(byte));
        }
        assert_eq!(
            scanner.finish().as_git(),
            expected[*name],
            "{name} byte by byte"
        );
    }
}

#[test]
fn index_eol_of_staged_text_past_the_former_file_limit_is_a_finding() {
    // 33 MiB staged texts: larger than a whole `ls-files --eol` batch, so they
    // are scanned as streams, where the former 32 MiB limit left the index
    // EOL check incomplete.
    let fixture = policy_fixture();
    fs::write(fixture.root.join(".gitattributes"), "*.xml text eol=lf\n").unwrap();
    let line = b"<Element attribute=\"value\">text</Element>";
    let mut crlf = Vec::new();
    let mut mixed = Vec::new();
    while crlf.len() <= 33 * 1024 * 1024 {
        crlf.extend_from_slice(line);
        crlf.extend_from_slice(b"\r\n");
        mixed.extend_from_slice(line);
        mixed.extend_from_slice(b"\r\n");
    }
    mixed.extend_from_slice(b"tail\n");
    fs::write(fixture.root.join("src/Crlf.xml"), &crlf).unwrap();
    fs::write(fixture.root.join("src/Mixed.xml"), &mixed).unwrap();
    // Stage the bytes as they are; the policy is what the check judges.
    git(
        &fixture.root,
        &[
            "-c",
            "core.autocrlf=false",
            "-c",
            "core.safecrlf=false",
            "add",
            "--",
            ".gitattributes",
        ],
    );
    let oid = |bytes: &[u8]| {
        let mut child = Command::new("git")
            .args(["hash-object", "-w", "--stdin"])
            .current_dir(&fixture.root)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        std::io::Write::write_all(child.stdin.as_mut().unwrap(), bytes).unwrap();
        let output = child.wait_with_output().unwrap();
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    };
    let (crlf_oid, mixed_oid) = (oid(&crlf), oid(&mixed));
    git(
        &fixture.root,
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            &format!("100644,{crlf_oid},src/Crlf.xml"),
        ],
    );
    git(
        &fixture.root,
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            &format!("100644,{mixed_oid},src/Mixed.xml"),
        ],
    );
    git(&fixture.root, &["add", "src/Configuration.xml"]);
    let cancellation = CancellationToken::new();
    let deadline = ProviderDeadline::no_deadline();
    let layout = SourceLayoutInspector::inspect(
        &fixture.context,
        &cancellation,
        deadline,
        &unica_workspace::discovery::ProjectDiscoveryReader::new(&fixture.context.workspace_root),
    )
    .unwrap();
    let repository = GitRepositoryInspector::new()
        .inspect_base(&fixture.context, &layout, &cancellation, deadline)
        .unwrap();
    let root = repository.repository_root.as_ref().unwrap();
    let mut continuation = super::ResourceContinuation::new(&fixture.continuation_file());
    for continued in [false, true] {
        let inspection = SourceResourcePolicyInspector::new()
            .inspect_excluding_continued(
                root,
                &layout.roots,
                &repository.entries,
                &Default::default(),
                &cancellation,
                deadline,
                continued.then_some(&mut continuation),
            )
            .unwrap();
        assert!(
            inspection.observations.iter().any(|observation| {
                observation.id == ProjectCheckId::RepositoryIndexEol
                    && observation.source_set.is_none()
                    && matches!(observation.outcome, ProjectCheckOutcome::Completed)
            }),
            "continued={continued}: {:?}",
            inspection.observations
        );
        for (path, expected) in [("src/Crlf.xml", "crlf"), ("src/Mixed.xml", "mixed")] {
            assert!(
                inspection.facts.iter().any(|fact| matches!(
                    fact,
                    ProjectHealthFact::IndexEolNotLf { path: actual, observed, .. }
                        if actual == path && observed == expected
                )),
                "continued={continued}, {path}: {:?}",
                inspection.facts
            );
        }
    }
}

fn inspect_staged_policy(fixture: &PolicyFixture) -> super::RepositoryPolicyInspection {
    let cancellation = CancellationToken::new();
    let deadline = ProviderDeadline::no_deadline();
    let layout = SourceLayoutInspector::inspect(
        &fixture.context,
        &cancellation,
        deadline,
        &unica_workspace::discovery::ProjectDiscoveryReader::new(&fixture.context.workspace_root),
    )
    .unwrap();
    let repository = GitRepositoryInspector::new()
        .inspect_base(&fixture.context, &layout, &cancellation, deadline)
        .unwrap();
    SourceResourcePolicyInspector::new()
        .inspect(
            repository.repository_root.as_ref().unwrap(),
            &layout.roots,
            &repository.entries,
            &cancellation,
            deadline,
        )
        .unwrap()
}

#[test]
fn staged_attribute_policy_past_the_former_file_and_byte_limits_is_checked() {
    // 1025 staged .gitattributes and more than 8 MiB of policy, past the
    // former 1024-file and 8 MiB refusals: Git reads the policy itself.
    let fixture = policy_fixture();
    let padding = "# portable text policy\n".repeat(9 * 1024 * 1024 / 23);
    fs::write(
        fixture.root.join(".gitattributes"),
        format!("*.xml text eol=lf\n{padding}"),
    )
    .unwrap();
    for number in 0..1024 {
        let directory = fixture.root.join(format!("policy/{number:04}"));
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join(".gitattributes"), "*.dat -text\n").unwrap();
    }
    git(&fixture.root, &["add", ".gitattributes", "policy", "src"]);

    let inspection = inspect_staged_policy(&fixture);

    assert!(
        inspection.observations.iter().any(|observation| {
            observation.id == ProjectCheckId::RepositoryAttributes
                && observation.source_set.is_none()
                && matches!(observation.outcome, ProjectCheckOutcome::Completed)
        }),
        "{:?}",
        inspection.observations
    );
    assert!(!inspection
        .facts
        .iter()
        .any(|fact| matches!(fact, ProjectHealthFact::TextPolicyMissing { .. })));
}

#[test]
fn attribute_policy_that_git_skips_is_reported_not_trusted() {
    // Git ignores an attribute line over 2048 bytes (and a file of 100 MiB
    // or more) with a warning. The skipped rule is the only text policy, so
    // trusting the answer would report missing policy for a project that
    // has one; the check must say the policy could not be read instead.
    let fixture = policy_fixture();
    fs::write(
        fixture.root.join(".gitattributes"),
        format!("*.xml text eol=lf {}\n", "x".repeat(3000)),
    )
    .unwrap();
    git(&fixture.root, &["add", ".gitattributes", "src"]);

    let inspection = inspect_staged_policy(&fixture);

    assert!(
        inspection.facts.iter().any(|fact| matches!(
            fact,
            ProjectHealthFact::GitInspectionIncomplete {
                check: ProjectCheckId::RepositoryAttributes,
                reason,
                ..
            } if reason.contains("ignoring overly long attributes line")
        )),
        "{:?}",
        inspection.facts
    );
    assert!(!inspection
        .facts
        .iter()
        .any(|fact| matches!(fact, ProjectHealthFact::TextPolicyMissing { .. })));
}

#[test]
fn root_check_continuations_live_in_their_directory_and_age_out() {
    let state = TempDir::new().unwrap();
    let directory = state.path().join("project-health");
    fs::create_dir_all(&directory).unwrap();
    let abandoned = directory.join("abandoned.bin");
    let recent = directory.join("recent.bin");
    fs::write(&abandoned, b"old").unwrap();
    fs::write(&recent, b"new").unwrap();
    let modified = std::time::SystemTime::now() - Duration::from_secs(3 * 3600);
    fs::File::options()
        .write(true)
        .open(&abandoned)
        .unwrap()
        .set_modified(modified)
        .unwrap();

    let store = super::RootCheckContinuationStore::in_directory(
        directory.clone(),
        Duration::from_secs(3600),
    );

    assert!(!abandoned.exists(), "a file past its retention is removed");
    assert!(recent.exists(), "a file within its retention is kept");
    let workspace = TempDir::new().unwrap();
    let continuation = store.for_workspace(workspace.path()).unwrap();
    let file = continuation.lock().unwrap().file.clone();
    assert_eq!(file.parent(), Some(directory.as_path()));
    let name = file.file_name().unwrap().to_str().unwrap();
    assert!(
        name.len() == 36
            && name.ends_with(".bin")
            && name[..32].bytes().all(|byte| byte.is_ascii_hexdigit()),
        "{name}"
    );
    assert!(super::RootCheckContinuationStore::default()
        .for_workspace(workspace.path())
        .is_none());
    // A missing directory is no error for the cleanup.
    super::RootCheckContinuationStore::in_directory(
        state.path().join("absent"),
        Duration::from_secs(3600),
    );
}

#[test]
fn continued_repository_eol_halves_a_timed_out_batch_on_retry() {
    struct TimeoutFirstBatchRunner {
        timed_out: Cell<bool>,
        eol_batch_sizes: RefCell<Vec<usize>>,
    }

    impl ProcessRunner for TimeoutFirstBatchRunner {
        fn run(&self, command: &ProcessCommand) -> Result<ProcessOutput, String> {
            let mut output = unica_process::runner::system_process_runner().run(command)?;
            if command.args.iter().any(|arg| arg == "--eol") {
                if !self.timed_out.get() {
                    self.timed_out.set(true);
                    let end_of_first_record = output.stdout.find('\0').unwrap() + 1;
                    output.stdout.truncate(end_of_first_record);
                    output.status_success = false;
                    output.timed_out = true;
                    return Ok(output);
                }
                self.eol_batch_sizes
                    .borrow_mut()
                    .push(output.stdout.matches('\0').count());
            }
            Ok(output)
        }

        fn run_with_input(
            &self,
            command: &ProcessCommand,
            input: &[u8],
        ) -> Result<ProcessOutput, String> {
            unica_process::runner::system_process_runner().run_with_input(command, input)
        }
    }

    let fixture = policy_fixture();
    fs::write(fixture.root.join(".gitattributes"), "*.xml text eol=lf\n").unwrap();
    for number in 0..65 {
        fs::write(
            fixture.root.join(format!("src/File{number:03}.xml")),
            format!("<Object name=\"{number}\"/>\n"),
        )
        .unwrap();
    }
    git(&fixture.root, &["add", ".gitattributes", "src"]);
    let cancellation = CancellationToken::new();
    let layout = SourceLayoutInspector::inspect(
        &fixture.context,
        &cancellation,
        ProviderDeadline::from_budget(Duration::from_secs(15)),
        &unica_workspace::discovery::ProjectDiscoveryReader::new(&fixture.context.workspace_root),
    )
    .unwrap();
    let repository = GitRepositoryInspector::new()
        .inspect_base(
            &fixture.context,
            &layout,
            &cancellation,
            ProviderDeadline::from_budget(Duration::from_secs(15)),
        )
        .unwrap();
    let root = repository.repository_root.as_ref().unwrap();
    let runner = TimeoutFirstBatchRunner {
        timed_out: Cell::new(false),
        eol_batch_sizes: RefCell::new(Vec::new()),
    };
    let mut continuation = super::ResourceContinuation::new(&fixture.continuation_file());
    let first = SourceResourcePolicyInspector::with_process_runner(
        &runner,
        crate::inspection::TEST_RESPONSE_HANDOFF_WINDOW,
    )
    .inspect_excluding_continued(
        root,
        &layout.roots,
        &repository.entries,
        &Default::default(),
        &cancellation,
        ProviderDeadline::from_budget(Duration::from_secs(15)),
        Some(&mut continuation),
    )
    .unwrap();
    assert!(first.observations.iter().any(|observation| {
        observation.id == ProjectCheckId::RepositoryIndexEol
            && matches!(observation.outcome, ProjectCheckOutcome::NotRun { .. })
    }));
    assert!(continuation.index_eol_keys_for_test().is_empty());
    assert_eq!(continuation.index_eol_batch_files(), 32);

    continuation.progress = Default::default();
    let retry = SourceResourcePolicyInspector::with_process_runner(
        &runner,
        crate::inspection::TEST_RESPONSE_HANDOFF_WINDOW,
    )
    .inspect_excluding_continued(
        root,
        &layout.roots,
        &repository.entries,
        &Default::default(),
        &cancellation,
        ProviderDeadline::from_budget(Duration::from_secs(15)),
        Some(&mut continuation),
    )
    .unwrap();
    assert!(retry.observations.iter().any(|observation| {
        observation.id == ProjectCheckId::RepositoryIndexEol
            && !matches!(observation.outcome, ProjectCheckOutcome::NotRun { .. })
    }));
    assert_eq!(*runner.eol_batch_sizes.borrow(), vec![32, 32, 2]);
    assert_eq!(continuation.index_eol_keys_for_test().len(), 66);
    assert_eq!(continuation.index_eol_batch_files(), 64);
}

#[test]
fn continued_repository_eol_keeps_batch_width_after_short_first_budget() {
    static ORIGIN: OnceLock<Instant> = OnceLock::new();

    fn fixed_clock() -> Instant {
        *ORIGIN.get_or_init(Instant::now)
    }

    struct TimeoutFirstEolRunner(Cell<bool>);

    impl ProcessRunner for TimeoutFirstEolRunner {
        fn run(&self, command: &ProcessCommand) -> Result<ProcessOutput, String> {
            let mut output = unica_process::runner::system_process_runner().run(command)?;
            if command.args.iter().any(|arg| arg == "--eol") {
                self.0.set(true);
                output.status_success = false;
                output.timed_out = true;
            }
            Ok(output)
        }

        fn run_with_input(
            &self,
            command: &ProcessCommand,
            input: &[u8],
        ) -> Result<ProcessOutput, String> {
            unica_process::runner::system_process_runner().run_with_input(command, input)
        }
    }

    let fixture = policy_fixture();
    git(&fixture.root, &["add", "src"]);
    let cancellation = CancellationToken::new();
    let layout = SourceLayoutInspector::inspect(
        &fixture.context,
        &cancellation,
        ProviderDeadline::from_budget(Duration::from_secs(15)),
        &unica_workspace::discovery::ProjectDiscoveryReader::new(&fixture.context.workspace_root),
    )
    .unwrap();
    let repository = GitRepositoryInspector::new()
        .inspect_base(
            &fixture.context,
            &layout,
            &cancellation,
            ProviderDeadline::from_budget(Duration::from_secs(15)),
        )
        .unwrap();
    let runner = TimeoutFirstEolRunner(Cell::new(false));
    let mut continuation = super::ResourceContinuation::new(&fixture.continuation_file());
    let incomplete = SourceResourcePolicyInspector::with_process_runner(
        &runner,
        crate::inspection::TEST_RESPONSE_HANDOFF_WINDOW,
    )
    .inspect_excluding_continued(
        repository.repository_root.as_ref().unwrap(),
        &layout.roots,
        &repository.entries,
        &Default::default(),
        &cancellation,
        ProviderDeadline::with_clock(fixed_clock() + Duration::from_secs(3), fixed_clock),
        Some(&mut continuation),
    )
    .unwrap();
    assert!(
        runner.0.get(),
        "the first EOL Git batch must reach the runner"
    );
    assert!(incomplete.observations.iter().any(|observation| {
        observation.id == ProjectCheckId::RepositoryIndexEol
            && matches!(observation.outcome, ProjectCheckOutcome::NotRun { .. })
    }));
    assert!(continuation.index_eol_keys_for_test().is_empty());
    assert_eq!(continuation.index_eol_batch_files(), 64);
}

#[test]
fn continued_repository_eol_keeps_batch_width_across_later_timeouts() {
    struct TimeoutAfterSuccessfulBatchRunner {
        eol_calls: Cell<usize>,
        completed_batch_sizes: RefCell<Vec<usize>>,
    }

    impl ProcessRunner for TimeoutAfterSuccessfulBatchRunner {
        fn run(&self, command: &ProcessCommand) -> Result<ProcessOutput, String> {
            let mut output = unica_process::runner::system_process_runner().run(command)?;
            if command.args.iter().any(|arg| arg == "--eol") {
                let call = self.eol_calls.get() + 1;
                self.eol_calls.set(call);
                if matches!(call, 2 | 4) {
                    output.status_success = false;
                    output.timed_out = true;
                } else {
                    self.completed_batch_sizes
                        .borrow_mut()
                        .push(output.stdout.matches('\0').count());
                }
            }
            Ok(output)
        }

        fn run_with_input(
            &self,
            command: &ProcessCommand,
            input: &[u8],
        ) -> Result<ProcessOutput, String> {
            unica_process::runner::system_process_runner().run_with_input(command, input)
        }
    }

    let fixture = policy_fixture();
    fs::write(fixture.root.join(".gitattributes"), "*.xml text eol=lf\n").unwrap();
    for number in 0..199 {
        fs::write(
            fixture.root.join(format!("src/File{number:03}.xml")),
            format!("<Object name=\"{number}\"/>\n"),
        )
        .unwrap();
    }
    git(&fixture.root, &["add", ".gitattributes", "src"]);
    let cancellation = CancellationToken::new();
    let layout = SourceLayoutInspector::inspect(
        &fixture.context,
        &cancellation,
        ProviderDeadline::from_budget(Duration::from_secs(15)),
        &unica_workspace::discovery::ProjectDiscoveryReader::new(&fixture.context.workspace_root),
    )
    .unwrap();
    let repository = GitRepositoryInspector::new()
        .inspect_base(
            &fixture.context,
            &layout,
            &cancellation,
            ProviderDeadline::from_budget(Duration::from_secs(15)),
        )
        .unwrap();
    let root = repository.repository_root.as_ref().unwrap();
    let runner = TimeoutAfterSuccessfulBatchRunner {
        eol_calls: Cell::new(0),
        completed_batch_sizes: RefCell::new(Vec::new()),
    };
    let mut continuation = super::ResourceContinuation::new(&fixture.continuation_file());
    for expected_retained in [64, 128] {
        continuation.progress = Default::default();
        let incomplete = SourceResourcePolicyInspector::with_process_runner(
            &runner,
            crate::inspection::TEST_RESPONSE_HANDOFF_WINDOW,
        )
        .inspect_excluding_continued(
            root,
            &layout.roots,
            &repository.entries,
            &Default::default(),
            &cancellation,
            ProviderDeadline::from_budget(Duration::from_secs(15)),
            Some(&mut continuation),
        )
        .unwrap();
        assert!(incomplete.observations.iter().any(|observation| {
            observation.id == ProjectCheckId::RepositoryIndexEol
                && matches!(observation.outcome, ProjectCheckOutcome::NotRun { .. })
        }));
        assert_eq!(
            continuation.index_eol_keys_for_test().len(),
            expected_retained
        );
        assert_eq!(continuation.index_eol_batch_files(), 64);
    }

    continuation.progress = Default::default();
    let completed = SourceResourcePolicyInspector::with_process_runner(
        &runner,
        crate::inspection::TEST_RESPONSE_HANDOFF_WINDOW,
    )
    .inspect_excluding_continued(
        root,
        &layout.roots,
        &repository.entries,
        &Default::default(),
        &cancellation,
        ProviderDeadline::from_budget(Duration::from_secs(15)),
        Some(&mut continuation),
    )
    .unwrap();
    assert!(completed.observations.iter().any(|observation| {
        observation.id == ProjectCheckId::RepositoryIndexEol
            && !matches!(observation.outcome, ProjectCheckOutcome::NotRun { .. })
    }));
    assert_eq!(continuation.index_eol_keys_for_test().len(), 200);
    assert_eq!(*runner.completed_batch_sizes.borrow(), vec![64, 64, 64, 8]);
    assert_eq!(continuation.index_eol_batch_files(), 64);
}

#[test]
fn continued_repository_eol_keeps_pending_batch_size_when_deadline_expires_before_git() {
    thread_local! {
        static TICKS: Cell<u64> = const { Cell::new(0) };
    }
    static ORIGIN: OnceLock<Instant> = OnceLock::new();

    fn advancing_clock() -> Instant {
        let tick = TICKS.with(|ticks| {
            let tick = ticks.get();
            ticks.set(tick + 1);
            tick
        });
        *ORIGIN.get_or_init(Instant::now) + Duration::from_millis(tick)
    }

    let root = TempDir::new().unwrap();
    let resources = (0..2)
        .map(|number| super::RepositoryResource {
            source_set: "main".into(),
            repo_path: format!("src/File{number}.xml"),
            kind: RepositoryResourceKind::Text,
            blob_oid: format!("{number:040x}"),
        })
        .collect::<Vec<_>>();
    let attributes = resources
        .iter()
        .map(|resource| {
            (
                resource.repo_path.clone(),
                super::AttributeValues {
                    text: "set".into(),
                    eol: "lf".into(),
                    filter: "unspecified".into(),
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
    let blob_sizes = resources
        .iter()
        .map(|resource| (resource.blob_oid.clone(), 1_usize))
        .collect::<BTreeMap<_, _>>();
    TICKS.with(|ticks| ticks.set(0));
    let deadline = ProviderDeadline::with_clock(
        *ORIGIN.get_or_init(Instant::now) + Duration::from_millis(3),
        advancing_clock,
    );
    let mut continuation = super::ResourceContinuation::new(&root.path().join("continuation.bin"));
    let mut records = BTreeMap::new();
    let result = SourceResourcePolicyInspector::with_process_runner(
        &FailingRunner,
        crate::inspection::TEST_RESPONSE_HANDOFF_WINDOW,
    )
    .incremental_index_eol(
        root.path(),
        &resources.iter().collect::<Vec<_>>(),
        &blob_sizes,
        &attributes,
        &attributes,
        &mut records,
        &mut continuation,
        &CancellationToken::new(),
        deadline,
    );
    assert_eq!(result, Err(super::ResourceProtocolParseError::TimedOut));
    assert!(records.is_empty());
    assert_eq!(continuation.index_eol_batch_files(), 64);
}

#[test]
fn continued_repository_eol_rejects_extra_batch_record_without_cache_commit() {
    struct ExtraEolRecordRunner;

    impl ProcessRunner for ExtraEolRecordRunner {
        fn run(&self, command: &ProcessCommand) -> Result<ProcessOutput, String> {
            let mut output = unica_process::runner::system_process_runner().run(command)?;
            if command.args.iter().any(|arg| arg == "--eol") {
                output
                    .stdout
                    .push_str("i/lf w/lf attr/text\tsrc/Unexpected.xml\0");
            }
            Ok(output)
        }

        fn run_with_input(
            &self,
            command: &ProcessCommand,
            input: &[u8],
        ) -> Result<ProcessOutput, String> {
            unica_process::runner::system_process_runner().run_with_input(command, input)
        }
    }

    let fixture = policy_fixture();
    fs::write(fixture.root.join("src/A.xml"), "<A/>\n").unwrap();
    fs::write(fixture.root.join(".gitattributes"), "*.xml text eol=lf\n").unwrap();
    git(&fixture.root, &["add", ".gitattributes", "src"]);
    let cancellation = CancellationToken::new();
    let deadline = ProviderDeadline::from_budget(Duration::from_secs(5));
    let layout = SourceLayoutInspector::inspect(
        &fixture.context,
        &cancellation,
        deadline,
        &unica_workspace::discovery::ProjectDiscoveryReader::new(&fixture.context.workspace_root),
    )
    .unwrap();
    let repository = GitRepositoryInspector::new()
        .inspect_base(&fixture.context, &layout, &cancellation, deadline)
        .unwrap();
    let mut continuation = super::ResourceContinuation::new(&fixture.continuation_file());
    let inspection = SourceResourcePolicyInspector::with_process_runner(
        &ExtraEolRecordRunner,
        crate::inspection::TEST_RESPONSE_HANDOFF_WINDOW,
    )
    .inspect_excluding_continued(
        repository.repository_root.as_ref().unwrap(),
        &layout.roots,
        &repository.entries,
        &Default::default(),
        &cancellation,
        deadline,
        Some(&mut continuation),
    )
    .unwrap();
    assert!(inspection.facts.iter().any(|fact| matches!(
        fact,
        ProjectHealthFact::GitInspectionIncomplete {
            check: ProjectCheckId::RepositoryIndexEol,
            reason,
            ..
        } if reason.contains("unexpected path")
    )));
    assert!(continuation.index_eol_keys_for_test().is_empty());
}

#[test]
fn continued_repository_eol_bounds_batch_by_git_blob_sizes() {
    struct LargeMetadataRunner {
        shared_oid: String,
        eol_batch_sizes: RefCell<Vec<usize>>,
    }

    impl ProcessRunner for LargeMetadataRunner {
        fn run(&self, command: &ProcessCommand) -> Result<ProcessOutput, String> {
            let output = unica_process::runner::system_process_runner().run(command)?;
            if command.args.iter().any(|arg| arg == "--eol") {
                self.eol_batch_sizes
                    .borrow_mut()
                    .push(output.stdout.matches('\0').count());
            }
            Ok(output)
        }

        fn run_with_input(
            &self,
            command: &ProcessCommand,
            input: &[u8],
        ) -> Result<ProcessOutput, String> {
            let mut output =
                unica_process::runner::system_process_runner().run_with_input(command, input)?;
            if command
                .args
                .iter()
                .any(|arg| arg.starts_with("--batch-check="))
                && input == format!("{}\n", self.shared_oid).as_bytes()
            {
                output.stdout = format!("{} blob {}\n", self.shared_oid, 31 * 1024 * 1024);
            }
            Ok(output)
        }
    }

    let fixture = policy_fixture();
    for name in ["Configuration.xml", "A.xml", "B.xml"] {
        fs::write(fixture.root.join("src").join(name), "<Same/>\n").unwrap();
    }
    fs::write(fixture.root.join(".gitattributes"), "*.xml text eol=lf\n").unwrap();
    git(&fixture.root, &["add", ".gitattributes", "src"]);
    let cancellation = CancellationToken::new();
    let deadline = ProviderDeadline::from_budget(Duration::from_secs(5));
    let layout = SourceLayoutInspector::inspect(
        &fixture.context,
        &cancellation,
        deadline,
        &unica_workspace::discovery::ProjectDiscoveryReader::new(&fixture.context.workspace_root),
    )
    .unwrap();
    let repository = GitRepositoryInspector::new()
        .inspect_base(&fixture.context, &layout, &cancellation, deadline)
        .unwrap();
    let runner = LargeMetadataRunner {
        shared_oid: repository
            .entries
            .iter()
            .find(|entry| entry.repo_path == "src/A.xml")
            .unwrap()
            .blob_oid
            .clone()
            .unwrap(),
        eol_batch_sizes: RefCell::new(Vec::new()),
    };
    let mut continuation = super::ResourceContinuation::new(&fixture.continuation_file());
    let inspection = SourceResourcePolicyInspector::with_process_runner(
        &runner,
        crate::inspection::TEST_RESPONSE_HANDOFF_WINDOW,
    )
    .inspect_excluding_continued(
        repository.repository_root.as_ref().unwrap(),
        &layout.roots,
        &repository.entries,
        &Default::default(),
        &cancellation,
        deadline,
        Some(&mut continuation),
    )
    .unwrap();
    assert!(inspection.observations.iter().any(|observation| {
        observation.id == ProjectCheckId::RepositoryIndexEol
            && !matches!(observation.outcome, ProjectCheckOutcome::NotRun { .. })
    }));
    assert_eq!(*runner.eol_batch_sizes.borrow(), vec![1, 1, 1]);
    assert_eq!(continuation.index_eol_keys_for_test().len(), 3);
}

#[test]
fn continued_repository_eol_cancellation_keeps_only_complete_batches() {
    struct CancelSecondBatchRunner {
        eol_calls: Cell<usize>,
        cancellation: CancellationToken,
    }

    impl ProcessRunner for CancelSecondBatchRunner {
        fn run(&self, command: &ProcessCommand) -> Result<ProcessOutput, String> {
            let output = unica_process::runner::system_process_runner().run(command)?;
            if command.args.iter().any(|arg| arg == "--eol") {
                let calls = self.eol_calls.get() + 1;
                self.eol_calls.set(calls);
                if calls == 2 {
                    self.cancellation.cancel();
                }
            }
            Ok(output)
        }

        fn run_with_input(
            &self,
            command: &ProcessCommand,
            input: &[u8],
        ) -> Result<ProcessOutput, String> {
            unica_process::runner::system_process_runner().run_with_input(command, input)
        }
    }

    let fixture = policy_fixture();
    fs::write(fixture.root.join(".gitattributes"), "*.xml text eol=lf\n").unwrap();
    for number in 0..65 {
        fs::write(
            fixture.root.join(format!("src/File{number:03}.xml")),
            format!("<Object name=\"{number}\"/>\n"),
        )
        .unwrap();
    }
    git(&fixture.root, &["add", ".gitattributes", "src"]);
    let cancellation = CancellationToken::new();
    let layout = SourceLayoutInspector::inspect(
        &fixture.context,
        &cancellation,
        ProviderDeadline::from_budget(Duration::from_secs(15)),
        &unica_workspace::discovery::ProjectDiscoveryReader::new(&fixture.context.workspace_root),
    )
    .unwrap();
    let repository = GitRepositoryInspector::new()
        .inspect_base(
            &fixture.context,
            &layout,
            &cancellation,
            ProviderDeadline::from_budget(Duration::from_secs(15)),
        )
        .unwrap();
    let runner = CancelSecondBatchRunner {
        eol_calls: Cell::new(0),
        cancellation: cancellation.clone(),
    };
    let mut continuation = super::ResourceContinuation::new(&fixture.continuation_file());
    let inspection = SourceResourcePolicyInspector::with_process_runner(
        &runner,
        crate::inspection::TEST_RESPONSE_HANDOFF_WINDOW,
    )
    .inspect_excluding_continued(
        repository.repository_root.as_ref().unwrap(),
        &layout.roots,
        &repository.entries,
        &Default::default(),
        &cancellation,
        ProviderDeadline::from_budget(Duration::from_secs(15)),
        Some(&mut continuation),
    );
    assert!(matches!(
        inspection,
        Err(ProjectHealthInspectionError::Cancelled)
    ));
    assert_eq!(runner.eol_calls.get(), 2);
    assert_eq!(continuation.index_eol_keys_for_test().len(), 64);
}

#[test]
fn continued_staged_eol_malformed_protocol_keeps_attribute_findings() {
    struct MalformedEolRunner;

    impl ProcessRunner for MalformedEolRunner {
        fn run(&self, command: &ProcessCommand) -> Result<ProcessOutput, String> {
            if command.args.iter().any(|argument| argument == "--eol") {
                return Ok(success_output("malformed\0"));
            }
            unica_process::runner::system_process_runner().run(command)
        }

        fn run_with_input(
            &self,
            command: &ProcessCommand,
            input: &[u8],
        ) -> Result<ProcessOutput, String> {
            unica_process::runner::system_process_runner().run_with_input(command, input)
        }
    }

    let fixture = policy_fixture();
    fs::write(fixture.root.join("src/A.xml"), "<A/>\n").unwrap();
    fs::write(
        fixture.root.join(".gitattributes"),
        "src/Configuration.xml text eol=lf\n",
    )
    .unwrap();
    git(&fixture.root, &["add", ".gitattributes", "src"]);
    let cancellation = CancellationToken::new();
    let layout = SourceLayoutInspector::inspect(
        &fixture.context,
        &cancellation,
        ProviderDeadline::from_budget(Duration::from_secs(5)),
        &unica_workspace::discovery::ProjectDiscoveryReader::new(&fixture.context.workspace_root),
    )
    .unwrap();
    let repository = GitRepositoryInspector::new()
        .inspect_base(
            &fixture.context,
            &layout,
            &cancellation,
            ProviderDeadline::from_budget(Duration::from_secs(5)),
        )
        .unwrap();
    let mut continuation = super::ResourceContinuation::new(&fixture.continuation_file());
    let policy = SourceResourcePolicyInspector::with_process_runner(
        &MalformedEolRunner,
        crate::inspection::TEST_RESPONSE_HANDOFF_WINDOW,
    )
    .inspect_excluding_continued(
        repository.repository_root.as_ref().unwrap(),
        &layout.roots,
        &repository.entries,
        &Default::default(),
        &cancellation,
        ProviderDeadline::from_budget(Duration::from_secs(5)),
        Some(&mut continuation),
    )
    .unwrap();
    assert!(policy.facts.iter().any(|fact| {
        matches!(fact, ProjectHealthFact::TextPolicyMissing { path, .. } if path == "src/A.xml")
    }));
    assert!(policy.facts.iter().any(|fact| {
        matches!(
            fact,
            ProjectHealthFact::GitInspectionIncomplete {
                check: ProjectCheckId::RepositoryIndexEol,
                ..
            }
        )
    }));
    assert!(policy.observations.iter().any(|observation| {
        observation.id == ProjectCheckId::RepositoryAttributes
            && matches!(observation.outcome, ProjectCheckOutcome::Completed)
    }));
}

#[test]
fn final_index_change_invalidates_earlier_attribute_findings() {
    let fixture = policy_fixture();
    fs::write(fixture.root.join("src/A.xml"), "<A/>\n").unwrap();
    fs::write(
        fixture.root.join(".gitattributes"),
        "src/Configuration.xml text eol=lf\n",
    )
    .unwrap();
    git(&fixture.root, &["add", ".gitattributes", "src"]);
    let cancellation = CancellationToken::new();
    let layout = SourceLayoutInspector::inspect(
        &fixture.context,
        &cancellation,
        ProviderDeadline::from_budget(Duration::from_secs(5)),
        &unica_workspace::discovery::ProjectDiscoveryReader::new(&fixture.context.workspace_root),
    )
    .unwrap();
    let repository = GitRepositoryInspector::new()
        .inspect_base(
            &fixture.context,
            &layout,
            &cancellation,
            ProviderDeadline::from_budget(Duration::from_secs(5)),
        )
        .unwrap();
    let root = fixture.root.clone();
    super::BEFORE_WORKING_REVALIDATION.with(|slot| {
        *slot.borrow_mut() = Some(Box::new(move || {
            fs::write(root.join(".gitignore"), "*.scratch\n").unwrap();
            git(&root, &["add", ".gitignore"]);
        }));
    });
    let mut continuation = super::ResourceContinuation::new(&fixture.continuation_file());
    let policy = SourceResourcePolicyInspector::new()
        .inspect_excluding_continued(
            repository.repository_root.as_ref().unwrap(),
            &layout.roots,
            &repository.entries,
            &Default::default(),
            &cancellation,
            ProviderDeadline::from_budget(Duration::from_secs(5)),
            Some(&mut continuation),
        )
        .unwrap();
    assert!(policy.observations.iter().any(|observation| {
        observation.id == ProjectCheckId::RepositoryAttributes
            && matches!(observation.outcome, ProjectCheckOutcome::NotRun { .. })
    }));
    assert!(policy.facts.iter().any(|fact| {
        matches!(
            fact,
            ProjectHealthFact::GitInspectionIncomplete {
                check: ProjectCheckId::RepositoryAttributes,
                ..
            }
        )
    }));
    assert!(!policy
        .facts
        .iter()
        .any(|fact| { matches!(fact, ProjectHealthFact::TextPolicyMissing { .. }) }));
}

#[test]
fn continued_working_eol_resumes_inside_one_file_across_deadlines() {
    for (tail, expected) in [
        (b"\r\n".as_slice(), super::WorkingEol::Supported),
        (b"\r\nTAIL\n".as_slice(), super::WorkingEol::Mixed),
        (b"\r".as_slice(), super::WorkingEol::BareCr),
    ] {
        let temp = TempDir::new().unwrap();
        let root = fs::canonicalize(temp.path()).unwrap();
        let mut bytes = vec![b'x'; 64 * 1024 - 1];
        bytes.extend_from_slice(tail);
        fs::write(root.join("partial.xml"), &bytes).unwrap();
        let mut checkpoint = WorkingCheckpoint::default();
        let reads = bytes.len().div_ceil(64 * 1024);
        let mut completed = false;
        for step in 0..=reads {
            DEADLINE_CLOCK_TICKS.with(|ticks| ticks.set(0));
            let deadline = ProviderDeadline::with_clock(
                advancing_deadline_clock()
                    + Duration::from_millis(if step == reads { 4 } else { 3 }),
                advancing_deadline_clock,
            );
            let result =
                checkpoint.inspect(&root, "partial.xml", &CancellationToken::new(), deadline);
            match result {
                Err(super::WorkingEolInspectionError::TimedOut) => {
                    assert_eq!(checkpoint.progress.working_eol_retained, 0);
                    let Some(super::WorkingEvidence::Partial { scanner, .. }) = checkpoint.evidence
                    else {
                        panic!("an unfinished file is not completed evidence");
                    };
                    assert_eq!(
                        scanner.offset,
                        ((step + 1) * 64 * 1024).min(bytes.len()) as u64
                    );
                    if step < reads {
                        assert_eq!(DEADLINE_CLOCK_TICKS.with(|ticks| ticks.get()), 4);
                    }
                }
                Ok(eol) => {
                    assert_eq!(step, reads, "the entire file and EOF must be read");
                    assert_eq!(eol, Some(expected));
                    assert!(matches!(
                        checkpoint.evidence,
                        Some(super::WorkingEvidence::Complete { .. })
                    ));
                    assert_eq!(checkpoint.progress.working_eol_retained, 1);
                    completed = true;
                    break;
                }
                other => panic!("unexpected continued inspection: {other:?}"),
            }
        }
        assert!(completed, "each request restarted the same file prefix");
    }
}
#[test]
fn working_eol_scanner_keeps_exact_offsets_for_short_reads() {
    struct ShortReader(std::io::Cursor<&'static [u8]>);
    impl std::io::Read for ShortReader {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            std::io::Read::read(&mut self.0, &mut buffer[..1])
        }
    }
    let bytes = b"x\r\ny\n";
    let mut reader = ShortReader(std::io::Cursor::new(bytes));
    let mut scanner = super::WorkingEolScanner::default();
    for step in 0..=bytes.len() {
        DEADLINE_CLOCK_TICKS.with(|ticks| ticks.set(0));
        let deadline = ProviderDeadline::with_clock(
            advancing_deadline_clock()
                + Duration::from_millis(if step == bytes.len() { 4 } else { 3 }),
            advancing_deadline_clock,
        );
        let result = super::inspect_working_eol_reader_from_state(
            &mut reader,
            &mut scanner,
            &CancellationToken::new(),
            deadline,
        );
        assert_eq!(scanner.offset, reader.0.position());
        if step < bytes.len() {
            assert_eq!(result, Err(super::WorkingEolInspectionError::TimedOut));
            assert_eq!(scanner.offset, (step + 1) as u64);
            if bytes[step] == b'\r' {
                assert!(scanner.previous_cr);
                assert!(!scanner.bare_cr);
            }
        } else {
            assert_eq!(result, Ok(Some(super::WorkingEol::Mixed)));
        }
    }
}

#[test]
fn working_eol_reader_rejects_cancellation_and_expiry_during_eof() {
    static NOW: AtomicUsize = AtomicUsize::new(0);
    fn clock() -> Instant {
        *DEADLINE_CLOCK_ORIGIN.get_or_init(Instant::now)
            + Duration::from_millis(NOW.load(Ordering::SeqCst) as u64)
    }
    struct EofReader {
        cancellation: CancellationToken,
        cancel: bool,
    }
    impl std::io::Read for EofReader {
        fn read(&mut self, _buffer: &mut [u8]) -> std::io::Result<usize> {
            if self.cancel {
                self.cancellation.cancel();
            } else {
                NOW.store(4, Ordering::SeqCst);
            }
            Ok(0)
        }
    }
    for cancel in [true, false] {
        NOW.store(0, Ordering::SeqCst);
        let cancellation = CancellationToken::new();
        let deadline = ProviderDeadline::with_clock(clock() + Duration::from_millis(3), clock);
        let mut scanner = super::WorkingEolScanner {
            previous_cr: true,
            ..Default::default()
        };
        let result = super::inspect_working_eol_reader_from_state(
            EofReader {
                cancellation: cancellation.clone(),
                cancel,
            },
            &mut scanner,
            &cancellation,
            deadline,
        );
        assert_eq!(
            result,
            Err(if cancel {
                super::WorkingEolInspectionError::Cancelled
            } else {
                super::WorkingEolInspectionError::TimedOut
            })
        );
        assert!(scanner.previous_cr);
        assert!(
            !scanner.bare_cr,
            "unverified EOF must not finish the pending CR"
        );
    }
}

#[test]
fn continued_working_eol_discards_partial_state_when_fingerprint_changes() {
    for mutation in ["prefix", "size", "replacement", "during_read"] {
        let temp = TempDir::new().unwrap();
        let root = fs::canonicalize(temp.path()).unwrap();
        let path = root.join("partial.xml");
        let mut bytes = vec![b'x'; 64 * 1024 - 1];
        bytes.extend_from_slice(b"\r\n");
        fs::write(&path, &bytes).unwrap();
        let before = super::working_file_fingerprint(
            &unica_workspace::test_support::read_file_from_fixture_file(
                fs::File::open(&path).unwrap(),
            ),
        )
        .unwrap();
        let changed_time = fs::metadata(&path)
            .unwrap()
            .modified()
            .unwrap()
            .checked_add(Duration::from_secs(1))
            .unwrap();
        let mut checkpoint = WorkingCheckpoint::default();
        if mutation == "during_read" {
            let changed_path = path.clone();
            let mut changed_bytes = bytes.clone();
            changed_bytes[0] = b'\n';
            super::AFTER_WORKING_EOL_BLOCK.with(|slot| {
                *slot.borrow_mut() = Some(Box::new(move || {
                    fs::write(&changed_path, changed_bytes).unwrap();
                    fs::OpenOptions::new()
                        .write(true)
                        .open(changed_path)
                        .unwrap()
                        .set_modified(changed_time)
                        .unwrap();
                    Err(super::WorkingEolInspectionError::TimedOut)
                }));
            });
        } else {
            super::stop_working_eol_after_block_for_test();
        }
        assert_eq!(
            checkpoint.inspect(
                &root,
                "partial.xml",
                &CancellationToken::new(),
                ProviderDeadline::from_budget(Duration::from_secs(5)),
            ),
            Err(super::WorkingEolInspectionError::TimedOut)
        );
        assert!(
            matches!(
                checkpoint.evidence,
                Some(super::WorkingEvidence::Partial { fingerprint, .. }) if fingerprint == before
            ),
            "partial state stays anchored to the original preimage"
        );
        let expected = if mutation == "size" {
            fs::write(&path, b"new\r\n").unwrap();
            super::WorkingEol::Supported
        } else {
            if mutation != "during_read" {
                bytes[0] = b'\n';
                if mutation == "replacement" {
                    let replacement = root.join("replacement.xml");
                    fs::write(&replacement, &bytes).unwrap();
                    fs::rename(replacement, &path).unwrap();
                } else {
                    fs::write(&path, &bytes).unwrap();
                    fs::OpenOptions::new()
                        .write(true)
                        .open(&path)
                        .unwrap()
                        .set_modified(changed_time)
                        .unwrap();
                }
            }
            super::WorkingEol::Mixed
        };
        let current = super::working_file_fingerprint(
            &unica_workspace::test_support::read_file_from_fixture_file(
                fs::File::open(&path).unwrap(),
            ),
        )
        .unwrap();
        assert_ne!(current, before);
        let result = checkpoint
            .inspect(
                &root,
                "partial.xml",
                &CancellationToken::new(),
                ProviderDeadline::from_budget(Duration::from_secs(5)),
            )
            .unwrap();
        assert_eq!(result, Some(expected));
        assert_eq!(
            checkpoint.evidence,
            Some(super::WorkingEvidence::Complete {
                fingerprint: current,
                eol: expected,
            })
        );
    }
}
#[test]
fn continued_working_eol_never_completes_an_invalid_or_cancelled_partial() {
    for case in ["offset", "cancelled", "removed", "before_read"] {
        let temp = TempDir::new().unwrap();
        let root = fs::canonicalize(temp.path()).unwrap();
        let path = root.join("partial.xml");
        fs::write(&path, vec![b'x'; 128 * 1024]).unwrap();
        let mut checkpoint = WorkingCheckpoint::default();
        let cancellation = CancellationToken::new();
        let deadline = if case == "before_read" {
            ProviderDeadline::from_budget(Duration::ZERO)
        } else {
            super::stop_working_eol_after_block_for_test();
            ProviderDeadline::from_budget(Duration::from_secs(5))
        };
        assert_eq!(
            checkpoint.inspect(&root, "partial.xml", &cancellation, deadline),
            Err(super::WorkingEolInspectionError::TimedOut)
        );
        assert_eq!(checkpoint.progress.working_eol_retained, 0);
        let Some(super::WorkingEvidence::Partial {
            fingerprint,
            mut scanner,
        }) = checkpoint.evidence
        else {
            panic!("an unfinished file is not completed evidence");
        };
        if case == "offset" {
            scanner.offset = fs::metadata(&path).unwrap().len() + 1;
            checkpoint.evidence = Some(super::WorkingEvidence::Partial {
                fingerprint,
                scanner,
            });
        } else if case == "cancelled" {
            cancellation.cancel();
        } else if case == "removed" {
            fs::remove_file(&path).unwrap();
        } else {
            assert_eq!(scanner.offset, 0);
        }
        let result = checkpoint.inspect(
            &root,
            "partial.xml",
            &cancellation,
            ProviderDeadline::from_budget(Duration::from_secs(5)),
        );
        match case {
            "offset" => assert!(matches!(
                result,
                Err(super::WorkingEolInspectionError::Incomplete(_))
            )),
            "cancelled" => assert_eq!(result, Err(super::WorkingEolInspectionError::Cancelled)),
            "removed" => assert_eq!(result, Ok(None)),
            _ => assert_eq!(result, Ok(Some(super::WorkingEol::Supported))),
        }
        match case {
            "removed" => assert_eq!(checkpoint.evidence, Some(super::WorkingEvidence::Absent)),
            "before_read" => assert!(matches!(
                checkpoint.evidence,
                Some(super::WorkingEvidence::Complete { .. })
            )),
            _ => assert_eq!(checkpoint.evidence, None),
        }
    }
}
#[test]
fn working_eol_scanner_keeps_its_offset_on_io_error_and_overflow() {
    struct Unreadable;
    impl std::io::Read for Unreadable {
        fn read(&mut self, _buffer: &mut [u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("injected working read failure"))
        }
    }
    let mut scanner = super::WorkingEolScanner::default();
    let result = super::inspect_working_eol_reader_from_state(
        Unreadable,
        &mut scanner,
        &CancellationToken::new(),
        ProviderDeadline::from_budget(Duration::from_secs(5)),
    );
    assert!(matches!(
        result,
        Err(super::WorkingEolInspectionError::Incomplete(_))
    ));
    assert_eq!(scanner.offset, 0);
    scanner.offset = u64::MAX;
    let result = super::inspect_working_eol_reader_from_state(
        std::io::Cursor::new(b"\n"),
        &mut scanner,
        &CancellationToken::new(),
        ProviderDeadline::from_budget(Duration::from_secs(5)),
    );
    assert!(matches!(
        result,
        Err(super::WorkingEolInspectionError::Incomplete(_))
    ));
    assert_eq!(scanner.offset, u64::MAX);
    assert!(!scanner.lf);
}

#[test]
fn continued_working_eol_rejects_a_new_file_and_late_cancellation() {
    let fixture = policy_fixture();
    let file = fixture.root.join("src/A.xml");
    fs::write(&file, "<A/>\n").unwrap();
    fs::write(fixture.root.join(".gitattributes"), "*.xml text eol=lf\n").unwrap();
    git(&fixture.root, &["add", ".gitattributes", "src"]);
    fs::remove_file(&file).unwrap();
    let cancellation = CancellationToken::new();
    let layout = SourceLayoutInspector::inspect(
        &fixture.context,
        &cancellation,
        ProviderDeadline::from_budget(Duration::from_secs(5)),
        &unica_workspace::discovery::ProjectDiscoveryReader::new(&fixture.context.workspace_root),
    )
    .unwrap();
    let repository = GitRepositoryInspector::new()
        .inspect_base(
            &fixture.context,
            &layout,
            &cancellation,
            ProviderDeadline::from_budget(Duration::from_secs(5)),
        )
        .unwrap();
    let root = repository.repository_root.as_ref().unwrap();
    let mut continuation = super::ResourceContinuation::new(&fixture.continuation_file());

    super::BEFORE_WORKING_REVALIDATION.with(|slot| {
        *slot.borrow_mut() = Some(Box::new(move || {
            fs::write(file, "<A>appeared</A>\n").unwrap();
        }));
    });
    let appeared = SourceResourcePolicyInspector::new()
        .inspect_excluding_continued(
            root,
            &layout.roots,
            &repository.entries,
            &Default::default(),
            &cancellation,
            ProviderDeadline::from_budget(Duration::from_secs(5)),
            Some(&mut continuation),
        )
        .unwrap();
    assert!(appeared.observations.iter().any(|observation| {
        observation.id == ProjectCheckId::RepositoryWorkingEol
            && matches!(observation.outcome, ProjectCheckOutcome::NotRun { .. })
    }));
    assert!(appeared.facts.iter().any(|fact| matches!(
        fact,
        ProjectHealthFact::GitInspectionIncomplete {
            check: ProjectCheckId::RepositoryWorkingEol,
            ..
        }
    )));

    continuation.progress = Default::default();
    let late_cancellation = cancellation.clone();
    super::BEFORE_WORKING_REVALIDATION.with(|slot| {
        *slot.borrow_mut() = Some(Box::new(move || late_cancellation.cancel()));
    });
    let cancelled = SourceResourcePolicyInspector::new().inspect_excluding_continued(
        root,
        &layout.roots,
        &repository.entries,
        &Default::default(),
        &cancellation,
        ProviderDeadline::from_budget(Duration::from_secs(5)),
        Some(&mut continuation),
    );
    assert!(matches!(
        cancelled,
        Err(ProjectHealthInspectionError::Cancelled)
    ));
}

#[test]
fn project_health_lfs_size_failure_is_not_run_but_advisory() {
    let fixture = policy_fixture();
    fs::write(fixture.root.join(".gitattributes"), "*.bin -text\n").unwrap();
    let target = fixture.root.join("target.bin");
    let link = fixture.root.join("src/Linked.bin");
    fs::write(&target, b"binary").unwrap();
    fs::write(&link, b"staged regular binary").unwrap();
    git(&fixture.root, &["add", ".gitattributes", "src/Linked.bin"]);
    fs::remove_file(&link).unwrap();
    match create_file_link_fixture_for_test(&target, &link).unwrap() {
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
    let inspection = inspect_policy(&fixture);

    assert!(
        inspection.facts.iter().any(|fact| matches!(
            fact,
            ProjectHealthFact::GitInspectionIncomplete {
                check: ProjectCheckId::RepositoryLfs,
                ..
            }
        )),
        "facts={:?}; observations={:?}",
        inspection.facts,
        inspection.observations
    );
    assert!(inspection.observations.iter().any(|observation| {
        observation.id == ProjectCheckId::RepositoryLfs
            && observation.source_set.is_none()
            && matches!(
                observation.outcome,
                crate::ProjectCheckOutcome::NotRun { .. }
            )
    }));
}

#[test]
fn project_health_lfs_size_does_not_follow_linked_parent_directory() {
    let fixture = policy_fixture();
    fs::write(fixture.root.join(".gitattributes"), "*.bin -text\n").unwrap();
    let staged_directory = fixture.root.join("src/Binaries");
    fs::create_dir_all(&staged_directory).unwrap();
    fs::write(staged_directory.join("Picture.bin"), b"small").unwrap();
    git(
        &fixture.root,
        &["add", ".gitattributes", "src/Binaries/Picture.bin"],
    );
    fs::remove_file(staged_directory.join("Picture.bin")).unwrap();
    fs::remove_dir(&staged_directory).unwrap();
    let external = fixture.root.join("external-binaries");
    fs::create_dir_all(&external).unwrap();
    fs::File::create(external.join("Picture.bin"))
        .unwrap()
        .set_len(LFS_SINGLE_FILE_THRESHOLD_BYTES)
        .unwrap();
    match create_directory_link_fixture_for_test(&external, &staged_directory).unwrap() {
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

    let inspection = inspect_policy(&fixture);

    assert!(
        inspection.facts.iter().any(|fact| matches!(
            fact,
            ProjectHealthFact::GitInspectionIncomplete {
                check,
                reason,
                ..
            } if matches!(check, ProjectCheckId::RepositoryAttributes | ProjectCheckId::RepositoryLfs)
                && (reason.contains("reparse point")
                    || reason.contains("symbolic links")
                    || reason.contains("linked or non-directory parent"))
        )),
        "facts={:?}; observations={:?}",
        inspection.facts,
        inspection.observations
    );
    assert!(inspection.observations.iter().any(|observation| {
        observation.id == ProjectCheckId::RepositoryLfs
            && observation.source_set.is_none()
            && matches!(observation.outcome, ProjectCheckOutcome::NotRun { .. })
    }));
    assert!(!inspection.facts.iter().any(|fact| matches!(
        fact,
        ProjectHealthFact::LfsConsider { largest_path, .. }
            if largest_path == "src/Binaries/Picture.bin"
    )));
}

#[test]
pub(crate) fn project_health_repository_policy_lfs_is_advisory_for_exact_large_binary() {
    let fixture = policy_fixture();
    fs::write(fixture.root.join(".gitattributes"), "*.bin -text\n").unwrap();
    let large = fs::File::create(fixture.root.join("src/Large.bin")).unwrap();
    large.set_len(LFS_SINGLE_FILE_THRESHOLD_BYTES).unwrap();
    git(&fixture.root, &["add", ".gitattributes", "src/Large.bin"]);

    let inspection = inspect_policy(&fixture);

    assert!(
        inspection.facts.iter().any(|fact| matches!(
            fact,
            ProjectHealthFact::LfsConsider { largest_path, .. }
                if largest_path == "src/Large.bin"
        )),
        "{:?}",
        inspection.facts
    );
}

thread_local! {
    static RESOURCE_POLICY_NOW: RefCell<Option<Instant>> = const { RefCell::new(None) };
    static RESOURCE_POLICY_EXPIRED: Cell<bool> = const { Cell::new(false) };
    static RESOURCE_POLICY_CHECKS_BEFORE_EXPIRY: Cell<usize> = const { Cell::new(0) };
}

fn resource_policy_now() -> Instant {
    RESOURCE_POLICY_NOW.with(|now| {
        let started = now.borrow().expect("resource policy clock initialized");
        if RESOURCE_POLICY_EXPIRED.with(Cell::get) {
            RESOURCE_POLICY_CHECKS_BEFORE_EXPIRY.with(|remaining| {
                let remaining_checks = remaining.get();
                if remaining_checks == 0 {
                    started + Duration::from_secs(1)
                } else {
                    remaining.set(remaining_checks - 1);
                    started
                }
            })
        } else {
            started
        }
    })
}

#[test]
fn project_health_binary_scan_honors_deadline_between_resources() {
    let fixture = policy_fixture();
    fs::write(fixture.root.join("src/Picture.bin"), b"binary").unwrap();
    let cancellation = CancellationToken::new();
    let layout = SourceLayoutInspector::inspect(
        &fixture.context,
        &cancellation,
        ProviderDeadline::from_budget(Duration::from_secs(2)),
        &unica_workspace::discovery::ProjectDiscoveryReader::new(&fixture.context.workspace_root),
    )
    .unwrap();
    let started = Instant::now();
    RESOURCE_POLICY_NOW.with(|now| *now.borrow_mut() = Some(started));
    RESOURCE_POLICY_EXPIRED.with(|expired| expired.set(false));
    // The staged attribute probe expires the clock. Its parse and the
    // attribute policy check it four times; the binary scan meets the expiry.
    RESOURCE_POLICY_CHECKS_BEFORE_EXPIRY.with(|remaining| remaining.set(4));
    let runner = ResourceLoopCheckpointRunner {
        cancellation: None,
        attribute_calls: Cell::new(0),
    };
    let binary_entry = GitIndexEntry {
        repo_path: "src/Picture.bin".into(),
        blob_oid: Some("a".repeat(40)),
        mode: Some("100644".into()),
    };

    let inspection = SourceResourcePolicyInspector::with_process_runner(
        &runner,
        crate::inspection::TEST_RESPONSE_HANDOFF_WINDOW,
    )
    .inspect(
        &fixture.root,
        &layout.roots,
        &[binary_entry],
        &cancellation,
        ProviderDeadline::with_clock(started + Duration::from_millis(10), resource_policy_now),
    )
    .unwrap();

    assert!(inspection.observations.iter().any(|observation| {
        observation.id == ProjectCheckId::RepositoryLfs
            && matches!(
                observation.outcome,
                crate::ProjectCheckOutcome::NotRun { .. }
            )
    }));
    assert!(inspection.facts.iter().any(|fact| matches!(
        fact,
        ProjectHealthFact::GitInspectionTimeout {
            check: ProjectCheckId::RepositoryLfs,
            ..
        }
    )));
}

#[test]
fn project_health_binary_scan_honors_cancellation_between_resources() {
    let fixture = policy_fixture();
    fs::write(fixture.root.join("src/Picture.bin"), b"binary").unwrap();
    let cancellation = CancellationToken::new();
    let layout = SourceLayoutInspector::inspect(
        &fixture.context,
        &cancellation,
        ProviderDeadline::from_budget(Duration::from_secs(2)),
        &unica_workspace::discovery::ProjectDiscoveryReader::new(&fixture.context.workspace_root),
    )
    .unwrap();
    let runner = ResourceLoopCheckpointRunner {
        cancellation: Some(cancellation.clone()),
        attribute_calls: Cell::new(0),
    };

    let result = SourceResourcePolicyInspector::with_process_runner(
        &runner,
        crate::inspection::TEST_RESPONSE_HANDOFF_WINDOW,
    )
    .inspect(
        &fixture.root,
        &layout.roots,
        &[GitIndexEntry {
            repo_path: "src/Picture.bin".into(),
            blob_oid: Some("a".repeat(40)),
            mode: Some("100644".into()),
        }],
        &cancellation,
        ProviderDeadline::from_budget(Duration::from_secs(2)),
    );

    assert!(matches!(
        result,
        Err(crate::ProjectHealthInspectionError::Cancelled)
    ));
}

#[test]
fn project_health_repository_policy_composes_into_a_valid_domain_snapshot() {
    let fixture = policy_fixture();
    fs::write(fixture.root.join("src/A.xml"), "<A/>\n").unwrap();
    git(&fixture.root, &["add", "src/A.xml"]);
    let cancellation = CancellationToken::new();
    let layout = SourceLayoutInspector::inspect(
        &fixture.context,
        &cancellation,
        ProviderDeadline::from_budget(Duration::from_secs(2)),
        &unica_workspace::discovery::ProjectDiscoveryReader::new(&fixture.context.workspace_root),
    )
    .unwrap();
    let git = GitRepositoryInspector::new()
        .inspect_base(
            &fixture.context,
            &layout,
            &cancellation,
            ProviderDeadline::from_budget(Duration::from_secs(5)),
        )
        .unwrap();
    let policy = SourceResourcePolicyInspector::new()
        .inspect(
            git.repository_root.as_ref().unwrap(),
            &layout.roots,
            &git.entries,
            &cancellation,
            ProviderDeadline::from_budget(Duration::from_secs(5)),
        )
        .unwrap();
    let mut observations = layout.observations;
    observations.extend(git.observations);
    observations.extend(policy.observations);
    let mut facts = layout.facts;
    facts.extend(git.facts);
    facts.extend(policy.facts);

    let report = crate::evaluate_project_health(crate::ProjectHealthSnapshot {
        workspace_root: fixture.context.workspace_root.display().to_string(),
        cache_root: fixture.context.cache_root.display().to_string(),
        repository_root: git.repository_root.map(|root| root.display().to_string()),
        source_sets: layout.source_sets,
        source_targets_complete: layout.source_targets_complete,
        observations,
        facts,
    })
    .unwrap();

    assert!(!report.repository_ready);
    assert!(report
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "git.text_policy_missing"));
}

#[test]
fn proven_runtime_sidecar_is_excluded_from_derivative_text_policy_checks() {
    let fixture = policy_fixture();
    fs::write(
        fixture.root.join("src/ConfigDumpInfo.xml"),
        "<ConfigDumpInfo><ConfigVersions/></ConfigDumpInfo>\n",
    )
    .unwrap();
    git(&fixture.root, &["add", "src/ConfigDumpInfo.xml"]);
    let cancellation = CancellationToken::new();
    let layout = SourceLayoutInspector::inspect(
        &fixture.context,
        &cancellation,
        ProviderDeadline::from_budget(Duration::from_secs(2)),
        &unica_workspace::discovery::ProjectDiscoveryReader::new(&fixture.context.workspace_root),
    )
    .unwrap();
    let git = GitRepositoryInspector::new()
        .inspect_base(
            &fixture.context,
            &layout,
            &cancellation,
            ProviderDeadline::from_budget(Duration::from_secs(5)),
        )
        .unwrap();
    let runtime_sidecars = git
        .facts
        .iter()
        .filter_map(|fact| match fact {
            ProjectHealthFact::RuntimeSidecarTracked { path, .. } => Some(path.clone()),
            _ => None,
        })
        .collect();

    let inspection = SourceResourcePolicyInspector::new()
        .inspect_excluding(
            git.repository_root.as_ref().unwrap(),
            &layout.roots,
            &git.entries,
            &runtime_sidecars,
            &cancellation,
            ProviderDeadline::from_budget(Duration::from_secs(5)),
        )
        .unwrap();

    assert!(!inspection.facts.iter().any(|fact| matches!(
        fact,
        ProjectHealthFact::TextPolicyMissing { path, .. }
            if path == "src/ConfigDumpInfo.xml"
    )));
}

struct PolicyFixture {
    _temp: TempDir,
    /// Daemon state outside the workspace, where continuations live.
    state: TempDir,
    root: PathBuf,
    context: WorkspaceContext,
}

impl PolicyFixture {
    fn continuation_file(&self) -> PathBuf {
        self.state.path().join("continuation.bin")
    }
}

fn policy_fixture() -> PolicyFixture {
    let temp = TempDir::new().unwrap();
    let root = temp.path().to_path_buf();
    git(&root, &["init"]);
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("src/Configuration.xml"), "<MetaDataObject/>\n").unwrap();
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
    PolicyFixture {
        _temp: temp,
        state: TempDir::new().unwrap(),
        root,
        context,
    }
}

fn inspect_policy(fixture: &PolicyFixture) -> super::RepositoryPolicyInspection {
    let cancellation = CancellationToken::new();
    let layout = SourceLayoutInspector::inspect(
        &fixture.context,
        &cancellation,
        ProviderDeadline::from_budget(Duration::from_secs(2)),
        &unica_workspace::discovery::ProjectDiscoveryReader::new(&fixture.context.workspace_root),
    )
    .unwrap();
    let git = GitRepositoryInspector::new()
        .inspect_base(
            &fixture.context,
            &layout,
            &cancellation,
            ProviderDeadline::from_budget(Duration::from_secs(5)),
        )
        .unwrap();
    let repository_root = git.repository_root.as_ref().unwrap();
    let inspection = SourceResourcePolicyInspector::new()
        .inspect(
            repository_root,
            &layout.roots,
            &git.entries,
            &cancellation,
            ProviderDeadline::from_budget(Duration::from_secs(5)),
        )
        .unwrap();
    assert!(inspection
        .observations
        .iter()
        .any(|observation| { observation.id == ProjectCheckId::RepositoryAttributes }));
    inspection
}

fn git(cwd: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

struct FailingRunner;

impl ProcessRunner for FailingRunner {
    fn run(&self, _command: &ProcessCommand) -> Result<ProcessOutput, String> {
        Ok(ProcessOutput {
            status_success: false,
            status: "exit status: 2".into(),
            stdout: String::new(),
            stderr: "forced failure".into(),
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

#[derive(Default)]
struct AttributeProbeRunner {
    commands: RefCell<Vec<ProcessCommand>>,
}

struct LateCancellingAttributeRunner {
    inner: AttributeProbeRunner,
    cancellation: CancellationToken,
}

struct LateCancellingEolIndexRunner {
    inner: AttributeProbeRunner,
    cancellation: CancellationToken,
    read_tree_calls: Cell<usize>,
}

struct ResourceLoopCheckpointRunner {
    cancellation: Option<CancellationToken>,
    attribute_calls: Cell<usize>,
}

impl ProcessRunner for ResourceLoopCheckpointRunner {
    fn run(&self, command: &ProcessCommand) -> Result<ProcessOutput, String> {
        if command.args.iter().any(|arg| arg == "rev-parse") {
            if command.args.iter().any(|arg| arg == "--show-object-format") {
                return Ok(success_output("sha1\n"));
            }
            return Ok(success_output(&format!(
                "{}\n",
                command.cwd.join(".git/objects").display()
            )));
        }
        Ok(success_output(""))
    }

    fn run_with_input(
        &self,
        _command: &ProcessCommand,
        _input: &[u8],
    ) -> Result<ProcessOutput, String> {
        let calls = self.attribute_calls.get() + 1;
        self.attribute_calls.set(calls);
        if calls == 2 {
            if let Some(cancellation) = &self.cancellation {
                cancellation.cancel();
            } else {
                RESOURCE_POLICY_EXPIRED.with(|expired| expired.set(true));
            }
        }
        let (text, eol) = ("unset", "unspecified");
        Ok(success_output(&format!(
            "src/Picture.bin\0text\0{text}\0src/Picture.bin\0eol\0{eol}\0src/Picture.bin\0filter\0unspecified\0"
        )))
    }
}

impl ProcessRunner for AttributeProbeRunner {
    fn run(&self, command: &ProcessCommand) -> Result<ProcessOutput, String> {
        self.commands.borrow_mut().push(command.clone());
        let stdout = if command.args.iter().any(|arg| arg == "cat-file")
            && command.args.iter().any(|arg| arg == "-s")
        {
            "5\n".into()
        } else if command.args.iter().any(|arg| arg == "--show-object-format") {
            "sha1\n".into()
        } else if command.args.iter().any(|arg| arg == "rev-parse") {
            format!("{}\n", command.cwd.join(".git/objects").display())
        } else if command.args.first().map(String::as_str) == Some("hash-object") {
            format!("{}\n", "a".repeat(40))
        } else if command.args.iter().any(|arg| arg == "ls-files") {
            "i/lf w/lf attr/text\tsrc/A.xml\0".into()
        } else {
            String::new()
        };
        Ok(success_output(&stdout))
    }

    fn run_with_input(
        &self,
        command: &ProcessCommand,
        _input: &[u8],
    ) -> Result<ProcessOutput, String> {
        if command.args.first().map(String::as_str) == Some("hash-object") {
            self.commands.borrow_mut().push(command.clone());
            return Ok(success_output(&format!("{}\n", "a".repeat(40))));
        }
        if command.args.iter().any(|arg| arg == "cat-file") {
            self.commands.borrow_mut().push(command.clone());
            return Ok(success_output(&format!("{} blob 5\n", "a".repeat(40))));
        }
        let staged_probe = command.args.iter().any(|arg| arg == "--git-dir")
            || command.args.iter().any(|arg| arg.starts_with("--git-dir="));
        self.commands.borrow_mut().push(command.clone());
        let value = if staged_probe { "set" } else { "unspecified" };
        Ok(success_output(&format!(
            "src/A.xml\0text\0{value}\0src/A.xml\0eol\0{value}\0src/A.xml\0filter\0unspecified\0"
        )))
    }
}

impl ProcessRunner for LateCancellingAttributeRunner {
    fn run(&self, command: &ProcessCommand) -> Result<ProcessOutput, String> {
        self.inner.run(command)
    }

    fn run_with_input(
        &self,
        command: &ProcessCommand,
        input: &[u8],
    ) -> Result<ProcessOutput, String> {
        if command.args.iter().any(|arg| arg == "check-attr")
            && command.env.iter().any(|(name, _)| name == "GIT_INDEX_FILE")
        {
            self.cancellation.cancel();
            return Err("staged attribute process failed after cancellation".into());
        }
        self.inner.run_with_input(command, input)
    }
}

impl ProcessRunner for LateCancellingEolIndexRunner {
    fn run(&self, command: &ProcessCommand) -> Result<ProcessOutput, String> {
        if command.args.iter().any(|arg| arg == "read-tree") {
            let calls = self.read_tree_calls.get() + 1;
            self.read_tree_calls.set(calls);
            if calls == 2 {
                self.cancellation.cancel();
                return Err("EOL index initialization failed after cancellation".into());
            }
        }
        self.inner.run(command)
    }

    fn run_with_input(
        &self,
        command: &ProcessCommand,
        input: &[u8],
    ) -> Result<ProcessOutput, String> {
        self.inner.run_with_input(command, input)
    }
}

fn success_output(stdout: &str) -> ProcessOutput {
    ProcessOutput {
        status_success: true,
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

/// The working evidence of one path across calls, kept the way a
/// continuation keeps it.
#[derive(Default)]
struct WorkingCheckpoint {
    evidence: Option<super::WorkingEvidence>,
    progress: super::ResourceInspectionProgress,
}

impl WorkingCheckpoint {
    fn inspect(
        &mut self,
        root: &Path,
        path: &str,
        cancellation: &CancellationToken,
        deadline: ProviderDeadline,
    ) -> Result<Option<super::WorkingEol>, super::WorkingEolInspectionError> {
        let (result, evidence) = super::inspect_working_eol_continued(
            root,
            Path::new(path),
            self.evidence,
            &mut self.progress,
            cancellation,
            deadline,
        );
        self.evidence = evidence;
        result
    }
}

/// Stage `paths` with one shared blob and no working files, the way a
/// large repository looks to the index without writing every file.
fn stage_synthetic_paths(root: &Path, paths: impl Iterator<Item = String>) {
    use std::io::Write;
    use std::process::Stdio;

    let blob = Command::new("git")
        .args(["hash-object", "-w", "--stdin"])
        .current_dir(root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    blob.stdin.as_ref().unwrap().write_all(b"<A/>\n").unwrap();
    let blob = blob.wait_with_output().unwrap();
    assert!(blob.status.success(), "git hash-object failed");
    let oid = String::from_utf8(blob.stdout).unwrap().trim().to_owned();
    let mut input = Vec::new();
    for path in paths {
        input.extend_from_slice(format!("100644 {oid}\t{path}\0").as_bytes());
    }
    let mut update = Command::new("git")
        .args(["update-index", "-z", "--index-info"])
        .current_dir(root)
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    update.stdin.take().unwrap().write_all(&input).unwrap();
    assert!(update.wait().unwrap().success(), "git update-index failed");
}
