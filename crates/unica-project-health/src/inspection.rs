pub(crate) mod git;
pub(crate) mod layout;
pub mod resources;

pub(crate) const PROJECT_HEALTH_STDOUT_CAPTURE_LIMIT: usize = 64 * 1024 * 1024;

use crate::{
    ProjectCheckId, ProjectCheckOutcome, ProjectHealthFact, ProjectHealthInspectionError,
    ProjectHealthSnapshot,
};
use git::GitRepositoryInspector;
use layout::SourceLayoutInspector;
use resources::{resource_observations, ResourceContinuation, SourceResourcePolicyInspector};
#[cfg(test)]
use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::path::Path;
use std::time::Duration;
use unica_process::runner::{system_process_runner, ProcessRunner};
use unica_source_contract::discovery::ProjectDiscoveryRead;
use unica_source_contract::workspace::WorkspaceContext;

#[cfg(test)]
pub(crate) const TEST_RESPONSE_HANDOFF_WINDOW: Duration = Duration::from_secs(7);
use unica_source_contract::SourceSetKind;
use unica_source_designer::probe::ConfigDumpInfoXmlKind;
use unica_work_control::cancellation::CancellationToken;
use unica_work_control::provider_deadline::ProviderDeadline;

#[cfg(test)]
thread_local! {
    static CANCEL_BEFORE_SNAPSHOT: Cell<bool> = const { Cell::new(false) };
}

#[derive(Default)]
struct SourceRootOwnerNode<'a> {
    exact_children: BTreeMap<OsString, usize>,
    identity_children: BTreeMap<unica_workspace::read::FileIdentity, usize>,
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
        if let Some(identity) = unica_workspace::paths::host_directory_child_identity(
            &self.nodes[node].path,
            component,
        )? {
            if let Some(child) = self.nodes[node].identity_children.get(&identity) {
                return Ok(Some(*child));
            }
            for child in &self.nodes[node].missing_children {
                if unica_workspace::paths::host_directory_child_names_equal(
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
            if unica_workspace::paths::host_path_components_equal(
                &child.component,
                component,
                self.nodes[node].case_sensitive,
            )? {
                return Ok(Some(child.node));
            }
        }
        Ok(None)
    }

    #[cfg(test)]
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
            unica_workspace::paths::host_filesystem_case_sensitive(path)
        })
    }

    fn new_with_case_policy<E>(
        repository_root: &Path,
        roots: impl IntoIterator<Item = &'a layout::InspectedSourceRoot>,
        checkpoint: &mut dyn FnMut() -> Result<(), E>,
        case_policy: &dyn Fn(&Path) -> std::io::Result<bool>,
    ) -> Result<Self, SourceRootOwnerIndexError<E>> {
        let repository_root = unica_workspace::paths::normalize_path_identity(repository_root)
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
            let Ok(path) = unica_workspace::paths::normalize_path_identity(&root.path) else {
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
                let child_identity = unica_workspace::paths::host_directory_child_identity(
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
        } else if deadline.is_elapsed() {
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
            root.source_set.source_format == unica_source_contract::SourceFormat::PlatformXml
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
                root.source_set.source_format = unica_source_contract::SourceFormat::PlatformXml;
                root
            })
            .collect(),
        incomplete_source_sets,
    })
}

fn staged_platform_format_marker(
    source_set: &unica_source_contract::ProjectSourceSet,
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

pub fn inspect_project_health_continued(
    context: &WorkspaceContext,
    cancellation: &CancellationToken,
    deadline: ProviderDeadline,
    continuation: Option<&mut ResourceContinuation>,
    discovery: &dyn ProjectDiscoveryRead,
    response_handoff_window: Duration,
) -> Result<ProjectHealthSnapshot, ProjectHealthInspectionError> {
    inspect_project_health_with(
        context,
        cancellation,
        deadline,
        system_process_runner(),
        continuation,
        discovery,
        response_handoff_window,
    )
}

#[cfg(test)]
pub(crate) fn inspect_project_health_with_runner(
    context: &WorkspaceContext,
    cancellation: &CancellationToken,
    deadline: ProviderDeadline,
    runner: &dyn ProcessRunner,
) -> Result<ProjectHealthSnapshot, ProjectHealthInspectionError> {
    inspect_project_health_with(
        context,
        cancellation,
        deadline,
        runner,
        None,
        &unica_workspace::discovery::ProjectDiscoveryReader::new(&context.workspace_root),
        TEST_RESPONSE_HANDOFF_WINDOW,
    )
}

/// Тестовый счётчик проверок здоровья проекта по корню рабочего пространства.
#[cfg(any(test, feature = "test-support"))]
pub mod health_inspection_probe {
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;

    static INSPECTED: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

    fn canonical(path: &Path) -> PathBuf {
        std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
    }

    pub(super) fn record(workspace_root: &Path) {
        INSPECTED
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .push(canonical(workspace_root));
    }

    /// Number of project-health inspections started for `workspace_root`.
    pub fn inspections_for_test(workspace_root: &Path) -> usize {
        let root = canonical(workspace_root);
        INSPECTED
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .iter()
            .filter(|inspected| **inspected == root)
            .count()
    }
}

fn inspect_project_health_with(
    context: &WorkspaceContext,
    cancellation: &CancellationToken,
    deadline: ProviderDeadline,
    runner: &dyn ProcessRunner,
    continuation: Option<&mut ResourceContinuation>,
    discovery: &dyn ProjectDiscoveryRead,
    response_handoff_window: Duration,
) -> Result<ProjectHealthSnapshot, ProjectHealthInspectionError> {
    #[cfg(any(test, feature = "test-support"))]
    health_inspection_probe::record(&context.workspace_root);
    let layout = SourceLayoutInspector::inspect(context, cancellation, deadline, discovery)?;
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
            root.source_set.source_format == unica_source_contract::SourceFormat::PlatformXml
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
            source_set.source_format == unica_source_contract::SourceFormat::Edt
                || eligible_resource_source_sets.contains(source_set.name.as_str())
        });
    if let Some(repository_root) = git.repository_root.as_ref().filter(|_| {
        repository_index_complete
            && git.resource_inspection_blocker.is_none()
            && resource_eligibility_reason.is_none()
            && !eligible_resource_roots.is_empty()
    }) {
        let resources =
            SourceResourcePolicyInspector::with_process_runner(runner, response_handoff_window)
                .inspect_excluding_continued(
                    repository_root,
                    &eligible_resource_roots,
                    &git.entries,
                    &excluded_config_dump_info_paths,
                    cancellation,
                    deadline,
                    continuation,
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
                source_set.source_format == unica_source_contract::SourceFormat::PlatformXml
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
    matrix: &mut Vec<crate::ProjectCheckObservation>,
    inspected: Vec<crate::ProjectCheckObservation>,
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
    observations: &mut Vec<crate::ProjectCheckObservation>,
    facts: &[crate::ProjectHealthFact],
    source_sets: &[&unica_source_contract::ProjectSourceSet],
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
            observations.push(crate::ProjectCheckObservation {
                id,
                scope: id.scope(),
                source_set: Some(source_set.name.clone()),
                outcome: crate::ProjectCheckOutcome::NotRun {
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
    fact: &crate::ProjectHealthFact,
    id: ProjectCheckId,
) -> bool {
    use crate::ProjectHealthFact;

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
    source_sets: Option<&[unica_source_contract::ProjectSourceSet]>,
) -> Vec<&unica_source_contract::ProjectSourceSet> {
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
#[path = "../tests/platform/project_health_inspection.rs"]
mod tests;
