pub(crate) mod git;
pub(crate) mod layout;
pub(crate) mod resources;

pub(crate) const PROJECT_HEALTH_STDOUT_CAPTURE_LIMIT: usize = 64 * 1024 * 1024;

use crate::domain::cancellation::CancellationToken;
use crate::domain::code_intelligence::ProviderDeadline;
use crate::domain::project_health::{
    ProjectCheckId, ProjectCheckOutcome, ProjectHealthFact, ProjectHealthInspectionError,
    ProjectHealthSnapshot,
};
use crate::domain::project_sources::{ConfigDumpInfoXmlKind, SourceSetKind};
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::internal_adapters::{system_process_runner, ProcessRunner};
use git::GitRepositoryInspector;
use layout::SourceLayoutInspector;
use resources::{resource_observations, SourceResourcePolicyInspector};
#[cfg(test)]
use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::path::Path;

#[cfg(test)]
thread_local! {
    static CANCEL_BEFORE_SNAPSHOT: Cell<bool> = const { Cell::new(false) };
}

#[derive(Default)]
struct SourceRootOwnerNode<'a> {
    exact_children: BTreeMap<OsString, usize>,
    identity_children: BTreeMap<crate::infrastructure::platform::filesystem::FileIdentity, usize>,
    missing_children: Vec<SourceRootOwnerChild>,
    owners: Vec<&'a layout::InspectedSourceRoot>,
    depth: usize,
    case_sensitive: bool,
    path: std::path::PathBuf,
}

struct SourceRootOwnerChild {
    component: OsString,
    node: usize,
}

pub(super) struct SourceRootOwnerIndex<'a> {
    nodes: Vec<SourceRootOwnerNode<'a>>,
}

impl<'a> SourceRootOwnerIndex<'a> {
    fn child_for_component(
        &self,
        node: usize,
        component: &std::ffi::OsStr,
    ) -> std::io::Result<Option<usize>> {
        if let Some(child) = self.nodes[node].exact_children.get(component) {
            return Ok(Some(*child));
        }
        if let Some(identity) =
            crate::infrastructure::platform::filesystem::host_directory_child_identity(
                &self.nodes[node].path,
                component,
            )?
        {
            if let Some(child) = self.nodes[node].identity_children.get(&identity) {
                return Ok(Some(*child));
            }
            for child in &self.nodes[node].missing_children {
                if crate::infrastructure::platform::filesystem::host_directory_child_names_equal(
                    &self.nodes[node].path,
                    &child.component,
                    component,
                    self.nodes[node].case_sensitive,
                )? {
                    return Ok(Some(child.node));
                }
            }
            return Ok(None);
        }
        for child in &self.nodes[node].missing_children {
            if crate::infrastructure::platform::filesystem::host_path_components_equal(
                &child.component,
                component,
                self.nodes[node].case_sensitive,
            )? {
                return Ok(Some(child.node));
            }
        }
        Ok(None)
    }

    pub(super) fn new(
        repository_root: &Path,
        roots: impl IntoIterator<Item = &'a layout::InspectedSourceRoot>,
    ) -> Result<Self, String> {
        Self::new_with_checkpoint(repository_root, roots, &mut || Ok::<_, ()>(())).map_err(
            |error| match error {
                SourceRootOwnerIndexError::Checkpoint(()) => {
                    unreachable!("uncontrolled source-root owner index construction cannot stop")
                }
                SourceRootOwnerIndexError::Path(reason) => reason,
            },
        )
    }

    pub(super) fn new_with_checkpoint<E>(
        repository_root: &Path,
        roots: impl IntoIterator<Item = &'a layout::InspectedSourceRoot>,
        checkpoint: &mut dyn FnMut() -> Result<(), E>,
    ) -> Result<Self, SourceRootOwnerIndexError<E>> {
        Self::new_with_case_policy(repository_root, roots, checkpoint, &|path| {
            crate::infrastructure::platform::filesystem::host_filesystem_case_sensitive(path)
        })
    }

    fn new_with_case_policy<E>(
        repository_root: &Path,
        roots: impl IntoIterator<Item = &'a layout::InspectedSourceRoot>,
        checkpoint: &mut dyn FnMut() -> Result<(), E>,
        case_policy: &dyn Fn(&Path) -> std::io::Result<bool>,
    ) -> Result<Self, SourceRootOwnerIndexError<E>> {
        let repository_root =
            crate::infrastructure::source_roots::normalize_path_identity(repository_root)
                .map_err(SourceRootOwnerIndexError::Path)?;
        let root_node = SourceRootOwnerNode {
            case_sensitive: case_policy(&repository_root).map_err(|error| {
                SourceRootOwnerIndexError::Path(format!(
                    "failed to determine filesystem path identity at {}: {error}",
                    repository_root.display()
                ))
            })?,
            path: repository_root.clone(),
            ..SourceRootOwnerNode::default()
        };
        let mut result = Self {
            nodes: vec![root_node],
        };
        for root in roots {
            checkpoint().map_err(SourceRootOwnerIndexError::Checkpoint)?;
            let Ok(path) = crate::infrastructure::source_roots::normalize_path_identity(&root.path)
            else {
                continue;
            };
            let Ok(relative) = path.strip_prefix(&repository_root) else {
                continue;
            };
            let mut node = 0_usize;
            let mut depth = 0_usize;
            let mut parent_path = repository_root.clone();
            for (component_index, component) in relative.components().enumerate() {
                if component_index % 256 == 0 {
                    checkpoint().map_err(SourceRootOwnerIndexError::Checkpoint)?;
                }
                let child_identity =
                    crate::infrastructure::platform::filesystem::host_directory_child_identity(
                        &result.nodes[node].path,
                        component.as_os_str(),
                    )
                    .map_err(|error| {
                        SourceRootOwnerIndexError::Path(format!(
                            "failed to inspect filesystem path identity at {}: {error}",
                            parent_path.display()
                        ))
                    })?;
                let child = if let Some(child) = result
                    .child_for_component(node, component.as_os_str())
                    .map_err(|error| {
                        SourceRootOwnerIndexError::Path(format!(
                            "failed to compare filesystem path identity at {}: {error}",
                            parent_path.display()
                        ))
                    })? {
                    result.nodes[node]
                        .exact_children
                        .insert(component.as_os_str().to_os_string(), child);
                    child
                } else {
                    let child = result.nodes.len();
                    let mut child_path = parent_path.clone();
                    child_path.push(component.as_os_str());
                    let case_sensitive = match case_policy(&child_path) {
                        Ok(case_sensitive) => case_sensitive,
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                            // A declared root may be missing. A child that does not
                            // exist cannot carry a platform-specific override yet,
                            // so it inherits the already-proven parent policy.
                            result.nodes[node].case_sensitive
                        }
                        Err(error) => {
                            return Err(SourceRootOwnerIndexError::Path(format!(
                                "failed to determine filesystem path identity at {}: {error}",
                                child_path.display()
                            )))
                        }
                    };
                    result.nodes.push(SourceRootOwnerNode {
                        case_sensitive,
                        path: child_path,
                        ..SourceRootOwnerNode::default()
                    });
                    result.nodes[node]
                        .exact_children
                        .insert(component.as_os_str().to_os_string(), child);
                    if let Some(identity) = child_identity {
                        result.nodes[node].identity_children.insert(identity, child);
                    } else {
                        result.nodes[node]
                            .missing_children
                            .push(SourceRootOwnerChild {
                                component: component.as_os_str().to_os_string(),
                                node: child,
                            });
                    }
                    child
                };
                parent_path.push(component.as_os_str());
                node = child;
                depth += 1;
            }
            result.nodes[node].owners.push(root);
            result.nodes[node].depth = depth;
        }
        Ok(result)
    }

    pub(super) fn deepest_owners_with_checkpoint<E>(
        &self,
        repo_path: &str,
        checkpoint: &mut dyn FnMut() -> Result<(), E>,
    ) -> Result<Option<(&[&'a layout::InspectedSourceRoot], usize)>, SourceRootOwnerLookupError<E>>
    {
        let mut node = 0_usize;
        let mut deepest = (!self.nodes[0].owners.is_empty()).then_some(0);
        for (component_index, component) in Path::new(repo_path).components().enumerate() {
            if component_index % 256 == 0 {
                checkpoint().map_err(SourceRootOwnerLookupError::Checkpoint)?;
            }
            let Some(child) = self
                .child_for_component(node, component.as_os_str())
                .map_err(|error| {
                    SourceRootOwnerLookupError::Path(format!(
                        "failed to compare repository path identity at {repo_path}: {error}"
                    ))
                })?
            else {
                break;
            };
            node = child;
            if !self.nodes[node].owners.is_empty() {
                deepest = Some(node);
            }
        }
        Ok(deepest.map(|node| (self.nodes[node].owners.as_slice(), self.nodes[node].depth)))
    }
}

#[derive(Debug)]
pub(super) enum SourceRootOwnerIndexError<E> {
    Checkpoint(E),
    Path(String),
}

#[derive(Debug)]
pub(super) enum SourceRootOwnerLookupError<E> {
    Checkpoint(E),
    Path(String),
}

#[derive(Debug)]
enum ResourceRootEligibilityError {
    Cancelled,
    TimedOut,
    Incomplete(String),
}

struct ResourceRootEligibility {
    roots: Vec<layout::InspectedSourceRoot>,
    incomplete_source_sets: BTreeSet<String>,
}

enum StagedPlatformFormatMarker {
    Present,
    Absent,
    Incomplete,
}

fn eligible_resource_roots_from_staged_index(
    repository_root: &Path,
    roots: &[layout::InspectedSourceRoot],
    entries: &[git::GitIndexEntry],
    staged_config_dump_info_kinds: &BTreeMap<String, ConfigDumpInfoXmlKind>,
    cancellation: &CancellationToken,
    deadline: ProviderDeadline,
) -> Result<ResourceRootEligibility, ResourceRootEligibilityError> {
    let mut checkpoint = || {
        if cancellation.is_cancelled() {
            Err(ResourceRootEligibilityError::Cancelled)
        } else if deadline.remaining().is_zero() {
            Err(ResourceRootEligibilityError::TimedOut)
        } else {
            Ok(())
        }
    };
    let owners =
        SourceRootOwnerIndex::new_with_checkpoint(repository_root, roots.iter(), &mut checkpoint)
            .map_err(|error| match error {
            SourceRootOwnerIndexError::Checkpoint(error) => error,
            SourceRootOwnerIndexError::Path(reason) => {
                ResourceRootEligibilityError::Incomplete(reason)
            }
        })?;
    let mut eligible = roots
        .iter()
        .filter(|root| {
            root.source_set.source_format
                == crate::domain::project_sources::SourceFormat::PlatformXml
        })
        .map(|root| root.source_set.name.clone())
        .collect::<BTreeSet<_>>();
    let mut incomplete_source_sets = BTreeSet::new();
    for (entry_index, entry) in entries.iter().enumerate() {
        if entry_index.is_multiple_of(256) {
            checkpoint()?;
        }
        let Some((entry_owners, prefix_depth)) = owners
            .deepest_owners_with_checkpoint(&entry.repo_path, &mut checkpoint)
            .map_err(|error| match error {
                SourceRootOwnerLookupError::Checkpoint(error) => error,
                SourceRootOwnerLookupError::Path(reason) => {
                    ResourceRootEligibilityError::Incomplete(reason)
                }
            })?
        else {
            continue;
        };
        let mut relative = String::new();
        for (component_index, component) in Path::new(&entry.repo_path)
            .components()
            .skip(prefix_depth)
            .enumerate()
        {
            if component_index.is_multiple_of(256) {
                checkpoint()?;
            }
            if !relative.is_empty() {
                relative.push('/');
            }
            relative.push_str(&component.as_os_str().to_string_lossy());
        }
        for (owner_index, root) in entry_owners.iter().enumerate() {
            if owner_index.is_multiple_of(256) {
                checkpoint()?;
            }
            match staged_platform_format_marker(
                &root.source_set,
                &relative,
                &entry.repo_path,
                staged_config_dump_info_kinds,
            ) {
                StagedPlatformFormatMarker::Present => {
                    if !eligible.contains(root.source_set.name.as_str()) {
                        eligible.insert(root.source_set.name.clone());
                    }
                    incomplete_source_sets.remove(&root.source_set.name);
                }
                StagedPlatformFormatMarker::Incomplete
                    if !eligible.contains(&root.source_set.name) =>
                {
                    if !incomplete_source_sets.contains(root.source_set.name.as_str()) {
                        incomplete_source_sets.insert(root.source_set.name.clone());
                    }
                }
                StagedPlatformFormatMarker::Absent | StagedPlatformFormatMarker::Incomplete => {}
            }
        }
    }
    checkpoint()?;
    Ok(ResourceRootEligibility {
        roots: roots
            .iter()
            .filter(|root| eligible.contains(&root.source_set.name))
            .cloned()
            .map(|mut root| {
                root.source_set.source_format =
                    crate::domain::project_sources::SourceFormat::PlatformXml;
                root
            })
            .collect(),
        incomplete_source_sets,
    })
}

fn staged_platform_format_marker(
    source_set: &crate::domain::project_sources::ProjectSourceSet,
    relative: &str,
    repo_path: &str,
    staged_config_dump_info_kinds: &BTreeMap<String, ConfigDumpInfoXmlKind>,
) -> StagedPlatformFormatMarker {
    match source_set.kind {
        SourceSetKind::Configuration | SourceSetKind::Extension => {
            if relative == "Configuration.xml" {
                StagedPlatformFormatMarker::Present
            } else {
                StagedPlatformFormatMarker::Absent
            }
        }
        SourceSetKind::ExternalProcessor | SourceSetKind::ExternalReport => {
            if relative.contains('/')
                || !Path::new(relative)
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("xml"))
            {
                return StagedPlatformFormatMarker::Absent;
            }
            if !Path::new(relative)
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.eq_ignore_ascii_case("ConfigDumpInfo.xml"))
            {
                return StagedPlatformFormatMarker::Present;
            }
            match staged_config_dump_info_kinds.get(repo_path) {
                Some(ConfigDumpInfoXmlKind::ExternalProcessor)
                    if source_set.kind == SourceSetKind::ExternalProcessor =>
                {
                    StagedPlatformFormatMarker::Present
                }
                Some(ConfigDumpInfoXmlKind::ExternalReport)
                    if source_set.kind == SourceSetKind::ExternalReport =>
                {
                    StagedPlatformFormatMarker::Present
                }
                Some(
                    ConfigDumpInfoXmlKind::ExternalProcessor
                    | ConfigDumpInfoXmlKind::ExternalReport,
                ) => StagedPlatformFormatMarker::Incomplete,
                Some(
                    ConfigDumpInfoXmlKind::RuntimeSidecar
                    | ConfigDumpInfoXmlKind::MetadataDescriptor
                    | ConfigDumpInfoXmlKind::Other,
                ) => StagedPlatformFormatMarker::Absent,
                None => StagedPlatformFormatMarker::Incomplete,
            }
        }
    }
}

pub(crate) fn inspect_project_health(
    context: &WorkspaceContext,
    cancellation: &CancellationToken,
    deadline: ProviderDeadline,
) -> Result<ProjectHealthSnapshot, ProjectHealthInspectionError> {
    inspect_project_health_with(context, cancellation, deadline, system_process_runner())
}

#[cfg(test)]
pub(crate) fn inspect_project_health_with_runner(
    context: &WorkspaceContext,
    cancellation: &CancellationToken,
    deadline: ProviderDeadline,
    runner: &dyn ProcessRunner,
) -> Result<ProjectHealthSnapshot, ProjectHealthInspectionError> {
    inspect_project_health_with(context, cancellation, deadline, runner)
}

fn inspect_project_health_with(
    context: &WorkspaceContext,
    cancellation: &CancellationToken,
    deadline: ProviderDeadline,
    runner: &dyn ProcessRunner,
) -> Result<ProjectHealthSnapshot, ProjectHealthInspectionError> {
    let layout = SourceLayoutInspector::inspect(context, cancellation, deadline)?;
    if cancellation.is_cancelled() {
        return Err(ProjectHealthInspectionError::Cancelled);
    }
    let git = GitRepositoryInspector::with_process_runner(runner).inspect_base(
        context,
        &layout,
        cancellation,
        deadline,
    )?;
    let repository_index_complete = git.observations.iter().any(|observation| {
        observation.id == ProjectCheckId::RepositoryIndex
            && observation.source_set.is_none()
            && matches!(observation.outcome, ProjectCheckOutcome::Completed)
    });
    let mut observations = layout.observations;
    observations.extend(git.observations);
    let mut facts = layout.facts;
    facts.extend(git.facts);
    let declared_source_sets = uniquely_addressable_source_sets(layout.source_sets.as_deref());
    let repository_matrix_reason = if !layout.source_targets_complete {
        "source-set targets are incomplete"
    } else if git.repository_root.is_none() {
        "Git repository root is unavailable"
    } else {
        "repository check was not reached"
    };
    complete_repository_matrix(
        &mut observations,
        &facts,
        &declared_source_sets,
        repository_matrix_reason,
    );
    let fallback_resource_roots = layout
        .roots
        .iter()
        .filter(|root| {
            root.source_set.source_format
                == crate::domain::project_sources::SourceFormat::PlatformXml
        })
        .cloned()
        .collect::<Vec<_>>();
    let (eligible_resource_roots, incomplete_resource_source_sets, resource_eligibility_reason) =
        if repository_index_complete {
            if let Some(repository_root) = git.repository_root.as_ref() {
                match eligible_resource_roots_from_staged_index(
                    repository_root,
                    &layout.roots,
                    &git.entries,
                    &git.staged_config_dump_info_kinds,
                    cancellation,
                    deadline,
                ) {
                    Ok(eligibility) => {
                        (eligibility.roots, eligibility.incomplete_source_sets, None)
                    }
                    Err(ResourceRootEligibilityError::Cancelled) => {
                        return Err(ProjectHealthInspectionError::Cancelled)
                    }
                    Err(ResourceRootEligibilityError::TimedOut) => {
                        let reason =
                            "staged source-format evidence exceeded the inspection deadline";
                        facts.push(ProjectHealthFact::GitInspectionTimeout {
                            check: ProjectCheckId::RepositoryAttributes,
                            source_set: None,
                        });
                        (Vec::new(), BTreeSet::new(), Some(reason.to_string()))
                    }
                    Err(ResourceRootEligibilityError::Incomplete(reason)) => {
                        facts.push(ProjectHealthFact::GitInspectionIncomplete {
                            check: ProjectCheckId::RepositoryAttributes,
                            source_set: None,
                            reason: reason.clone(),
                        });
                        (Vec::new(), BTreeSet::new(), Some(reason))
                    }
                }
            } else {
                (fallback_resource_roots.clone(), BTreeSet::new(), None)
            }
        } else {
            (fallback_resource_roots, BTreeSet::new(), None)
        };
    let resource_reason = if let Some(reason) = resource_eligibility_reason.as_deref() {
        reason
    } else if !repository_index_complete {
        "Git index snapshot is unavailable"
    } else if let Some(reason) = git.resource_inspection_blocker.as_deref() {
        reason
    } else if git.repository_root.is_none() {
        "Git repository root is unavailable"
    } else {
        "source-set targets are incomplete"
    };
    let mut resource_matrix = resource_observations(
        declared_source_sets.iter().copied(),
        resource_reason,
        layout.source_targets_complete,
    );
    if !eligible_resource_roots.is_empty() {
        let eligible_names = eligible_resource_roots
            .iter()
            .map(|root| root.source_set.name.as_str())
            .collect::<BTreeSet<_>>();
        for observation in &mut resource_matrix {
            if observation
                .source_set
                .as_deref()
                .is_none_or(|source_set| eligible_names.contains(source_set))
                && matches!(
                    observation.outcome,
                    ProjectCheckOutcome::NotApplicable { .. }
                )
            {
                observation.outcome = ProjectCheckOutcome::NotRun {
                    reason: resource_reason.into(),
                };
            }
        }
    }
    if !incomplete_resource_source_sets.is_empty() {
        let reason = git
            .resource_inspection_blocker
            .as_deref()
            .unwrap_or("staged ConfigDumpInfo source-format classification is incomplete");
        for observation in &mut resource_matrix {
            if observation
                .source_set
                .as_deref()
                .is_none_or(|source_set| incomplete_resource_source_sets.contains(source_set))
            {
                observation.outcome = ProjectCheckOutcome::NotRun {
                    reason: reason.into(),
                };
            }
        }
    }
    if let Some(reason) = resource_eligibility_reason.as_deref() {
        for observation in &mut resource_matrix {
            observation.outcome = ProjectCheckOutcome::NotRun {
                reason: reason.into(),
            };
        }
    }
    let excluded_config_dump_info_paths = git
        .entries
        .iter()
        .filter(|entry| {
            Path::new(&entry.repo_path)
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.eq_ignore_ascii_case("ConfigDumpInfo.xml"))
        })
        .filter(|entry| {
            !matches!(
                git.staged_config_dump_info_kinds.get(&entry.repo_path),
                Some(
                    ConfigDumpInfoXmlKind::ExternalProcessor
                        | ConfigDumpInfoXmlKind::ExternalReport
                        | ConfigDumpInfoXmlKind::MetadataDescriptor
                )
            )
        })
        .map(|entry| entry.repo_path.clone())
        .collect::<BTreeSet<_>>();
    let eligible_resource_source_sets = eligible_resource_roots
        .iter()
        .map(|root| root.source_set.name.as_str())
        .collect::<BTreeSet<_>>();
    let repository_resource_profiles_complete = incomplete_resource_source_sets.is_empty()
        && declared_source_sets.iter().all(|source_set| {
            source_set.source_format == crate::domain::project_sources::SourceFormat::Edt
                || eligible_resource_source_sets.contains(source_set.name.as_str())
        });
    if let Some(repository_root) = git.repository_root.as_ref().filter(|_| {
        repository_index_complete
            && git.resource_inspection_blocker.is_none()
            && resource_eligibility_reason.is_none()
            && !eligible_resource_roots.is_empty()
    }) {
        let resources = SourceResourcePolicyInspector::with_process_runner(runner)
            .inspect_excluding(
                repository_root,
                &eligible_resource_roots,
                &git.entries,
                &excluded_config_dump_info_paths,
                cancellation,
                deadline,
            )?;
        merge_resource_observations(
            &mut resource_matrix,
            resources.observations,
            &eligible_resource_roots,
            repository_resource_profiles_complete,
        );
        facts.extend(resources.facts);
    } else {
        let platform_resource_policy_required = !eligible_resource_roots.is_empty()
            || !incomplete_resource_source_sets.is_empty()
            || declared_source_sets.iter().any(|source_set| {
                source_set.source_format
                    == crate::domain::project_sources::SourceFormat::PlatformXml
            });
        if platform_resource_policy_required {
            if let Some(reason) = git.resource_inspection_blocker.as_ref() {
                facts.push(ProjectHealthFact::GitInspectionIncomplete {
                    check: ProjectCheckId::RepositoryAttributes,
                    source_set: None,
                    reason: reason.clone(),
                });
            }
        }
    }
    observations.extend(resource_matrix);
    #[cfg(test)]
    if CANCEL_BEFORE_SNAPSHOT.with(|slot| slot.replace(false)) {
        cancellation.cancel();
    }
    if cancellation.is_cancelled() {
        return Err(ProjectHealthInspectionError::Cancelled);
    }
    Ok(ProjectHealthSnapshot {
        workspace_root: context.workspace_root.display().to_string(),
        cache_root: context.cache_root.display().to_string(),
        repository_root: git.repository_root.map(|root| root.display().to_string()),
        source_sets: layout.source_sets,
        source_targets_complete: layout.source_targets_complete,
        observations,
        facts,
    })
}

fn merge_resource_observations(
    matrix: &mut Vec<crate::domain::project_health::ProjectCheckObservation>,
    inspected: Vec<crate::domain::project_health::ProjectCheckObservation>,
    eligible_roots: &[layout::InspectedSourceRoot],
    aggregate_complete: bool,
) {
    let eligible_names = eligible_roots
        .iter()
        .map(|root| root.source_set.name.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    for observation in inspected {
        let replace = observation
            .source_set
            .as_deref()
            .map(|source_set| eligible_names.contains(source_set))
            .unwrap_or(aggregate_complete);
        if !replace {
            continue;
        }
        matrix.retain(|existing| {
            (existing.id, existing.source_set.as_deref())
                != (observation.id, observation.source_set.as_deref())
        });
        matrix.push(observation);
    }
}

fn complete_repository_matrix(
    observations: &mut Vec<crate::domain::project_health::ProjectCheckObservation>,
    facts: &[crate::domain::project_health::ProjectHealthFact],
    source_sets: &[&crate::domain::project_sources::ProjectSourceSet],
    reason: &str,
) {
    for source_set in source_sets {
        for id in [
            ProjectCheckId::RepositoryIgnore,
            ProjectCheckId::RepositoryGeneratedPaths,
            ProjectCheckId::RepositoryConfigDumpInfo,
        ] {
            if observations.iter().any(|observation| {
                observation.id == id
                    && observation.source_set.as_deref() == Some(source_set.name.as_str())
            }) {
                continue;
            }
            observations.push(crate::domain::project_health::ProjectCheckObservation {
                id,
                scope: id.scope(),
                source_set: Some(source_set.name.clone()),
                outcome: crate::domain::project_health::ProjectCheckOutcome::NotRun {
                    reason: reason.into(),
                },
            });
        }
    }
    for id in [
        ProjectCheckId::RepositoryIgnore,
        ProjectCheckId::RepositoryGeneratedPaths,
        ProjectCheckId::RepositoryConfigDumpInfo,
    ] {
        if facts
            .iter()
            .any(|fact| ordinary_repository_fact_targets_check(fact, id))
        {
            // A proven failure dominates incomplete per-set coverage in the
            // aggregate result. Keep the aggregate observation completed so
            // the domain can publish that ordinary fact as `failed`.
            continue;
        }
        let per_set_reason = observations.iter().find_map(|observation| {
            (observation.id == id && observation.source_set.is_some())
                .then_some(&observation.outcome)
                .and_then(|outcome| match outcome {
                    ProjectCheckOutcome::NotRun { reason } => Some(reason.clone()),
                    ProjectCheckOutcome::Completed | ProjectCheckOutcome::NotApplicable { .. } => {
                        None
                    }
                })
        });
        if let Some(reason) = per_set_reason {
            if let Some(aggregate) = observations
                .iter_mut()
                .find(|observation| observation.id == id && observation.source_set.is_none())
            {
                if matches!(aggregate.outcome, ProjectCheckOutcome::Completed) {
                    aggregate.outcome = ProjectCheckOutcome::NotRun { reason };
                }
            }
        }
    }
}

fn ordinary_repository_fact_targets_check(
    fact: &crate::domain::project_health::ProjectHealthFact,
    id: ProjectCheckId,
) -> bool {
    use crate::domain::project_health::ProjectHealthFact;

    matches!(
        (id, fact),
        (
            ProjectCheckId::RepositoryIgnore,
            ProjectHealthFact::IgnoreRuleMissing { .. }
                | ProjectHealthFact::IgnoreRuleLocalOnly { .. }
        ) | (
            ProjectCheckId::RepositoryGeneratedPaths,
            ProjectHealthFact::GeneratedPathTracked { .. }
        ) | (
            ProjectCheckId::RepositoryConfigDumpInfo,
            ProjectHealthFact::RuntimeSidecarTracked { .. }
                | ProjectHealthFact::ConfigDumpInfoUnclassified { .. }
        )
    )
}

fn uniquely_addressable_source_sets(
    source_sets: Option<&[crate::domain::project_sources::ProjectSourceSet]>,
) -> Vec<&crate::domain::project_sources::ProjectSourceSet> {
    let mut counts = BTreeMap::<&str, usize>::new();
    for source_set in source_sets.into_iter().flatten() {
        *counts.entry(source_set.name.as_str()).or_default() += 1;
    }
    source_sets
        .into_iter()
        .flatten()
        .filter(|source_set| counts.get(source_set.name.as_str()) == Some(&1))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{inspect_project_health_with_runner, SourceRootOwnerIndex};
    use crate::domain::cancellation::CancellationToken;
    use crate::domain::code_intelligence::ProviderDeadline;
    use crate::domain::project_health::{ProjectCheckId, ProjectCheckOutcome, ProjectHealthFact};
    use crate::domain::project_sources::{ProjectSourceSet, SourceFormat, SourceSetKind};
    use crate::domain::workspace::WorkspaceContext;
    use crate::infrastructure::internal_adapters::{ProcessCommand, ProcessOutput, ProcessRunner};
    use std::cell::RefCell;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::Duration;
    use tempfile::TempDir;

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
            Err(crate::domain::project_health::ProjectHealthInspectionError::Cancelled)
        ));
    }

    #[test]
    fn source_owner_index_keeps_repository_root_and_equal_root_ambiguity() {
        let repository_root = Path::new("/repository");
        let root =
            |name: &str| crate::infrastructure::project_health::layout::InspectedSourceRoot {
                source_set: ProjectSourceSet {
                    name: name.into(),
                    kind: SourceSetKind::Configuration,
                    path: ".".into(),
                    source_format: SourceFormat::PlatformXml,
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
            crate::infrastructure::source_roots::normalize_path_identity(repository_root).unwrap();
        if crate::infrastructure::platform::filesystem::host_filesystem_case_sensitive(
            &repository_identity,
        )
        .unwrap()
        {
            return;
        }
        let root = crate::infrastructure::project_health::layout::InspectedSourceRoot {
            source_set: ProjectSourceSet {
                name: "main".into(),
                kind: SourceSetKind::Configuration,
                path: "src".into(),
                source_format: SourceFormat::PlatformXml,
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
            crate::infrastructure::source_roots::normalize_path_identity(repository_root).unwrap();
        if !crate::infrastructure::platform::filesystem::host_filesystem_case_sensitive(
            &repository_identity,
        )
        .unwrap()
        {
            return;
        }
        fs::create_dir_all(repository_root.join("modules")).unwrap();
        let root = crate::infrastructure::project_health::layout::InspectedSourceRoot {
            source_set: ProjectSourceSet {
                name: "upper".into(),
                kind: SourceSetKind::Configuration,
                path: "modules/MISSING/SRC".into(),
                source_format: SourceFormat::PlatformXml,
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
            .deepest_owners_with_checkpoint("modules/missing/SRC/Hidden.xml", &mut || Ok::<_, ()>(
                ()
            ),)
            .unwrap()
            .is_none());
        assert_eq!(
            index
                .deepest_owners_with_checkpoint(
                    "modules/MISSING/SRC/Hidden.xml",
                    &mut || Ok::<_, ()>(()),
                )
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
        let root = crate::infrastructure::project_health::layout::InspectedSourceRoot {
            source_set: ProjectSourceSet {
                name: "main".into(),
                kind: SourceSetKind::Configuration,
                path: "src".into(),
                source_format: SourceFormat::PlatformXml,
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
        let root = crate::infrastructure::project_health::layout::InspectedSourceRoot {
            source_set: ProjectSourceSet {
                name: "main".into(),
                kind: SourceSetKind::Configuration,
                path: "missing/nested".into(),
                source_format: SourceFormat::PlatformXml,
                format_evidence: Vec::new(),
                format_probe_error: None,
            },
            path: repository_root.join("missing/nested"),
        };

        let index = SourceRootOwnerIndex::new(repository_root, [&root]).unwrap();
        let owners = index
            .deepest_owners_with_checkpoint("missing/nested/Configuration.xml", &mut || {
                Ok::<_, ()>(())
            })
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
                observation.id != ProjectCheckId::RepositoryIndex
                    || observation.source_set.is_none()
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
                || !matches!(
                    observation.id.scope(),
                    crate::domain::project_health::DiagnosticScope::Repository
                )
        }));
        assert!(crate::domain::project_health::evaluate_project_health(snapshot).is_ok());
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
        let report = crate::domain::project_health::evaluate_project_health(snapshot).unwrap();

        for check in [
            ProjectCheckId::RepositoryAttributes,
            ProjectCheckId::RepositoryIndexEol,
            ProjectCheckId::RepositoryWorkingEol,
            ProjectCheckId::RepositoryLfs,
        ] {
            assert!(report.checks.iter().any(|reported| {
                reported.id == check.as_str()
                    && reported.source_set.as_deref() == Some("main")
                    && reported.status
                        == crate::domain::project_health::ProjectCheckStatus::NotApplicable
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
        let report = crate::domain::project_health::evaluate_project_health(snapshot).unwrap();

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
                        && reported.status
                            == crate::domain::project_health::ProjectCheckStatus::NotRun
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
        let report = crate::domain::project_health::evaluate_project_health(snapshot).unwrap();

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
                        && reported.status
                            == crate::domain::project_health::ProjectCheckStatus::NotRun
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
        let report = crate::domain::project_health::evaluate_project_health(snapshot).unwrap();

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
                        && reported.status
                            == crate::domain::project_health::ProjectCheckStatus::NotRun
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
}
