use crate::domain::cancellation::CancellationToken;
use crate::domain::code_intelligence::ProviderDeadline;
use crate::domain::project_health::{
    ProjectCheckId, ProjectCheckObservation, ProjectCheckOutcome, ProjectHealthFact,
    ProjectHealthInspectionError,
};
use crate::domain::project_sources::{ProjectSourceSet, SourceFormat};
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::platform::filesystem::{
    open_absolute_directory_path_nofollow, open_any_child_nofollow,
};
use crate::infrastructure::project_sources::discover_project_source_map_controlled;
use crate::infrastructure::source_roots::{
    inspect_declared_source_root_route, normalize_path_identity, reject_linked_source_root_route,
    NamedSourceSetErrorKind,
};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

pub(crate) struct SourceLayoutInspector;

const SOURCE_DISCOVERY_CANCELLED: &str = "project health source discovery cancelled";
const SOURCE_DISCOVERY_DEADLINE: &str = "project health source discovery deadline expired";

pub(crate) struct SourceLayoutInspection {
    pub(crate) source_sets: Option<Vec<ProjectSourceSet>>,
    pub(crate) source_targets_complete: bool,
    pub(crate) repository_targets_complete: bool,
    pub(crate) observations: Vec<ProjectCheckObservation>,
    pub(crate) facts: Vec<ProjectHealthFact>,
    pub(crate) roots: Vec<InspectedSourceRoot>,
}

#[derive(Clone)]
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
        if let Some(incomplete) =
            incomplete_if_inspection_inactive(cancellation, deadline, "before source discovery")?
        {
            return Ok(incomplete);
        }
        let workspace_identity = normalize_path_identity(&context.workspace_root)
            .map_err(ProjectHealthInspectionError::Fatal)?;
        let mut checkpoint = || {
            if cancellation.is_cancelled() {
                Err(SOURCE_DISCOVERY_CANCELLED.to_string())
            } else if deadline.remaining().is_zero() {
                Err(SOURCE_DISCOVERY_DEADLINE.to_string())
            } else {
                Ok(())
            }
        };
        let source_map = match discover_project_source_map_controlled(
            &context.workspace_root,
            &mut checkpoint,
        ) {
            Ok(source_map) => source_map,
            Err(reason) if reason == SOURCE_DISCOVERY_CANCELLED => {
                return Err(ProjectHealthInspectionError::Cancelled);
            }
            Err(reason) if reason == SOURCE_DISCOVERY_DEADLINE => {
                let reason = "project health deadline expired during source discovery".to_owned();
                return Ok(incomplete_source_layout(reason));
            }
            Err(reason) => {
                return Ok(incomplete_source_layout(reason));
            }
        };
        if let Some(incomplete) =
            incomplete_if_inspection_inactive(cancellation, deadline, "during source discovery")?
        {
            return Ok(incomplete);
        }
        let mut observations = vec![completed(ProjectCheckId::SourceDiscovery, None)];
        let mut facts = Vec::new();
        let mut roots = Vec::new();
        let mut source_targets_complete = true;
        let mut repository_targets_complete = true;
        if source_map.source_sets.is_empty() {
            facts.push(ProjectHealthFact::NoSourceSets);
            source_targets_complete = false;
            repository_targets_complete = false;
        }
        let mut sets_by_name = BTreeMap::<&str, Vec<&ProjectSourceSet>>::new();
        for source_set in &source_map.source_sets {
            if let Some(incomplete) = incomplete_if_inspection_inactive(
                cancellation,
                deadline,
                "while grouping source sets",
            )? {
                return Ok(incomplete);
            }
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
                repository_targets_complete = false;
            }
        }
        let cache_identity = normalize_path_identity(&context.cache_root)
            .map_err(ProjectHealthInspectionError::Fatal)?;
        for source_set in source_map.source_sets.iter().filter(|source_set| {
            sets_by_name
                .get(source_set.name.as_str())
                .is_some_and(|matches| matches.len() == 1)
        }) {
            if let Some(incomplete) = incomplete_if_inspection_inactive(
                cancellation,
                deadline,
                "while inspecting source sets",
            )? {
                return Ok(incomplete);
            }
            if matches!(
                source_set.source_format,
                SourceFormat::Unknown | SourceFormat::Invalid
            ) {
                source_targets_complete = false;
            }
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
            if let Some(reason) = source_set.format_probe_error.as_deref() {
                source_targets_complete = false;
                mark_source_check_not_run(
                    &mut observations,
                    ProjectCheckId::SourceFormat,
                    &source_set.name,
                    reason,
                );
            }
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
                        repository_targets_complete = false;
                        mark_source_check_not_run(
                            &mut observations,
                            ProjectCheckId::SourceGeneratedPaths,
                            &source_set.name,
                            "source route could not be proven safe",
                        );
                        if source_set.format_evidence.is_empty() {
                            mark_source_check_not_run(
                                &mut observations,
                                ProjectCheckId::SourceFormat,
                                &source_set.name,
                                "source route could not be proven safe before format probing",
                            );
                        } else {
                            append_format_fact(&mut facts, source_set);
                        }
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
                let linked_route =
                    reject_linked_source_root_route(&context.workspace_root, &route.lexical_path)
                        .is_err();
                if linked_route {
                    evidence.push(format!("linked route: {}", route.lexical_path.display()));
                    mark_source_check_not_run(
                        &mut observations,
                        ProjectCheckId::SourceGeneratedPaths,
                        &source_set.name,
                        "linked source route was not inspected for generated paths",
                    );
                    if source_set.format_evidence.is_empty() {
                        mark_source_check_not_run(
                            &mut observations,
                            ProjectCheckId::SourceFormat,
                            &source_set.name,
                            "linked source route was not probed for format evidence",
                        );
                    }
                }
                facts.push(ProjectHealthFact::SourceRootIsWorkspace {
                    source_set: source_set.name.clone(),
                    path: source_set.path.clone(),
                    evidence,
                });
                source_targets_complete = false;
                mark_source_check_not_run(
                    &mut observations,
                    ProjectCheckId::SourceGeneratedPaths,
                    &source_set.name,
                    "workspace root was rejected before generated paths were inspected",
                );
            } else {
                match reject_linked_source_root_route(&context.workspace_root, &route.lexical_path)
                {
                    Ok(()) => {
                        match fs::symlink_metadata(&route.lexical_path) {
                            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                                facts.push(ProjectHealthFact::SourcePathMissing {
                                    source_set: source_set.name.clone(),
                                    path: source_set.path.clone(),
                                });
                                mark_source_check_not_run(
                                    &mut observations,
                                    ProjectCheckId::SourceGeneratedPaths,
                                    &source_set.name,
                                    "missing source root has no generated paths to inspect",
                                );
                            }
                            Err(error) => {
                                facts.push(ProjectHealthFact::SourcePathUnsafe {
                                    source_set: source_set.name.clone(),
                                    path: source_set.path.clone(),
                                    reason: format!("source root metadata failed: {error}"),
                                });
                                source_targets_complete = false;
                                repository_targets_complete = false;
                                mark_source_check_not_run(
                                    &mut observations,
                                    ProjectCheckId::SourceGeneratedPaths,
                                    &source_set.name,
                                    "source root metadata could not be inspected",
                                );
                                continue;
                            }
                            Ok(metadata) if !metadata.is_dir() => {
                                facts.push(ProjectHealthFact::SourcePathUnsafe {
                                    source_set: source_set.name.clone(),
                                    path: source_set.path.clone(),
                                    reason: "source root is not a directory".into(),
                                });
                                source_targets_complete = false;
                                repository_targets_complete = false;
                                mark_source_check_not_run(
                                    &mut observations,
                                    ProjectCheckId::SourceGeneratedPaths,
                                    &source_set.name,
                                    "source root is not a directory",
                                );
                                continue;
                            }
                            Ok(_) => {
                                let source_directory = match open_absolute_directory_path_nofollow(
                                    &route.identity_path,
                                ) {
                                    Ok(directory) => directory,
                                    Err(error) => {
                                        facts.push(ProjectHealthFact::SourcePathUnsafe {
                                            source_set: source_set.name.clone(),
                                            path: source_set.path.clone(),
                                            reason: format!(
                                                "source root could not be opened safely: {error}"
                                            ),
                                        });
                                        source_targets_complete = false;
                                        repository_targets_complete = false;
                                        mark_source_check_not_run(
                                            &mut observations,
                                            ProjectCheckId::SourceGeneratedPaths,
                                            &source_set.name,
                                            "source root could not be opened safely",
                                        );
                                        continue;
                                    }
                                };
                                match open_any_child_nofollow(
                                    &source_directory,
                                    std::ffi::OsStr::new(".build"),
                                ) {
                                    Ok(_) => {
                                        facts.push(ProjectHealthFact::GeneratedBuildPresent {
                                            source_set: source_set.name.clone(),
                                            path: route
                                                .identity_path
                                                .join(".build")
                                                .display()
                                                .to_string(),
                                        });
                                    }
                                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                                    Err(error) => {
                                        facts.push(ProjectHealthFact::SourcePathUnsafe {
                                            source_set: source_set.name.clone(),
                                            path: source_set.path.clone(),
                                            reason: format!(
                                                "generated path lookup failed: {error}"
                                            ),
                                        });
                                        source_targets_complete = false;
                                        mark_source_check_not_run(
                                            &mut observations,
                                            ProjectCheckId::SourceGeneratedPaths,
                                            &source_set.name,
                                            "generated path lookup was incomplete",
                                        );
                                    }
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
                        mark_source_check_not_run(
                            &mut observations,
                            ProjectCheckId::SourceGeneratedPaths,
                            &source_set.name,
                            "missing source root has no generated paths to inspect",
                        );
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
                        repository_targets_complete = false;
                        mark_source_check_not_run(
                            &mut observations,
                            ProjectCheckId::SourceGeneratedPaths,
                            &source_set.name,
                            "linked or unsafe source route was not inspected for generated paths",
                        );
                        if source_set.format_evidence.is_empty() {
                            mark_source_check_not_run(
                                &mut observations,
                                ProjectCheckId::SourceFormat,
                                &source_set.name,
                                "linked or unsafe source route was not probed for format evidence",
                            );
                        }
                    }
                }
            }
            if observations.iter().any(|observation| {
                observation.id == ProjectCheckId::SourceFormat
                    && observation.source_set.as_deref() == Some(source_set.name.as_str())
                    && matches!(observation.outcome, ProjectCheckOutcome::Completed)
            }) {
                append_format_fact(&mut facts, source_set);
            }
        }
        Ok(SourceLayoutInspection {
            source_sets: Some(source_map.source_sets),
            source_targets_complete,
            repository_targets_complete,
            observations,
            facts,
            roots,
        })
    }
}

fn incomplete_if_inspection_inactive(
    cancellation: &CancellationToken,
    deadline: ProviderDeadline,
    phase: &str,
) -> Result<Option<SourceLayoutInspection>, ProjectHealthInspectionError> {
    if cancellation.is_cancelled() {
        return Err(ProjectHealthInspectionError::Cancelled);
    }
    if deadline.remaining().is_zero() {
        return Ok(Some(incomplete_source_layout(format!(
            "project health deadline expired {phase}"
        ))));
    }
    Ok(None)
}

fn incomplete_source_layout(reason: String) -> SourceLayoutInspection {
    SourceLayoutInspection {
        source_sets: None,
        source_targets_complete: false,
        repository_targets_complete: false,
        observations: vec![ProjectCheckObservation {
            id: ProjectCheckId::SourceDiscovery,
            scope: ProjectCheckId::SourceDiscovery.scope(),
            source_set: None,
            outcome: ProjectCheckOutcome::NotRun {
                reason: reason.clone(),
            },
        }],
        facts: vec![ProjectHealthFact::SourceInspectionIncomplete { reason }],
        roots: Vec::new(),
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

fn mark_source_check_not_run(
    observations: &mut [ProjectCheckObservation],
    id: ProjectCheckId,
    source_set: &str,
    reason: &str,
) {
    for observation in observations.iter_mut().filter(|observation| {
        observation.id == id && observation.source_set.as_deref() == Some(source_set)
    }) {
        observation.outcome = ProjectCheckOutcome::NotRun {
            reason: reason.to_owned(),
        };
    }
}

#[cfg(test)]
mod tests {
    use super::SourceLayoutInspector;
    use crate::domain::cancellation::CancellationToken;
    use crate::domain::code_intelligence::ProviderDeadline;
    use crate::domain::project_health::{ProjectCheckId, ProjectCheckOutcome, ProjectHealthFact};
    use crate::domain::project_sources::SourceFormat;
    use crate::domain::workspace::WorkspaceContext;
    use crate::infrastructure::platform::testing::{
        create_directory_link_fixture_for_test, FileLinkFixtureOutcome,
    };
    use std::fs;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::OnceLock;
    use std::time::{Duration, Instant};
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
            assert!(!inspection.source_targets_complete);
            assert!(inspection.repository_targets_complete);
            assert!(inspection.roots.is_empty());
            assert!(inspection.observations.iter().any(|observation| {
                observation.id == ProjectCheckId::SourceGeneratedPaths
                    && matches!(observation.outcome, ProjectCheckOutcome::NotRun { .. })
            }));
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
    fn oversized_project_config_is_rejected_before_unbounded_read() {
        let fixture = raw_layout_fixture("");
        let config = fixture.context.workspace_root.join("v8project.yaml");
        fs::File::create(&config)
            .unwrap()
            .set_len(8 * 1024 * 1024 + 1)
            .unwrap();

        let inspection = inspect(&fixture);

        assert!(
            matches!(
                inspection.facts.as_slice(),
                [ProjectHealthFact::SourceInspectionIncomplete { reason }]
                    if reason.contains("exceeds") && reason.contains("8388608")
            ),
            "{:?}",
            inspection.facts
        );
    }

    #[test]
    fn large_platform_marker_does_not_consume_health_source_map_bytes() {
        let fixture = raw_layout_fixture(
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        );
        fs::create_dir_all(fixture.context.workspace_root.join("src")).unwrap();
        fs::File::create(fixture.context.workspace_root.join("src/Configuration.xml"))
            .unwrap()
            .set_len(9 * 1024 * 1024)
            .unwrap();

        let inspection = inspect(&fixture);

        assert!(inspection.source_targets_complete, "{:?}", inspection.facts);
        assert!(inspection.facts.is_empty(), "{:?}", inspection.facts);
        assert_eq!(
            inspection.source_sets.as_ref().unwrap()[0].source_format,
            SourceFormat::PlatformXml
        );
    }

    #[test]
    fn excessive_source_set_count_is_a_typed_incomplete_discovery() {
        let mut config = "format: DESIGNER\nsource-set:\n".to_string();
        for index in 0..=1024 {
            config.push_str(&format!(
                "  - name: set{index}\n    type: CONFIGURATION\n    path: src/{index}\n"
            ));
        }
        let fixture = raw_layout_fixture(&config);

        let inspection = inspect(&fixture);

        assert!(!inspection.source_targets_complete);
        assert!(
            matches!(
                inspection.facts.as_slice(),
                [ProjectHealthFact::SourceInspectionIncomplete { reason }]
                    if reason.contains("source sets") && reason.contains("1024")
            ),
            "{:?}",
            inspection.facts
        );
    }

    #[test]
    fn health_source_set_limit_aborts_before_full_yaml_ast_materialization() {
        let mut config = "format: DESIGNER\nsource-set:\n".to_string();
        for _ in 0..400_000 {
            config.push_str("  - {}\n");
        }
        let fixture = raw_layout_fixture(&config);

        let inspection = inspect(&fixture);

        assert!(!inspection.source_targets_complete);
        assert!(matches!(
            inspection.facts.as_slice(),
            [ProjectHealthFact::SourceInspectionIncomplete { reason }]
                if (reason.contains("source sets") && reason.contains("1024"))
                    || (reason.contains("expanded YAML") && reason.contains("health inspection"))
        ));
    }

    #[test]
    fn health_yaml_parser_preserves_numeric_base_path_semantics() {
        let fixture = raw_layout_fixture(
            "format: DESIGNER\nbasePath: 123\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        );
        fs::create_dir_all(fixture.context.workspace_root.join("123/src")).unwrap();

        let inspection = inspect(&fixture);

        assert!(inspection.source_sets.is_some(), "{:?}", inspection.facts);
        assert_eq!(inspection.source_sets.as_ref().unwrap()[0].path, "123/src");
    }

    #[test]
    fn health_yaml_parser_rejects_numeric_format_like_ordinary_parser() {
        let fixture = raw_layout_fixture("format: 123\nsource-set: []\n");

        let inspection = inspect(&fixture);

        assert!(matches!(
            inspection.facts.as_slice(),
            [ProjectHealthFact::SourceInspectionIncomplete { reason }]
                if reason.contains("field `format` must be a string")
        ));
    }

    #[test]
    fn health_yaml_parser_rejects_null_format_like_ordinary_parser() {
        let fixture = raw_layout_fixture("format: null\nsource-set: []\n");

        let inspection = inspect(&fixture);

        assert!(matches!(
            inspection.facts.as_slice(),
            [ProjectHealthFact::SourceInspectionIncomplete { reason }]
                if reason.contains("field `format` must be a string")
        ));
    }

    #[test]
    fn health_yaml_parser_rejects_duplicate_top_level_contract_fields() {
        for duplicate in [
            "format: DESIGNER\nformat: EDT\n",
            "basePath: src\nbasePath: other\n",
            "source-set: []\nsource-set: []\n",
        ] {
            let fixture = raw_layout_fixture(duplicate);
            let inspection = inspect(&fixture);
            assert!(
                matches!(
                    inspection.facts.as_slice(),
                    [ProjectHealthFact::SourceInspectionIncomplete { reason }]
                        if reason.contains("duplicate")
                ),
                "config={duplicate:?}; facts={:?}",
                inspection.facts
            );
        }
    }

    #[test]
    fn health_yaml_parser_rejects_aliases_before_value_materialization() {
        let fixture = raw_layout_fixture(&format!(
            "defaults: &large {{name: {}, type: CONFIGURATION, path: src}}\nsource-set:\n{}",
            "x".repeat(4096),
            "  - *large\n".repeat(1024)
        ));

        let inspection = inspect(&fixture);

        assert!(matches!(
            inspection.facts.as_slice(),
            [ProjectHealthFact::SourceInspectionIncomplete { reason }]
                if reason.contains("expanded YAML") && reason.contains("health inspection")
        ));
    }

    #[test]
    fn health_yaml_parser_accepts_small_bounded_source_set_alias() {
        let fixture = raw_layout_fixture(
            "shared: &main {name: main, type: CONFIGURATION, path: src}\nformat: DESIGNER\nsource-set: [*main]\n",
        );
        fs::create_dir_all(fixture.context.workspace_root.join("src")).unwrap();

        let inspection = inspect(&fixture);

        assert!(inspection.source_sets.is_some(), "{:?}", inspection.facts);
        assert_eq!(inspection.source_sets.as_ref().unwrap()[0].name, "main");
        assert_eq!(inspection.source_sets.as_ref().unwrap()[0].path, "src");
    }

    #[test]
    fn health_yaml_parser_accepts_anchor_tokens_inside_unknown_block_scalar() {
        let fixture = raw_layout_fixture(
            "notes: |\n  * ordinary bullet\n  & ordinary ampersand\nformat: DESIGNER\nsource-set: []\n",
        );

        let inspection = inspect(&fixture);

        assert!(inspection.source_sets.is_some(), "{:?}", inspection.facts);
    }

    #[test]
    fn health_yaml_parser_bounds_container_nodes_before_value_materialization() {
        let fixture = raw_layout_fixture(&format!(
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n    ignored: [{}]\n",
            "[],".repeat(140_000)
        ));

        let inspection = inspect(&fixture);

        assert!(matches!(
            inspection.facts.as_slice(),
            [ProjectHealthFact::SourceInspectionIncomplete { reason }]
                if reason.contains("expanded YAML") && reason.contains("health inspection")
        ));
    }

    #[test]
    fn health_yaml_parser_bounds_value_selected_by_an_aliased_root_key() {
        let fixture = raw_layout_fixture(&format!(
            "key: &contract source-set\n? *contract\n:\n  - name: main\n    type: CONFIGURATION\n    path: src\n    ignored: [{}]\n",
            "[],".repeat(140_000)
        ));

        let inspection = inspect(&fixture);

        assert!(matches!(
            inspection.facts.as_slice(),
            [ProjectHealthFact::SourceInspectionIncomplete { reason }]
                if reason.contains("expanded YAML") && reason.contains("health inspection")
        ));
    }

    #[test]
    fn unknown_source_format_makes_source_targets_incomplete() {
        let fixture = raw_layout_fixture(
            "source-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        );
        fs::create_dir_all(fixture.context.workspace_root.join("src")).unwrap();

        let inspection = inspect(&fixture);

        assert!(!inspection.source_targets_complete);
        assert!(inspection
            .facts
            .iter()
            .any(|fact| matches!(fact, ProjectHealthFact::SourceFormatUnknown { .. })));
    }

    #[test]
    fn conflicting_source_format_makes_source_targets_incomplete() {
        let fixture = raw_layout_fixture(
            "source-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        );
        fs::create_dir_all(fixture.context.workspace_root.join("src/Configuration")).unwrap();
        fs::write(
            fixture.context.workspace_root.join("src/Configuration.xml"),
            "<MetaDataObject/>",
        )
        .unwrap();
        fs::write(
            fixture
                .context
                .workspace_root
                .join("src/Configuration/Configuration.mdo"),
            "<mdclass:Configuration/>",
        )
        .unwrap();

        let inspection = inspect(&fixture);

        assert!(!inspection.source_targets_complete);
        assert!(inspection
            .facts
            .iter()
            .any(|fact| matches!(fact, ProjectHealthFact::SourceFormatInvalid { .. })));
    }

    static DISCOVERY_DEADLINE_TICKS: AtomicUsize = AtomicUsize::new(0);
    static DISCOVERY_DEADLINE_ORIGIN: OnceLock<Instant> = OnceLock::new();

    fn advancing_discovery_clock() -> Instant {
        let tick = DISCOVERY_DEADLINE_TICKS.fetch_add(1, Ordering::SeqCst) as u64;
        *DISCOVERY_DEADLINE_ORIGIN.get_or_init(Instant::now) + Duration::from_millis(tick)
    }

    #[test]
    fn source_discovery_honors_deadline_at_internal_checkpoints() {
        let fixture = raw_layout_fixture(
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        );
        DISCOVERY_DEADLINE_TICKS.store(0, Ordering::SeqCst);
        let deadline = ProviderDeadline::with_clock(
            advancing_discovery_clock() + Duration::from_millis(5),
            advancing_discovery_clock,
        );

        let inspection =
            SourceLayoutInspector::inspect(&fixture.context, &CancellationToken::new(), deadline)
                .unwrap();

        assert!(!inspection.source_targets_complete);
        assert!(inspection.source_sets.is_none());
        assert!(matches!(
            inspection.facts.as_slice(),
            [ProjectHealthFact::SourceInspectionIncomplete { reason }]
                if reason == "project health deadline expired during source discovery"
        ));
    }

    static LAYOUT_DEADLINE_TICKS: AtomicUsize = AtomicUsize::new(0);
    static LAYOUT_DEADLINE_ORIGIN: OnceLock<Instant> = OnceLock::new();

    fn advancing_layout_clock() -> Instant {
        let tick = LAYOUT_DEADLINE_TICKS.fetch_add(1, Ordering::SeqCst) as u64;
        *LAYOUT_DEADLINE_ORIGIN.get_or_init(Instant::now) + Duration::from_millis(tick)
    }

    #[test]
    fn every_layout_deadline_discards_partial_results_into_a_typed_snapshot() {
        let fixture = raw_layout_fixture(
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        );
        fs::create_dir_all(fixture.context.workspace_root.join("src")).unwrap();

        let mut timed_out_phases = std::collections::BTreeSet::new();
        for budget_millis in 0..128 {
            LAYOUT_DEADLINE_TICKS.store(0, Ordering::SeqCst);
            let deadline = ProviderDeadline::with_clock(
                advancing_layout_clock() + Duration::from_millis(budget_millis),
                advancing_layout_clock,
            );
            let inspection = SourceLayoutInspector::inspect(
                &fixture.context,
                &CancellationToken::new(),
                deadline,
            )
            .unwrap_or_else(|error| {
                panic!("budget {budget_millis} ms must produce a typed snapshot: {error:?}")
            });

            if let Some(reason) = inspection.facts.iter().find_map(|fact| match fact {
                ProjectHealthFact::SourceInspectionIncomplete { reason }
                    if reason.contains("deadline") =>
                {
                    Some(reason.clone())
                }
                _ => None,
            }) {
                timed_out_phases.insert(reason);
                assert!(
                    inspection.source_sets.is_none(),
                    "budget {budget_millis} retained partial source sets"
                );
                assert_eq!(inspection.observations.len(), 1);
                assert!(matches!(
                    inspection.observations[0].outcome,
                    ProjectCheckOutcome::NotRun { .. }
                ));
            }
        }
        for phase in [
            "before source discovery",
            "during source discovery",
            "while grouping source sets",
            "while inspecting source sets",
        ] {
            assert!(
                timed_out_phases.iter().any(|reason| reason.contains(phase)),
                "deadline phase {phase:?} was not exercised: {timed_out_phases:?}"
            );
        }
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
        assert!(inspection.observations.iter().any(|observation| {
            observation.id == ProjectCheckId::SourceGeneratedPaths
                && matches!(observation.outcome, ProjectCheckOutcome::NotRun { .. })
        }));
    }

    #[test]
    fn unreadable_source_root_is_not_ready_for_source_operations() {
        use crate::infrastructure::platform::testing::set_unix_mode_for_test;

        let fixture = layout_fixture("src");
        let root = fixture.context.workspace_root.join("src");
        fs::create_dir_all(&root).unwrap();
        if !set_unix_mode_for_test(&root, 0o000).unwrap() {
            return;
        }

        let inspection = inspect(&fixture);

        set_unix_mode_for_test(&root, 0o700).unwrap();
        assert!(!inspection.source_targets_complete);
        assert!(inspection.roots.is_empty());
        assert!(inspection.facts.iter().any(|fact| matches!(
            fact,
            ProjectHealthFact::SourcePathUnsafe { reason, .. }
                if reason.contains("open") || reason.contains("denied")
        )));
        assert!(inspection.observations.iter().any(|observation| {
            observation.id == ProjectCheckId::SourceGeneratedPaths
                && matches!(observation.outcome, ProjectCheckOutcome::NotRun { .. })
        }));
    }

    #[test]
    fn regular_file_cannot_be_a_source_root() {
        let fixture = layout_fixture("src-file");
        fs::write(
            fixture.context.workspace_root.join("src-file"),
            "not a directory",
        )
        .unwrap();

        let inspection = inspect(&fixture);

        assert!(!inspection.source_targets_complete);
        assert!(inspection.roots.is_empty());
        assert!(inspection.facts.iter().any(|fact| matches!(
            fact,
            ProjectHealthFact::SourcePathUnsafe { reason, .. }
                if reason.contains("not a directory")
        )));
        assert!(inspection.observations.iter().any(|observation| {
            observation.id == ProjectCheckId::SourceGeneratedPaths
                && matches!(observation.outcome, ProjectCheckOutcome::NotRun { .. })
        }));
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
        assert!(inspection.observations.iter().any(|observation| {
            observation.id == ProjectCheckId::SourceFormat
                && matches!(observation.outcome, ProjectCheckOutcome::NotRun { .. })
        }));
        assert!(inspection.observations.iter().any(|observation| {
            observation.id == ProjectCheckId::SourceGeneratedPaths
                && matches!(observation.outcome, ProjectCheckOutcome::NotRun { .. })
        }));
    }

    #[test]
    fn linked_route_without_trusted_format_does_not_pass_format_or_generated_checks() {
        let fixture = raw_layout_fixture(
            "source-set:\n  - name: external\n    type: EXTERNAL_DATA_PROCESSORS\n    path: epf\n",
        );
        let external = fixture.context.workspace_root.join("external");
        fs::create_dir_all(&external).unwrap();
        fs::write(external.join("SecretPayroll.xml"), "<MetaDataObject/>").unwrap();
        let outcome = create_directory_link_fixture_for_test(
            &external,
            fixture.context.workspace_root.join("epf"),
        )
        .unwrap();
        if outcome != FileLinkFixtureOutcome::Created {
            return;
        }

        let inspection = inspect(&fixture);

        for check in [
            ProjectCheckId::SourceFormat,
            ProjectCheckId::SourceGeneratedPaths,
        ] {
            assert!(inspection.observations.iter().any(|observation| {
                observation.id == check
                    && matches!(observation.outcome, ProjectCheckOutcome::NotRun { .. })
            }));
        }
        assert!(!inspection.source_sets.unwrap()[0]
            .format_evidence
            .iter()
            .any(|evidence| evidence.contains("SecretPayroll")));
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
