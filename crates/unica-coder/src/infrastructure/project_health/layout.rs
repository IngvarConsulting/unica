use crate::domain::cancellation::CancellationToken;
use crate::domain::code_intelligence::ProviderDeadline;
use crate::domain::project_health::{
    ProjectCheckId, ProjectCheckObservation, ProjectCheckOutcome, ProjectHealthFact,
    ProjectHealthInspectionError,
};
use crate::domain::project_sources::{ProjectSourceSet, SourceFormat};
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::project_sources::discover_project_source_map;
use crate::infrastructure::source_roots::{
    inspect_declared_source_root_route, normalize_path_identity, reject_linked_source_root_route,
    NamedSourceSetErrorKind,
};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

pub(crate) struct SourceLayoutInspector;

pub(crate) struct SourceLayoutInspection {
    pub(crate) source_sets: Option<Vec<ProjectSourceSet>>,
    pub(crate) source_targets_complete: bool,
    pub(crate) observations: Vec<ProjectCheckObservation>,
    pub(crate) facts: Vec<ProjectHealthFact>,
    pub(crate) roots: Vec<InspectedSourceRoot>,
}

pub(crate) struct InspectedSourceRoot {
    pub(crate) source_set: ProjectSourceSet,
    pub(crate) path: PathBuf,
}

impl SourceLayoutInspector {
    pub(crate) fn inspect(
        context: &WorkspaceContext,
        cancellation: &CancellationToken,
        deadline: ProviderDeadline,
    ) -> Result<SourceLayoutInspection, ProjectHealthInspectionError> {
        if cancellation.is_cancelled() {
            return Err(ProjectHealthInspectionError::Cancelled);
        }
        if deadline.remaining().is_zero() {
            return Err(ProjectHealthInspectionError::Fatal(
                "project health deadline expired before source discovery".into(),
            ));
        }
        let workspace_identity = normalize_path_identity(&context.workspace_root)
            .map_err(ProjectHealthInspectionError::Fatal)?;
        let source_map = match discover_project_source_map(&context.workspace_root) {
            Ok(source_map) => source_map,
            Err(reason) => {
                return Ok(SourceLayoutInspection {
                    source_sets: None,
                    source_targets_complete: false,
                    observations: vec![completed(ProjectCheckId::SourceDiscovery, None)],
                    facts: vec![ProjectHealthFact::SourceInspectionIncomplete { reason }],
                    roots: Vec::new(),
                });
            }
        };
        let mut observations = vec![completed(ProjectCheckId::SourceDiscovery, None)];
        let mut facts = Vec::new();
        let mut roots = Vec::new();
        let mut source_targets_complete = true;
        if source_map.source_sets.is_empty() {
            facts.push(ProjectHealthFact::NoSourceSets);
            source_targets_complete = false;
        }
        let mut sets_by_name = BTreeMap::<&str, Vec<&ProjectSourceSet>>::new();
        for source_set in &source_map.source_sets {
            sets_by_name
                .entry(source_set.name.as_str())
                .or_default()
                .push(source_set);
        }
        for (name, source_sets) in &sets_by_name {
            if source_sets.len() > 1 {
                facts.push(ProjectHealthFact::SourceNameAmbiguous {
                    name: (*name).to_owned(),
                    count: source_sets.len(),
                });
                source_targets_complete = false;
            }
        }
        let cache_identity = normalize_path_identity(&context.cache_root)
            .map_err(ProjectHealthInspectionError::Fatal)?;
        for source_set in source_map.source_sets.iter().filter(|source_set| {
            sets_by_name
                .get(source_set.name.as_str())
                .is_some_and(|matches| matches.len() == 1)
        }) {
            observations.push(completed(
                ProjectCheckId::SourceLayout,
                Some(&source_set.name),
            ));
            observations.push(completed(
                ProjectCheckId::SourceFormat,
                Some(&source_set.name),
            ));
            observations.push(completed(
                ProjectCheckId::SourceGeneratedPaths,
                Some(&source_set.name),
            ));
            let route =
                match inspect_declared_source_root_route(&context.workspace_root, &source_set.path)
                {
                    Ok(route) => route,
                    Err(error) => {
                        facts.push(ProjectHealthFact::SourcePathUnsafe {
                            source_set: source_set.name.clone(),
                            path: source_set.path.clone(),
                            reason: error.to_string(),
                        });
                        source_targets_complete = false;
                        append_format_fact(&mut facts, source_set);
                        continue;
                    }
                };
            if route.identity_path == workspace_identity {
                let mut evidence = vec![
                    format!("normalized identity: {}", workspace_identity.display()),
                    format!(
                        "derived service path: {}",
                        context.workspace_root.join(".build").display()
                    ),
                ];
                if reject_linked_source_root_route(&context.workspace_root, &route.lexical_path)
                    .is_err()
                {
                    evidence.push(format!("linked route: {}", route.lexical_path.display()));
                }
                facts.push(ProjectHealthFact::SourceRootIsWorkspace {
                    source_set: source_set.name.clone(),
                    path: source_set.path.clone(),
                    evidence,
                });
                roots.push(InspectedSourceRoot {
                    source_set: source_set.clone(),
                    path: route.identity_path,
                });
            } else {
                match reject_linked_source_root_route(&context.workspace_root, &route.lexical_path)
                {
                    Ok(()) => {
                        if fs::symlink_metadata(&route.lexical_path).is_err() {
                            facts.push(ProjectHealthFact::SourcePathMissing {
                                source_set: source_set.name.clone(),
                                path: source_set.path.clone(),
                            });
                        } else {
                            if fs::symlink_metadata(route.identity_path.join(".build")).is_ok() {
                                facts.push(ProjectHealthFact::GeneratedBuildPresent {
                                    source_set: source_set.name.clone(),
                                    path: route.identity_path.join(".build").display().to_string(),
                                });
                            }
                            if crate::infrastructure::platform::filesystem::path_starts_with_host_root(
                                &cache_identity,
                                &route.identity_path,
                            ) {
                                facts.push(ProjectHealthFact::CacheInsideSourceSet {
                                    source_set: source_set.name.clone(),
                                    source_root: route.identity_path.display().to_string(),
                                    cache_root: cache_identity.display().to_string(),
                                });
                            }
                        }
                        roots.push(InspectedSourceRoot {
                            source_set: source_set.clone(),
                            path: route.identity_path,
                        });
                    }
                    Err(error) if error.kind == NamedSourceSetErrorKind::NotFound => {
                        facts.push(ProjectHealthFact::SourcePathMissing {
                            source_set: source_set.name.clone(),
                            path: source_set.path.clone(),
                        });
                        roots.push(InspectedSourceRoot {
                            source_set: source_set.clone(),
                            path: route.identity_path,
                        });
                    }
                    Err(error) => {
                        facts.push(ProjectHealthFact::SourcePathUnsafe {
                            source_set: source_set.name.clone(),
                            path: source_set.path.clone(),
                            reason: error.to_string(),
                        });
                        source_targets_complete = false;
                    }
                }
            }
            append_format_fact(&mut facts, source_set);
        }
        Ok(SourceLayoutInspection {
            source_sets: Some(source_map.source_sets),
            source_targets_complete,
            observations,
            facts,
            roots,
        })
    }
}

fn append_format_fact(facts: &mut Vec<ProjectHealthFact>, source_set: &ProjectSourceSet) {
    match source_set.source_format {
        SourceFormat::Invalid => facts.push(ProjectHealthFact::SourceFormatInvalid {
            source_set: source_set.name.clone(),
            evidence: source_set.format_evidence.clone(),
        }),
        SourceFormat::Unknown => facts.push(ProjectHealthFact::SourceFormatUnknown {
            source_set: source_set.name.clone(),
            evidence: source_set.format_evidence.clone(),
        }),
        SourceFormat::PlatformXml | SourceFormat::Edt => {}
    }
}

fn completed(id: ProjectCheckId, source_set: Option<&str>) -> ProjectCheckObservation {
    ProjectCheckObservation {
        id,
        scope: id.scope(),
        source_set: source_set.map(str::to_owned),
        outcome: ProjectCheckOutcome::Completed,
    }
}

#[cfg(test)]
mod tests {
    use super::SourceLayoutInspector;
    use crate::domain::cancellation::CancellationToken;
    use crate::domain::code_intelligence::ProviderDeadline;
    use crate::domain::project_health::ProjectHealthFact;
    use crate::domain::workspace::WorkspaceContext;
    use crate::infrastructure::platform::testing::{
        create_directory_link_fixture_for_test, FileLinkFixtureOutcome,
    };
    use std::fs;
    use std::time::Duration;
    use tempfile::TempDir;

    #[test]
    fn root_identity_equal_to_workspace_is_one_primary_fact() {
        for configured in [".", "./", "src/.."] {
            let fixture = layout_fixture(configured);
            let inspection = SourceLayoutInspector::inspect(
                &fixture.context,
                &CancellationToken::new(),
                ProviderDeadline::from_budget(Duration::from_secs(1)),
            )
            .unwrap();

            assert_eq!(inspection.facts.len(), 1, "configured path: {configured}");
            let ProjectHealthFact::SourceRootIsWorkspace { evidence, .. } = &inspection.facts[0]
            else {
                panic!("root-is-workspace fact expected for {configured}");
            };
            assert!(
                evidence.iter().any(|value| value.contains(".build")),
                "{evidence:?}"
            );
        }
    }

    #[test]
    fn malformed_project_config_is_a_typed_incomplete_snapshot() {
        let fixture = raw_layout_fixture("source-set: [\n");

        let inspection = inspect(&fixture);

        assert!(inspection.source_sets.is_none());
        assert!(!inspection.source_targets_complete);
        assert!(inspection.roots.is_empty());
        assert!(matches!(
            inspection.facts.as_slice(),
            [ProjectHealthFact::SourceInspectionIncomplete { reason }]
                if reason.contains("failed to parse")
        ));
    }

    #[test]
    fn complete_empty_discovery_reports_no_source_sets() {
        let fixture = raw_layout_fixture("format: DESIGNER\nsource-set: []\n");

        let inspection = inspect(&fixture);

        assert_eq!(inspection.source_sets.as_ref().map(Vec::len), Some(0));
        assert!(!inspection.source_targets_complete);
        assert!(matches!(
            inspection.facts.as_slice(),
            [ProjectHealthFact::NoSourceSets]
        ));
    }

    #[test]
    fn duplicate_names_have_one_group_fact_and_no_ambiguous_root() {
        let fixture = raw_layout_fixture(
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src/a\n  - name: main\n    type: EXTENSION\n    path: src/b\n",
        );
        fs::create_dir_all(fixture.context.workspace_root.join("src/a")).unwrap();
        fs::create_dir_all(fixture.context.workspace_root.join("src/b")).unwrap();

        let inspection = inspect(&fixture);

        assert!(!inspection.source_targets_complete);
        assert!(inspection.roots.is_empty());
        assert!(matches!(
            inspection.facts.as_slice(),
            [ProjectHealthFact::SourceNameAmbiguous { name, count }]
                if name == "main" && *count == 2
        ));
    }

    #[test]
    fn missing_source_root_is_reported_without_becoming_fatal() {
        let fixture = layout_fixture("src/missing");

        let inspection = inspect(&fixture);

        assert!(inspection.source_targets_complete);
        assert_eq!(inspection.roots.len(), 1);
        assert!(matches!(
            inspection.facts.as_slice(),
            [ProjectHealthFact::SourcePathMissing { source_set, path }]
                if source_set == "main" && path == "src/missing"
        ));
    }

    #[test]
    fn outside_source_root_is_a_typed_unsafe_path() {
        let fixture = layout_fixture("../outside");

        let inspection = inspect(&fixture);

        assert!(!inspection.source_targets_complete);
        assert!(inspection.roots.is_empty());
        assert!(matches!(
            inspection.facts.as_slice(),
            [ProjectHealthFact::SourcePathUnsafe { source_set, path, .. }]
                if source_set == "main" && path == "../outside"
        ));
    }

    #[test]
    fn build_and_cache_are_independent_facts_for_a_nested_root() {
        let fixture = layout_fixture("src");
        fs::create_dir_all(fixture.context.workspace_root.join("src/.build/unica")).unwrap();
        let mut context = fixture.context.clone();
        context.cache_root = context.workspace_root.join("src/.build/unica");

        let inspection = SourceLayoutInspector::inspect(
            &context,
            &CancellationToken::new(),
            ProviderDeadline::from_budget(Duration::from_secs(1)),
        )
        .unwrap();

        assert_eq!(inspection.facts.len(), 2);
        assert!(inspection
            .facts
            .iter()
            .any(|fact| matches!(fact, ProjectHealthFact::CacheInsideSourceSet { .. })));
        assert!(inspection
            .facts
            .iter()
            .any(|fact| matches!(fact, ProjectHealthFact::GeneratedBuildPresent { .. })));
    }

    #[test]
    fn linked_alias_to_workspace_reports_the_primary_identity_cause_with_evidence() {
        let fixture = layout_fixture("alias");
        let outcome = create_directory_link_fixture_for_test(
            &fixture.context.workspace_root,
            fixture.context.workspace_root.join("alias"),
        )
        .unwrap();
        if outcome != FileLinkFixtureOutcome::Created {
            return;
        }

        let inspection = inspect(&fixture);

        assert!(matches!(
            inspection.facts.as_slice(),
            [ProjectHealthFact::SourceRootIsWorkspace { evidence, .. }]
                if evidence.iter().any(|item| item.contains("linked route: "))
        ));
    }

    #[test]
    fn linked_route_to_another_nested_root_is_unsafe() {
        let fixture = layout_fixture("alias");
        let nested = fixture.context.workspace_root.join("actual");
        fs::create_dir_all(&nested).unwrap();
        let outcome = create_directory_link_fixture_for_test(
            &nested,
            fixture.context.workspace_root.join("alias"),
        )
        .unwrap();
        if outcome != FileLinkFixtureOutcome::Created {
            return;
        }

        let inspection = inspect(&fixture);

        assert!(!inspection.source_targets_complete);
        assert!(inspection.roots.is_empty());
        assert!(matches!(
            inspection.facts.as_slice(),
            [ProjectHealthFact::SourcePathUnsafe { reason, .. }]
                if reason.contains("symbolic link or reparse point")
        ));
    }

    struct LayoutFixture {
        _temp: TempDir,
        context: WorkspaceContext,
    }

    fn layout_fixture(configured_path: &str) -> LayoutFixture {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        fs::create_dir_all(workspace.join("src")).unwrap();
        fs::create_dir_all(workspace.join(".build/unica")).unwrap();
        fs::write(
            workspace.join("v8project.yaml"),
            format!(
                "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: {configured_path}\n"
            ),
        )
        .unwrap();
        fs::write(workspace.join("Configuration.xml"), "<MetaDataObject/>").unwrap();
        LayoutFixture {
            _temp: temp,
            context: WorkspaceContext {
                cwd: workspace.clone(),
                workspace_root: workspace.clone(),
                cache_root: workspace.join(".build/unica"),
                workspace_epoch: 0,
            },
        }
    }

    fn raw_layout_fixture(config: &str) -> LayoutFixture {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        fs::create_dir_all(&workspace).unwrap();
        fs::write(workspace.join("v8project.yaml"), config).unwrap();
        LayoutFixture {
            _temp: temp,
            context: WorkspaceContext {
                cwd: workspace.clone(),
                workspace_root: workspace.clone(),
                cache_root: workspace.join(".build/unica"),
                workspace_epoch: 0,
            },
        }
    }

    fn inspect(fixture: &LayoutFixture) -> super::SourceLayoutInspection {
        SourceLayoutInspector::inspect(
            &fixture.context,
            &CancellationToken::new(),
            ProviderDeadline::from_budget(Duration::from_secs(1)),
        )
        .unwrap()
    }
}
