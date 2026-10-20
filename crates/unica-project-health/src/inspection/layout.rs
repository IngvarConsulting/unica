use crate::{
    ProjectCheckId, ProjectCheckObservation, ProjectCheckOutcome, ProjectHealthFact,
    ProjectHealthInspectionError,
};
use unica_source_contract::{ProjectSourceSet, SourceFormat};
use unica_work_control::cancellation::CancellationToken;
use unica_work_control::provider_deadline::ProviderDeadline;

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use unica_source_contract::discovery::NamedSourceSetErrorKind;
use unica_source_contract::discovery::ProjectDiscoveryRead;
use unica_source_contract::workspace::WorkspaceContext;
use unica_source_designer::project_sources::discover_project_source_map_controlled;
use unica_workspace::discovery::{
    inspect_declared_source_root_route, reject_linked_source_root_route,
};
use unica_workspace::paths::normalize_path_identity;
use unica_workspace::read::{open_absolute_read_directory_nofollow, open_any_read_child_nofollow};

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
        discovery: &dyn ProjectDiscoveryRead,
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
            } else if deadline.is_elapsed() {
                Err(SOURCE_DISCOVERY_DEADLINE.to_string())
            } else {
                Ok(())
            }
        };
        let source_map = match discover_project_source_map_controlled(discovery, &mut checkpoint) {
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
                                let source_directory = match open_absolute_read_directory_nofollow(
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
                                match open_any_read_child_nofollow(
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
                                if unica_workspace::paths::path_starts_with_host_root(
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
    if deadline.is_elapsed() {
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
#[path = "../../tests/platform/project_health_layout.rs"]
mod tests;
