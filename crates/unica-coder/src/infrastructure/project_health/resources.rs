use crate::domain::cancellation::CancellationToken;
use crate::domain::code_intelligence::ProviderDeadline;
use crate::domain::project_health::{
    ProjectCheckId, ProjectCheckObservation, ProjectCheckOutcome, ProjectHealthFact,
    ProjectHealthInspectionError,
};
use crate::domain::project_sources::SourceFormat;
use crate::infrastructure::internal_adapters::{
    system_process_runner, ProcessCommand, ProcessOutput, ProcessRunner,
};
use crate::infrastructure::platform::filesystem::{
    host_path_text, open_absolute_directory_path_nofollow, open_directory_child_nofollow,
    open_regular_child_nofollow,
};
use crate::infrastructure::project_health::git::GitIndexEntry;
use crate::infrastructure::project_health::layout::InspectedSourceRoot;
use crate::infrastructure::project_health::{SourceRootOwnerIndex, SourceRootOwnerIndexError};
use crate::infrastructure::source_roots::normalize_path_identity;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

pub(crate) const LFS_SINGLE_FILE_THRESHOLD_BYTES: u64 = 10 * 1024 * 1024;
pub(crate) const LFS_AGGREGATE_THRESHOLD_BYTES: u64 = 100 * 1024 * 1024;
pub(crate) const MAX_WORKING_EOL_FILE_BYTES: u64 = 32 * 1024 * 1024;
pub(crate) const MAX_WORKING_EOL_TOTAL_BYTES: u64 = 256 * 1024 * 1024;
const MAX_STAGED_POLICY_FILES: usize = 1024;
const MAX_STAGED_POLICY_TOTAL_BYTES: usize = 8 * 1024 * 1024;
const MAX_INDEX_EOL_FILE_BYTES: usize = 32 * 1024 * 1024;
const MAX_INDEX_EOL_TOTAL_BYTES: usize = 256 * 1024 * 1024;
const MAX_CLASSIFIED_RESOURCES: usize = 65_536;
const MAX_RESOURCE_OWNER_EXPANSION_BYTES: usize = 8 * 1024 * 1024;
const MAX_RESOURCE_OWNERSHIP_REASON_BYTES: usize = 512;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RepositoryResourceKind {
    Text,
    Binary,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RepositoryResource {
    pub(crate) source_set: String,
    pub(crate) repo_path: String,
    pub(crate) worktree_path: PathBuf,
    pub(crate) kind: RepositoryResourceKind,
    pub(crate) blob_oid: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResourceOwnershipError {
    pub(crate) repo_path: String,
    pub(crate) source_sets: Vec<String>,
}

enum ResourceClassificationError {
    Cancelled,
    TimedOut,
    Incomplete(String),
    Ownership(ResourceOwnershipError),
}

struct ResourceClassification {
    resources: Vec<RepositoryResource>,
    ownership_errors: Vec<ResourceOwnershipError>,
}

fn reserve_resource_owner_expansion(
    retained_count: usize,
    retained: usize,
    additional_count: usize,
    additional: usize,
) -> Result<(usize, usize), ResourceClassificationError> {
    let next_count = retained_count.saturating_add(additional_count);
    if next_count > MAX_CLASSIFIED_RESOURCES {
        return Err(ResourceClassificationError::Incomplete(format!(
            "resource classification count exceeds {MAX_CLASSIFIED_RESOURCES} entries"
        )));
    }
    let next = retained.saturating_add(additional);
    if next > MAX_RESOURCE_OWNER_EXPANSION_BYTES {
        return Err(ResourceClassificationError::Incomplete(format!(
            "resource owner expansion budget exceeds {MAX_RESOURCE_OWNER_EXPANSION_BYTES} bytes"
        )));
    }
    Ok((next_count, next))
}

fn bounded_resource_ownership_reason(error: &ResourceOwnershipError) -> String {
    const PREFIX: &str =
        "resource does not have one regular stage-0 blob or has ambiguous source-set owners";
    let detailed_bytes = PREFIX
        .len()
        .saturating_add(2)
        .saturating_add(error.repo_path.len())
        .saturating_add(2)
        .saturating_add(error.source_sets.iter().fold(0_usize, |total, source_set| {
            total.saturating_add(source_set.len()).saturating_add(2)
        }));
    if detailed_bytes <= MAX_RESOURCE_OWNERSHIP_REASON_BYTES {
        return format!(
            "{PREFIX}: {}; {}",
            error.repo_path,
            error.source_sets.join(", ")
        );
    }
    format!(
        "resource ownership is ambiguous or non-regular; path bytes={}, owners={} (details omitted by inspection budget)",
        error.repo_path.len(),
        error.source_sets.len()
    )
}

pub(crate) struct RepositoryPolicyInspection {
    pub(crate) observations: Vec<ProjectCheckObservation>,
    pub(crate) facts: Vec<ProjectHealthFact>,
}

pub(crate) struct SourceResourcePolicyInspector<'a> {
    runner: &'a dyn ProcessRunner,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AttributeValues {
    text: String,
    eol: String,
    filter: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct EolValues {
    index: String,
    worktree: String,
}

struct AttributePolicyEvaluation {
    facts: Vec<ProjectHealthFact>,
    text_marked_binary: BTreeSet<String>,
}

#[derive(Debug)]
struct IsolatedGitIndex {
    _root: TempDir,
    git_dir: PathBuf,
    worktree: PathBuf,
    index: PathBuf,
    object_directory: PathBuf,
}

impl SourceResourcePolicyInspector<'static> {
    pub(crate) fn new() -> Self {
        Self {
            runner: system_process_runner(),
        }
    }
}

impl<'a> SourceResourcePolicyInspector<'a> {
    pub(crate) fn with_process_runner(runner: &'a dyn ProcessRunner) -> Self {
        Self { runner }
    }

    #[cfg(test)]
    pub(crate) fn with_runner(runner: &'a dyn ProcessRunner) -> Self {
        Self::with_process_runner(runner)
    }

    pub(crate) fn classify(
        repository_root: &Path,
        roots: &[InspectedSourceRoot],
        entries: &[GitIndexEntry],
        excluded_runtime_sidecars: &BTreeSet<String>,
    ) -> Result<Vec<RepositoryResource>, ResourceOwnershipError> {
        let classification = Self::classify_with_checkpoint(
            repository_root,
            roots,
            entries,
            excluded_runtime_sidecars,
            &mut || Ok(()),
        )
        .map_err(|error| match error {
            ResourceClassificationError::Cancelled | ResourceClassificationError::TimedOut => {
                unreachable!("uncontrolled resource classification cannot stop")
            }
            ResourceClassificationError::Incomplete(reason) => ResourceOwnershipError {
                repo_path: reason,
                source_sets: Vec::new(),
            },
            ResourceClassificationError::Ownership(error) => error,
        })?;
        if let Some(error) = classification.ownership_errors.into_iter().next() {
            Err(error)
        } else {
            Ok(classification.resources)
        }
    }

    fn classify_with_checkpoint(
        repository_root: &Path,
        roots: &[InspectedSourceRoot],
        entries: &[GitIndexEntry],
        excluded_runtime_sidecars: &BTreeSet<String>,
        checkpoint: &mut dyn FnMut() -> Result<(), ResourceClassificationError>,
    ) -> Result<ResourceClassification, ResourceClassificationError> {
        let repository_root = normalize_path_identity(repository_root).map_err(|_| {
            ResourceClassificationError::Ownership(ResourceOwnershipError {
                repo_path: repository_root.display().to_string(),
                source_sets: Vec::new(),
            })
        })?;
        let owners =
            SourceRootOwnerIndex::new_with_checkpoint(&repository_root, roots.iter(), checkpoint)
                .map_err(|error| match error {
                SourceRootOwnerIndexError::Checkpoint(error) => error,
                SourceRootOwnerIndexError::Path(reason) => {
                    ResourceClassificationError::Ownership(ResourceOwnershipError {
                        repo_path: reason,
                        source_sets: Vec::new(),
                    })
                }
            })?;
        let mut resources = Vec::new();
        let mut ownership_errors = Vec::new();
        let mut retained_owner_count = 0_usize;
        let mut retained_owner_bytes = 0_usize;
        checkpoint()?;
        for (entry_index, entry) in entries.iter().enumerate() {
            if entry_index % 256 == 0 {
                checkpoint()?;
            }
            if excluded_runtime_sidecars.contains(&entry.repo_path) {
                continue;
            }
            let Some((owners, prefix_depth)) =
                owners.deepest_owners_with_checkpoint(&entry.repo_path, checkpoint)?
            else {
                continue;
            };
            if owners.len() > 1 {
                (retained_owner_count, retained_owner_bytes) = reserve_resource_owner_expansion(
                    retained_owner_count,
                    retained_owner_bytes,
                    owners.len(),
                    owners.iter().fold(0_usize, |total, root| {
                        total.saturating_add(root.source_set.name.len())
                    }),
                )?;
                let mut source_sets = owners
                    .iter()
                    .map(|root| root.source_set.name.clone())
                    .collect::<Vec<_>>();
                source_sets.sort();
                source_sets.dedup();
                ownership_errors.push(ResourceOwnershipError {
                    repo_path: entry.repo_path.clone(),
                    source_sets,
                });
                continue;
            }
            let root = owners[0];
            if root.source_set.source_format != SourceFormat::PlatformXml {
                continue;
            }
            let mut relative = String::new();
            for (component_index, component) in Path::new(&entry.repo_path)
                .components()
                .skip(prefix_depth)
                .enumerate()
            {
                if component_index % 256 == 0 {
                    checkpoint()?;
                }
                if !relative.is_empty() {
                    relative.push('/');
                }
                relative.push_str(&component.as_os_str().to_string_lossy());
            }
            if let Some(kind) = classify_platform_xml_relative_path(&relative) {
                if entry.blob_oid.is_none()
                    || !entry
                        .mode
                        .as_deref()
                        .is_some_and(|mode| matches!(mode, "100644" | "100755"))
                {
                    (retained_owner_count, retained_owner_bytes) =
                        reserve_resource_owner_expansion(
                            retained_owner_count,
                            retained_owner_bytes,
                            1,
                            root.source_set.name.len(),
                        )?;
                    ownership_errors.push(ResourceOwnershipError {
                        repo_path: entry.repo_path.clone(),
                        source_sets: vec![root.source_set.name.clone()],
                    });
                    continue;
                }
                (retained_owner_count, retained_owner_bytes) = reserve_resource_owner_expansion(
                    retained_owner_count,
                    retained_owner_bytes,
                    1,
                    root.source_set.name.len(),
                )?;
                resources.push(RepositoryResource {
                    source_set: root.source_set.name.clone(),
                    repo_path: entry.repo_path.clone(),
                    worktree_path: repository_root.join(&entry.repo_path),
                    kind,
                    blob_oid: entry
                        .blob_oid
                        .clone()
                        .expect("regular stage-0 resource has a blob object id"),
                });
            }
        }
        resources.sort_by(|left, right| left.repo_path.cmp(&right.repo_path));
        checkpoint()?;
        Ok(ResourceClassification {
            resources,
            ownership_errors,
        })
    }

    pub(crate) fn inspect(
        &self,
        repository_root: &Path,
        roots: &[InspectedSourceRoot],
        entries: &[GitIndexEntry],
        cancellation: &CancellationToken,
        deadline: ProviderDeadline,
    ) -> Result<RepositoryPolicyInspection, ProjectHealthInspectionError> {
        self.inspect_excluding(
            repository_root,
            roots,
            entries,
            &BTreeSet::new(),
            cancellation,
            deadline,
        )
    }

    pub(crate) fn inspect_excluding(
        &self,
        repository_root: &Path,
        roots: &[InspectedSourceRoot],
        entries: &[GitIndexEntry],
        excluded_runtime_sidecars: &BTreeSet<String>,
        cancellation: &CancellationToken,
        deadline: ProviderDeadline,
    ) -> Result<RepositoryPolicyInspection, ProjectHealthInspectionError> {
        if cancellation.is_cancelled() {
            return Err(ProjectHealthInspectionError::Cancelled);
        }
        let mut observations = resource_observations(
            roots.iter().map(|root| &root.source_set),
            "repository resource check was not reached",
            true,
        );
        let classification = match Self::classify_with_checkpoint(
            repository_root,
            roots,
            entries,
            excluded_runtime_sidecars,
            &mut || {
                if cancellation.is_cancelled() {
                    Err(ResourceClassificationError::Cancelled)
                } else if deadline.remaining().is_zero() {
                    Err(ResourceClassificationError::TimedOut)
                } else {
                    Ok(())
                }
            },
        ) {
            Ok(classification) => classification,
            Err(ResourceClassificationError::Cancelled) => {
                return Err(ProjectHealthInspectionError::Cancelled)
            }
            Err(ResourceClassificationError::TimedOut) => {
                let reason = "resource ownership classification exceeded the inspection deadline";
                for check in resource_check_ids() {
                    mark_check_not_run(&mut observations, check, reason);
                }
                return Ok(RepositoryPolicyInspection {
                    observations,
                    facts: vec![ProjectHealthFact::GitInspectionTimeout {
                        check: ProjectCheckId::RepositoryAttributes,
                        source_set: None,
                    }],
                });
            }
            Err(ResourceClassificationError::Incomplete(reason)) => {
                for check in resource_check_ids() {
                    mark_check_not_run(&mut observations, check, &reason);
                }
                return Ok(RepositoryPolicyInspection {
                    observations,
                    facts: vec![ProjectHealthFact::GitInspectionIncomplete {
                        check: ProjectCheckId::RepositoryAttributes,
                        source_set: None,
                        reason,
                    }],
                });
            }
            Err(ResourceClassificationError::Ownership(error)) => {
                let reason = format!(
                    "resource ownership could not be proven from repository root {}",
                    error.repo_path
                );
                for check in resource_check_ids() {
                    mark_check_not_run(&mut observations, check, &reason);
                }
                return Ok(RepositoryPolicyInspection {
                    observations,
                    facts: vec![ProjectHealthFact::GitInspectionIncomplete {
                        check: ProjectCheckId::RepositoryAttributes,
                        source_set: None,
                        reason,
                    }],
                });
            }
        };
        let mut incomplete_reasons = BTreeMap::<String, (usize, String)>::new();
        let mut global_incomplete_reason = None::<(usize, String)>;
        for (error_index, error) in classification.ownership_errors.into_iter().enumerate() {
            if error_index.is_multiple_of(256) {
                if cancellation.is_cancelled() {
                    return Err(ProjectHealthInspectionError::Cancelled);
                }
                if deadline.remaining().is_zero() {
                    return Ok(timeout_policy(
                        observations,
                        ProjectCheckId::RepositoryAttributes,
                    ));
                }
            }
            let reason = bounded_resource_ownership_reason(&error);
            for (source_set_index, source_set) in error.source_sets.iter().enumerate() {
                if source_set_index.is_multiple_of(256) {
                    if cancellation.is_cancelled() {
                        return Err(ProjectHealthInspectionError::Cancelled);
                    }
                    if deadline.remaining().is_zero() {
                        return Ok(timeout_policy(
                            observations,
                            ProjectCheckId::RepositoryAttributes,
                        ));
                    }
                }
                let summary = incomplete_reasons
                    .entry(source_set.clone())
                    .or_insert_with(|| (0, reason.clone()));
                summary.0 = summary.0.saturating_add(1);
            }
            if error.source_sets.is_empty() {
                let summary = global_incomplete_reason.get_or_insert((0, reason));
                summary.0 = summary.0.saturating_add(1);
            }
        }
        if cancellation.is_cancelled() {
            return Err(ProjectHealthInspectionError::Cancelled);
        }
        if deadline.remaining().is_zero() {
            return Ok(timeout_policy(
                observations,
                ProjectCheckId::RepositoryAttributes,
            ));
        }
        let incomplete_source_sets = incomplete_reasons.keys().cloned().collect::<BTreeSet<_>>();
        let aggregate_reason = global_incomplete_reason
            .as_ref()
            .map(|(_, reason)| reason.clone())
            .unwrap_or_else(|| {
                format!(
                    "resource ownership is incomplete for {} source sets",
                    incomplete_source_sets.len()
                )
            });
        for observation in &mut observations {
            if !resource_check_ids().contains(&observation.id)
                || matches!(
                    observation.outcome,
                    ProjectCheckOutcome::NotApplicable { .. }
                )
            {
                continue;
            }
            let reason = match observation.source_set.as_deref() {
                Some(source_set) => incomplete_reasons.get(source_set).map(|(_, reason)| reason),
                None if !incomplete_source_sets.is_empty()
                    || global_incomplete_reason.is_some() =>
                {
                    Some(&aggregate_reason)
                }
                None => None,
            };
            if let Some(reason) = reason {
                observation.outcome = ProjectCheckOutcome::NotRun {
                    reason: reason.clone(),
                };
            }
        }
        let mut facts = Vec::with_capacity(
            incomplete_reasons
                .len()
                .saturating_add(usize::from(global_incomplete_reason.is_some())),
        );
        for (source_set_index, (source_set, (count, reason))) in
            incomplete_reasons.iter().enumerate()
        {
            if source_set_index.is_multiple_of(256) {
                if cancellation.is_cancelled() {
                    return Err(ProjectHealthInspectionError::Cancelled);
                }
                if deadline.remaining().is_zero() {
                    return Ok(timeout_policy(
                        observations,
                        ProjectCheckId::RepositoryAttributes,
                    ));
                }
            }
            facts.push(ProjectHealthFact::GitInspectionIncompleteCounted {
                check: ProjectCheckId::RepositoryAttributes,
                source_set: Some(source_set.clone()),
                reason: reason.clone(),
                count: *count,
            });
        }
        if let Some((count, reason)) = global_incomplete_reason {
            facts.push(ProjectHealthFact::GitInspectionIncompleteCounted {
                check: ProjectCheckId::RepositoryAttributes,
                source_set: None,
                reason,
                count,
            });
        }
        let resources = classification
            .resources
            .into_iter()
            .filter(|resource| !incomplete_source_sets.contains(&resource.source_set))
            .collect::<Vec<_>>();
        if resources.is_empty() {
            if observations.iter().any(|observation| {
                matches!(observation.outcome, ProjectCheckOutcome::NotRun { .. })
            }) {
                for check in resource_check_ids() {
                    mark_check_completed(&mut observations, check, &incomplete_source_sets);
                }
            }
            return Ok(RepositoryPolicyInspection {
                observations,
                facts,
            });
        }
        let paths = resources
            .iter()
            .map(|resource| resource.repo_path.clone())
            .collect::<Vec<_>>();
        let input = nul_input(&paths);
        let effective = match self.run_with_input(
            repository_root,
            [
                "check-attr",
                "-z",
                "--cached",
                "text",
                "eol",
                "filter",
                "--stdin",
            ],
            &input,
            cancellation,
            deadline,
        ) {
            Ok(output) if semantic_success(&output) => {
                match parse_attribute_records_controlled(
                    &output.stdout,
                    &paths,
                    cancellation,
                    deadline,
                ) {
                    Ok(records) => records,
                    Err(ResourceProtocolParseError::Cancelled) => {
                        return Err(ProjectHealthInspectionError::Cancelled)
                    }
                    Err(ResourceProtocolParseError::TimedOut) => {
                        return Ok(timeout_policy(
                            observations,
                            ProjectCheckId::RepositoryAttributes,
                        ));
                    }
                    Err(ResourceProtocolParseError::Malformed(reason)) => {
                        return Ok(incomplete_policy(
                            observations,
                            ProjectCheckId::RepositoryAttributes,
                            reason,
                        ));
                    }
                }
            }
            Ok(output) if output.cancelled => return Err(ProjectHealthInspectionError::Cancelled),
            Ok(output) => {
                return Ok(incomplete_policy(
                    observations,
                    ProjectCheckId::RepositoryAttributes,
                    output_reason(&output),
                ));
            }
            Err(_reason) if cancellation.is_cancelled() => {
                return Err(ProjectHealthInspectionError::Cancelled)
            }
            Err(reason) => {
                return Ok(incomplete_policy(
                    observations,
                    ProjectCheckId::RepositoryAttributes,
                    reason,
                ));
            }
        };

        let staged_attribute_entries = entries
            .iter()
            .filter(|entry| {
                Path::new(&entry.repo_path)
                    .file_name()
                    .and_then(|name| name.to_str())
                    == Some(".gitattributes")
            })
            .collect::<Vec<_>>();
        if staged_attribute_entries.len() > MAX_STAGED_POLICY_FILES {
            return Ok(incomplete_policy(
                observations,
                ProjectCheckId::RepositoryAttributes,
                format!(
                    "staged attribute policy contains {} .gitattributes files; inspection supports at most {MAX_STAGED_POLICY_FILES}",
                    staged_attribute_entries.len()
                ),
            ));
        }
        if let Err(reason) = self.validate_blob_sizes(
            repository_root,
            &staged_attribute_entries,
            MAX_STAGED_POLICY_TOTAL_BYTES,
            MAX_STAGED_POLICY_TOTAL_BYTES,
            "staged attribute policy",
            cancellation,
            deadline,
        ) {
            return if cancellation.is_cancelled() {
                Err(ProjectHealthInspectionError::Cancelled)
            } else {
                Ok(incomplete_policy(
                    observations,
                    ProjectCheckId::RepositoryAttributes,
                    reason,
                ))
            };
        }
        let staged_index = match self.create_isolated_index(
            repository_root,
            &staged_attribute_entries,
            cancellation,
            deadline,
        ) {
            Ok(index) => index,
            Err(_) if cancellation.is_cancelled() => {
                return Err(ProjectHealthInspectionError::Cancelled)
            }
            Err(reason) => {
                return if cancellation.is_cancelled() {
                    Err(ProjectHealthInspectionError::Cancelled)
                } else {
                    Ok(incomplete_policy(
                        observations,
                        ProjectCheckId::RepositoryAttributes,
                        reason,
                    ))
                };
            }
        };
        let staged_command = isolated_git_command(
            repository_root,
            &staged_index,
            vec![
                "-c".into(),
                format!(
                    "core.attributesFile={}",
                    host_path_text(
                        staged_index
                            ._root
                            .path()
                            .join("empty-attributes")
                            .display()
                            .to_string()
                    )
                ),
                "check-attr".into(),
                "-z".into(),
                "--cached".into(),
                "text".into(),
                "eol".into(),
                "filter".into(),
                "--stdin".into(),
            ],
            cancellation,
            deadline,
        );
        let staged = match sticky_process_result(
            self.runner.run_with_input(&staged_command, &input),
            cancellation,
        ) {
            Ok(output) if semantic_success(&output) => {
                match parse_attribute_records_controlled(
                    &output.stdout,
                    &paths,
                    cancellation,
                    deadline,
                ) {
                    Ok(records) => records,
                    Err(ResourceProtocolParseError::Cancelled) => {
                        return Err(ProjectHealthInspectionError::Cancelled)
                    }
                    Err(ResourceProtocolParseError::TimedOut) => {
                        return Ok(timeout_policy(
                            observations,
                            ProjectCheckId::RepositoryAttributes,
                        ));
                    }
                    Err(ResourceProtocolParseError::Malformed(reason)) => {
                        return Ok(incomplete_policy(
                            observations,
                            ProjectCheckId::RepositoryAttributes,
                            reason,
                        ));
                    }
                }
            }
            Ok(output) if output.cancelled => return Err(ProjectHealthInspectionError::Cancelled),
            Ok(output) => {
                return Ok(incomplete_policy(
                    observations,
                    ProjectCheckId::RepositoryAttributes,
                    output_reason(&output),
                ));
            }
            Err(reason) => {
                return Ok(incomplete_policy(
                    observations,
                    ProjectCheckId::RepositoryAttributes,
                    reason,
                ));
            }
        };
        let attribute_policy = match evaluate_attribute_policy_with_checkpoint(
            &resources,
            &staged,
            &effective,
            &mut || resource_protocol_checkpoint(cancellation, deadline),
        ) {
            Ok(policy) => policy,
            Err(ResourceProtocolParseError::Cancelled) => {
                return Err(ProjectHealthInspectionError::Cancelled)
            }
            Err(ResourceProtocolParseError::TimedOut) => {
                return Ok(timeout_policy(
                    observations,
                    ProjectCheckId::RepositoryAttributes,
                ));
            }
            Err(ResourceProtocolParseError::Malformed(reason)) => {
                return Ok(incomplete_policy(
                    observations,
                    ProjectCheckId::RepositoryAttributes,
                    reason,
                ));
            }
        };
        let AttributePolicyEvaluation {
            facts: attribute_facts,
            text_marked_binary,
        } = attribute_policy;
        facts.extend(attribute_facts);
        mark_check_completed(
            &mut observations,
            ProjectCheckId::RepositoryAttributes,
            &incomplete_source_sets,
        );

        let expected_text_resources = resources
            .iter()
            .filter(|resource| resource.kind == RepositoryResourceKind::Text)
            .collect::<Vec<_>>();
        if let Err(reason) = self.validate_resource_blob_sizes(
            repository_root,
            &expected_text_resources,
            cancellation,
            deadline,
        ) {
            if cancellation.is_cancelled() {
                return Err(ProjectHealthInspectionError::Cancelled);
            }
            mark_check_not_run(
                &mut observations,
                ProjectCheckId::RepositoryIndexEol,
                &reason,
            );
            facts.push(ProjectHealthFact::GitInspectionIncomplete {
                check: ProjectCheckId::RepositoryIndexEol,
                source_set: None,
                reason,
            });
            return Ok(RepositoryPolicyInspection {
                observations,
                facts,
            });
        }
        let eol_entries = expected_text_resources
            .iter()
            .map(|resource| GitIndexEntry {
                repo_path: resource.repo_path.clone(),
                blob_oid: Some(resource.blob_oid.clone()),
                mode: Some("100644".into()),
            })
            .collect::<Vec<_>>();
        let eol = if eol_entries.is_empty() {
            BTreeMap::new()
        } else {
            let eol_entry_refs = eol_entries.iter().collect::<Vec<_>>();
            let eol_index = match self.create_isolated_index(
                repository_root,
                &eol_entry_refs,
                cancellation,
                deadline,
            ) {
                Ok(index) => index,
                Err(_) if cancellation.is_cancelled() => {
                    return Err(ProjectHealthInspectionError::Cancelled)
                }
                Err(reason) => {
                    mark_check_not_run(
                        &mut observations,
                        ProjectCheckId::RepositoryIndexEol,
                        &reason,
                    );
                    facts.push(ProjectHealthFact::GitInspectionIncomplete {
                        check: ProjectCheckId::RepositoryIndexEol,
                        source_set: None,
                        reason,
                    });
                    return Ok(RepositoryPolicyInspection {
                        observations,
                        facts,
                    });
                }
            };
            let eol_command = isolated_git_command(
                repository_root,
                &eol_index,
                vec!["ls-files".into(), "--eol".into(), "-z".into()],
                cancellation,
                deadline,
            );
            let eol_output =
                match sticky_process_result(self.runner.run(&eol_command), cancellation) {
                    Ok(output) if semantic_success(&output) => output,
                    Ok(output) if output.cancelled => {
                        return Err(ProjectHealthInspectionError::Cancelled)
                    }
                    Ok(output) => {
                        let reason = output_reason(&output);
                        mark_check_not_run(
                            &mut observations,
                            ProjectCheckId::RepositoryIndexEol,
                            &reason,
                        );
                        facts.push(ProjectHealthFact::GitInspectionIncomplete {
                            check: ProjectCheckId::RepositoryIndexEol,
                            source_set: None,
                            reason,
                        });
                        return Ok(RepositoryPolicyInspection {
                            observations,
                            facts,
                        });
                    }
                    Err(reason) => {
                        mark_check_not_run(
                            &mut observations,
                            ProjectCheckId::RepositoryIndexEol,
                            &reason,
                        );
                        facts.push(ProjectHealthFact::GitInspectionIncomplete {
                            check: ProjectCheckId::RepositoryIndexEol,
                            source_set: None,
                            reason,
                        });
                        return Ok(RepositoryPolicyInspection {
                            observations,
                            facts,
                        });
                    }
                };
            let expected_text_eol_paths = expected_text_resources
                .iter()
                .map(|resource| resource.repo_path.clone())
                .collect::<Vec<_>>();
            match parse_eol_records_controlled(
                &eol_output.stdout,
                &expected_text_eol_paths,
                cancellation,
                deadline,
            ) {
                Ok(records) => records,
                Err(ResourceProtocolParseError::Cancelled) => {
                    return Err(ProjectHealthInspectionError::Cancelled)
                }
                Err(ResourceProtocolParseError::TimedOut) => {
                    let reason = "Git EOL protocol parsing exceeded the inspection deadline";
                    mark_check_not_run(
                        &mut observations,
                        ProjectCheckId::RepositoryIndexEol,
                        reason,
                    );
                    facts.push(ProjectHealthFact::GitInspectionTimeout {
                        check: ProjectCheckId::RepositoryIndexEol,
                        source_set: None,
                    });
                    return Ok(RepositoryPolicyInspection {
                        observations,
                        facts,
                    });
                }
                Err(ResourceProtocolParseError::Malformed(reason)) => {
                    mark_check_not_run(
                        &mut observations,
                        ProjectCheckId::RepositoryIndexEol,
                        &reason,
                    );
                    facts.push(ProjectHealthFact::GitInspectionIncomplete {
                        check: ProjectCheckId::RepositoryIndexEol,
                        source_set: None,
                        reason,
                    });
                    return Ok(RepositoryPolicyInspection {
                        observations,
                        facts,
                    });
                }
            }
        };
        let index_eol_facts = match index_eol_facts_with_checkpoint(
            &resources,
            &eol,
            &text_marked_binary,
            &mut || resource_protocol_checkpoint(cancellation, deadline),
        ) {
            Ok(index_eol_facts) => index_eol_facts,
            Err(ResourceProtocolParseError::Cancelled) => {
                return Err(ProjectHealthInspectionError::Cancelled)
            }
            Err(ResourceProtocolParseError::TimedOut) => {
                let reason = "Git index EOL evaluation exceeded the inspection deadline";
                mark_check_not_run(
                    &mut observations,
                    ProjectCheckId::RepositoryIndexEol,
                    reason,
                );
                facts.push(ProjectHealthFact::GitInspectionTimeout {
                    check: ProjectCheckId::RepositoryIndexEol,
                    source_set: None,
                });
                return Ok(RepositoryPolicyInspection {
                    observations,
                    facts,
                });
            }
            Err(ResourceProtocolParseError::Malformed(reason)) => {
                return Ok(incomplete_policy(
                    observations,
                    ProjectCheckId::RepositoryIndexEol,
                    reason,
                ));
            }
        };
        facts.extend(index_eol_facts);
        mark_check_completed(
            &mut observations,
            ProjectCheckId::RepositoryIndexEol,
            &incomplete_source_sets,
        );
        let mut total_working_bytes = 0_u64;
        let mut working_incomplete_source_sets = BTreeSet::new();
        let mut working_timed_out = false;
        let mut working_facts = Vec::new();
        let mut lfs_incomplete_source_sets = BTreeSet::new();
        let mut lfs_timed_out = false;
        let mut binary_by_source = BTreeMap::<String, Vec<(String, u64)>>::new();
        for resource in &resources {
            if cancellation.is_cancelled() {
                return Err(ProjectHealthInspectionError::Cancelled);
            }
            if deadline.remaining().is_zero() {
                match resource.kind {
                    RepositoryResourceKind::Text => working_timed_out = true,
                    RepositoryResourceKind::Binary => lfs_timed_out = true,
                }
                continue;
            }
            let attributes = &staged[&resource.repo_path];
            match resource.kind {
                RepositoryResourceKind::Text => {
                    // `-text` is the primary policy defect for a known platform XML
                    // resource. Git deliberately stops treating that path as text, so
                    // any EOL interpretation derived from its working bytes would be a
                    // misleading secondary diagnostic.
                    if text_marked_binary.contains(&resource.repo_path) {
                        continue;
                    }
                    match inspect_working_eol(
                        repository_root,
                        Path::new(&resource.repo_path),
                        &mut total_working_bytes,
                        cancellation,
                        deadline,
                    ) {
                        Ok(Some(WorkingEol::Mixed)) => {
                            working_facts.push(ProjectHealthFact::MixedEol {
                                source_set: resource.source_set.clone(),
                                path: resource.repo_path.clone(),
                            });
                        }
                        Ok(Some(WorkingEol::BareCr)) => {
                            working_facts.push(ProjectHealthFact::WorkingEolUnsupported {
                                source_set: resource.source_set.clone(),
                                path: resource.repo_path.clone(),
                                observed: "cr".into(),
                            });
                        }
                        Ok(Some(WorkingEol::Supported) | None) => {}
                        Err(WorkingEolInspectionError::Cancelled) => {
                            return Err(ProjectHealthInspectionError::Cancelled);
                        }
                        Err(WorkingEolInspectionError::TimedOut) => {
                            working_timed_out = true;
                        }
                        Err(WorkingEolInspectionError::Incomplete(reason)) => {
                            let reason = format!("{}: {reason}", resource.repo_path);
                            working_incomplete_source_sets.insert(resource.source_set.clone());
                            mark_source_check_not_run(
                                &mut observations,
                                ProjectCheckId::RepositoryWorkingEol,
                                &resource.source_set,
                                &reason,
                            );
                            facts.push(ProjectHealthFact::GitInspectionIncomplete {
                                check: ProjectCheckId::RepositoryWorkingEol,
                                source_set: Some(resource.source_set.clone()),
                                reason,
                            });
                        }
                    }
                }
                RepositoryResourceKind::Binary => {
                    if attributes.filter == "lfs" {
                        continue;
                    }
                    match repository_regular_file_size(
                        repository_root,
                        Path::new(&resource.repo_path),
                    ) {
                        Ok(Some(size)) => binary_by_source
                            .entry(resource.source_set.clone())
                            .or_default()
                            .push((resource.repo_path.clone(), size)),
                        Ok(None) => {}
                        Err(reason) => {
                            let reason = format!("{}: {reason}", resource.repo_path);
                            lfs_incomplete_source_sets.insert(resource.source_set.clone());
                            mark_source_check_not_run(
                                &mut observations,
                                ProjectCheckId::RepositoryLfs,
                                &resource.source_set,
                                &reason,
                            );
                            facts.push(ProjectHealthFact::GitInspectionIncomplete {
                                check: ProjectCheckId::RepositoryLfs,
                                source_set: Some(resource.source_set.clone()),
                                reason,
                            });
                        }
                    }
                }
            }
        }
        if working_timed_out {
            let reason = "deadline expired during working EOL inspection";
            mark_check_not_run(
                &mut observations,
                ProjectCheckId::RepositoryWorkingEol,
                reason,
            );
            facts.push(ProjectHealthFact::GitInspectionTimeout {
                check: ProjectCheckId::RepositoryWorkingEol,
                source_set: None,
            });
        } else {
            working_facts.retain(|fact| match fact {
                ProjectHealthFact::MixedEol { source_set, .. }
                | ProjectHealthFact::WorkingEolUnsupported { source_set, .. } => {
                    !working_incomplete_source_sets.contains(source_set)
                }
                _ => true,
            });
            let working_incomplete_source_sets = incomplete_source_sets
                .union(&working_incomplete_source_sets)
                .cloned()
                .collect();
            mark_check_completed(
                &mut observations,
                ProjectCheckId::RepositoryWorkingEol,
                &working_incomplete_source_sets,
            );
            facts.extend(working_facts);
        }
        if lfs_timed_out {
            mark_check_not_run(
                &mut observations,
                ProjectCheckId::RepositoryLfs,
                "deadline expired during LFS size inspection",
            );
            facts.push(ProjectHealthFact::GitInspectionTimeout {
                check: ProjectCheckId::RepositoryLfs,
                source_set: None,
            });
        } else {
            for source_set in &lfs_incomplete_source_sets {
                binary_by_source.remove(source_set);
            }
            match lfs_facts_with_checkpoint(binary_by_source, &mut || {
                resource_protocol_checkpoint(cancellation, deadline)
            }) {
                Ok(lfs_facts) => {
                    facts.extend(lfs_facts);
                    let lfs_incomplete_source_sets = incomplete_source_sets
                        .union(&lfs_incomplete_source_sets)
                        .cloned()
                        .collect();
                    mark_check_completed(
                        &mut observations,
                        ProjectCheckId::RepositoryLfs,
                        &lfs_incomplete_source_sets,
                    );
                }
                Err(ResourceProtocolParseError::Cancelled) => {
                    return Err(ProjectHealthInspectionError::Cancelled)
                }
                Err(ResourceProtocolParseError::TimedOut) => {
                    mark_check_not_run(
                        &mut observations,
                        ProjectCheckId::RepositoryLfs,
                        "deadline expired during LFS aggregation",
                    );
                    facts.push(ProjectHealthFact::GitInspectionTimeout {
                        check: ProjectCheckId::RepositoryLfs,
                        source_set: None,
                    });
                }
                Err(ResourceProtocolParseError::Malformed(reason)) => {
                    mark_check_not_run(&mut observations, ProjectCheckId::RepositoryLfs, &reason);
                    facts.push(ProjectHealthFact::GitInspectionIncomplete {
                        check: ProjectCheckId::RepositoryLfs,
                        source_set: None,
                        reason,
                    });
                }
            }
        }
        Ok(RepositoryPolicyInspection {
            observations,
            facts,
        })
    }

    fn run<const N: usize>(
        &self,
        cwd: &Path,
        args: [&str; N],
        cancellation: &CancellationToken,
        deadline: ProviderDeadline,
    ) -> Result<ProcessOutput, String> {
        let result = self.runner.run(&process_command(
            cwd,
            args.into_iter().map(str::to_owned).collect(),
            cancellation,
            deadline,
        ));
        sticky_process_result(result, cancellation)
    }

    fn run_with_input<const N: usize>(
        &self,
        cwd: &Path,
        args: [&str; N],
        input: &[u8],
        cancellation: &CancellationToken,
        deadline: ProviderDeadline,
    ) -> Result<ProcessOutput, String> {
        self.run_with_input_vec(
            cwd,
            args.into_iter().map(str::to_owned).collect(),
            input,
            cancellation,
            deadline,
        )
    }

    fn run_with_input_vec(
        &self,
        cwd: &Path,
        args: Vec<String>,
        input: &[u8],
        cancellation: &CancellationToken,
        deadline: ProviderDeadline,
    ) -> Result<ProcessOutput, String> {
        let result = self
            .runner
            .run_with_input(&process_command(cwd, args, cancellation, deadline), input);
        sticky_process_result(result, cancellation)
    }

    fn validate_resource_blob_sizes(
        &self,
        repository_root: &Path,
        resources: &[&RepositoryResource],
        cancellation: &CancellationToken,
        deadline: ProviderDeadline,
    ) -> Result<(), String> {
        let entries = resources
            .iter()
            .map(|resource| GitIndexEntry {
                repo_path: resource.repo_path.clone(),
                blob_oid: Some(resource.blob_oid.clone()),
                mode: Some("100644".into()),
            })
            .collect::<Vec<_>>();
        self.validate_blob_sizes(
            repository_root,
            &entries.iter().collect::<Vec<_>>(),
            MAX_INDEX_EOL_FILE_BYTES,
            MAX_INDEX_EOL_TOTAL_BYTES,
            "staged text resource policy",
            cancellation,
            deadline,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn validate_blob_sizes(
        &self,
        repository_root: &Path,
        entries: &[&GitIndexEntry],
        max_file_bytes: usize,
        max_total_bytes: usize,
        label: &str,
        cancellation: &CancellationToken,
        deadline: ProviderDeadline,
    ) -> Result<(), String> {
        let mut unique_oids = BTreeSet::new();
        for entry in entries {
            if cancellation.is_cancelled() {
                return Err(format!("{label} inspection was cancelled"));
            }
            if deadline.remaining().is_zero() {
                return Err(format!("{label} inspection timed out"));
            }
            let oid = entry.blob_oid.as_deref().ok_or_else(|| {
                format!(
                    "{label} path does not have one regular stage-0 blob: {}",
                    entry.repo_path
                )
            })?;
            if !entry
                .mode
                .as_deref()
                .is_some_and(|mode| matches!(mode, "100644" | "100755"))
            {
                return Err(format!(
                    "{label} path does not have one regular stage-0 blob: {}",
                    entry.repo_path
                ));
            }
            unique_oids.insert(oid.to_owned());
        }
        let mut input = Vec::new();
        for oid in &unique_oids {
            input.extend_from_slice(oid.as_bytes());
            input.push(b'\n');
        }
        let mut by_oid = BTreeMap::<String, usize>::new();
        if !unique_oids.is_empty() {
            let output = self.run_with_input_vec(
                repository_root,
                vec![
                    "--no-replace-objects".into(),
                    "cat-file".into(),
                    "--batch-check=%(objectname) %(objecttype) %(objectsize)".into(),
                ],
                &input,
                cancellation,
                deadline,
            )?;
            if !semantic_success(&output) {
                return Err(output_reason(&output));
            }
            for line in output.stdout.lines() {
                let fields = line.split_whitespace().collect::<Vec<_>>();
                if fields.len() != 3 || fields[1] != "blob" {
                    return Err(format!(
                        "Git returned invalid {label} blob metadata: {line}"
                    ));
                }
                let size = fields[2]
                    .parse::<usize>()
                    .map_err(|_| format!("Git returned an invalid blob size: {line}"))?;
                if by_oid.insert(fields[0].to_owned(), size).is_some() {
                    return Err(format!("Git returned duplicate {label} blob metadata"));
                }
            }
            if by_oid.len() != unique_oids.len()
                || unique_oids.iter().any(|oid| !by_oid.contains_key(oid))
            {
                return Err(format!("Git omitted {label} blob metadata"));
            }
        }
        let mut total = 0_usize;
        for entry in entries {
            let oid = entry
                .blob_oid
                .as_deref()
                .expect("validated staged blob has an object id");
            let size = by_oid[oid];
            if size > max_file_bytes {
                return Err(format!(
                    "{label} path {} is {size} bytes; inspection supports at most {max_file_bytes} bytes per file",
                    entry.repo_path
                ));
            }
            total = total.saturating_add(size);
            if total > max_total_bytes {
                return Err(format!(
                    "{label} is {total} bytes; inspection supports at most {max_total_bytes} bytes in total"
                ));
            }
        }
        Ok(())
    }

    fn create_isolated_index(
        &self,
        repository_root: &Path,
        entries: &[&GitIndexEntry],
        cancellation: &CancellationToken,
        deadline: ProviderDeadline,
    ) -> Result<IsolatedGitIndex, String> {
        let root = TempDir::new()
            .map_err(|error| format!("create isolated Git inspection directory: {error}"))?;
        let git_dir = root.path().join("git");
        let worktree = root.path().join("worktree");
        let index = root.path().join("index");
        fs::create_dir_all(git_dir.join("refs/heads"))
            .map_err(|error| format!("create isolated Git refs: {error}"))?;
        fs::create_dir_all(git_dir.join("objects"))
            .map_err(|error| format!("create isolated Git objects directory: {error}"))?;
        fs::create_dir_all(&worktree)
            .map_err(|error| format!("create isolated Git worktree: {error}"))?;
        fs::write(git_dir.join("HEAD"), b"ref: refs/heads/unborn\n")
            .map_err(|error| format!("write isolated Git HEAD: {error}"))?;
        let object_output = self.run(
            repository_root,
            [
                "rev-parse",
                "--path-format=absolute",
                "--git-path",
                "objects",
            ],
            cancellation,
            deadline,
        )?;
        if !semantic_success(&object_output) {
            return Err(output_reason(&object_output));
        }
        let object_directory_text = object_output.stdout.strip_suffix('\n').ok_or_else(|| {
            "Git object-directory output is missing its terminal newline".to_string()
        })?;
        let object_directory = PathBuf::from(object_directory_text);
        if !object_directory.is_absolute() {
            return Err("Git returned a non-absolute object directory".into());
        }
        let object_format_output = self.run(
            repository_root,
            ["rev-parse", "--show-object-format"],
            cancellation,
            deadline,
        )?;
        if !semantic_success(&object_format_output) {
            return Err(output_reason(&object_format_output));
        }
        let object_format = object_format_output
            .stdout
            .strip_suffix('\n')
            .ok_or_else(|| {
                "Git object-format output is missing its terminal newline".to_string()
            })?;
        let isolated_config = match object_format {
            "sha1" => "[core]\nrepositoryformatversion = 0\nbare = false\n".to_string(),
            "sha256" => "[core]\nrepositoryformatversion = 1\nbare = false\n[extensions]\nobjectFormat = sha256\n".to_string(),
            other => return Err(format!("unsupported Git object format: {other}")),
        };
        fs::write(git_dir.join("config"), isolated_config)
            .map_err(|error| format!("write isolated Git config: {error}"))?;
        let isolated = IsolatedGitIndex {
            _root: root,
            git_dir,
            worktree,
            index,
            object_directory,
        };
        let initialize = isolated_git_command(
            repository_root,
            &isolated,
            vec!["read-tree".into(), "--empty".into()],
            cancellation,
            deadline,
        );
        let output = sticky_process_result(self.runner.run(&initialize), cancellation)?;
        if !semantic_success(&output) {
            return Err(output_reason(&output));
        }
        if !entries.is_empty() {
            let mut index_info = Vec::new();
            for entry in entries {
                let path = Path::new(&entry.repo_path);
                if path.is_absolute()
                    || path
                        .components()
                        .any(|component| !matches!(component, std::path::Component::Normal(_)))
                {
                    return Err(format!(
                        "staged policy path is not repository-relative: {}",
                        entry.repo_path
                    ));
                }
                let mode = entry.mode.as_deref().ok_or_else(|| {
                    format!("staged policy path has no mode: {}", entry.repo_path)
                })?;
                let oid = entry.blob_oid.as_deref().ok_or_else(|| {
                    format!("staged policy path has no blob: {}", entry.repo_path)
                })?;
                index_info
                    .extend_from_slice(format!("{mode} {oid}\t{}\0", entry.repo_path).as_bytes());
            }
            let update = isolated_git_command(
                repository_root,
                &isolated,
                vec!["update-index".into(), "-z".into(), "--index-info".into()],
                cancellation,
                deadline,
            );
            let output = sticky_process_result(
                self.runner.run_with_input(&update, &index_info),
                cancellation,
            )?;
            if !semantic_success(&output) {
                return Err(output_reason(&output));
            }
        }
        Ok(isolated)
    }
}

pub(crate) fn classify_platform_xml_relative_path(
    normalized_relative_path: &str,
) -> Option<RepositoryResourceKind> {
    let segments = normalized_relative_path.split('/').collect::<Vec<_>>();
    if segments.len() == 4
        && segments[0] == "XDTOPackages"
        && !segments[1].is_empty()
        && segments[2] == "Ext"
        && segments[3] == "Package.bin"
    {
        return Some(RepositoryResourceKind::Text);
    }
    let extension = Path::new(normalized_relative_path)
        .extension()
        .and_then(|extension| extension.to_str())?
        .to_ascii_lowercase();
    if matches!(extension.as_str(), "xml" | "bsl") {
        Some(RepositoryResourceKind::Text)
    } else if matches!(
        extension.as_str(),
        "bin"
            | "axdt"
            | "addin"
            | "cf"
            | "cfe"
            | "epf"
            | "erf"
            | "png"
            | "jpg"
            | "jpeg"
            | "gif"
            | "bmp"
            | "ico"
            | "zip"
            | "7z"
            | "gz"
    ) {
        Some(RepositoryResourceKind::Binary)
    } else {
        None
    }
}

fn parse_attribute_records(
    stdout: &str,
    expected_paths: &[String],
) -> Result<BTreeMap<String, AttributeValues>, String> {
    parse_attribute_records_with_checkpoint(stdout, expected_paths, &mut || Ok(()))
        .map_err(ResourceProtocolParseError::into_reason)
}

fn parse_attribute_records_controlled(
    stdout: &str,
    expected_paths: &[String],
    cancellation: &CancellationToken,
    deadline: ProviderDeadline,
) -> Result<BTreeMap<String, AttributeValues>, ResourceProtocolParseError> {
    parse_attribute_records_with_checkpoint(stdout, expected_paths, &mut || {
        resource_protocol_checkpoint(cancellation, deadline)
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ResourceProtocolParseError {
    Malformed(String),
    Cancelled,
    TimedOut,
}

impl ResourceProtocolParseError {
    fn into_reason(self) -> String {
        match self {
            Self::Malformed(reason) => reason,
            Self::Cancelled => "Git resource protocol parsing was cancelled".into(),
            Self::TimedOut => "Git resource protocol parsing timed out".into(),
        }
    }
}

fn parse_attribute_records_with_checkpoint(
    stdout: &str,
    expected_paths: &[String],
    checkpoint: &mut dyn FnMut() -> Result<(), ResourceProtocolParseError>,
) -> Result<BTreeMap<String, AttributeValues>, ResourceProtocolParseError> {
    checkpoint()?;
    if !stdout.is_empty() && !stdout.ends_with('\0') {
        return Err(ResourceProtocolParseError::Malformed(
            "Git check-attr output is missing its terminal NUL".into(),
        ));
    }
    let mut expected = BTreeSet::new();
    for (path_index, path) in expected_paths.iter().enumerate() {
        if path_index.is_multiple_of(256) {
            checkpoint()?;
        }
        expected.insert(path.as_str());
    }
    let mut raw = BTreeMap::<String, BTreeMap<String, String>>::new();
    let mut fields = stdout.split_terminator('\0');
    let mut record_index = 0_usize;
    loop {
        if record_index.is_multiple_of(256) {
            checkpoint()?;
        }
        let Some(path) = fields.next() else { break };
        let attribute = fields.next().ok_or_else(|| {
            ResourceProtocolParseError::Malformed(
                "Git check-attr output is not a sequence of triples".into(),
            )
        })?;
        let value = fields.next().ok_or_else(|| {
            ResourceProtocolParseError::Malformed(
                "Git check-attr output is not a sequence of triples".into(),
            )
        })?;
        if !expected.contains(path) {
            return Err(ResourceProtocolParseError::Malformed(format!(
                "Git check-attr returned unexpected path {path}"
            )));
        }
        let attributes = raw.entry(path.to_owned()).or_default();
        if attributes
            .insert(attribute.to_owned(), value.to_owned())
            .is_some()
        {
            return Err(ResourceProtocolParseError::Malformed(
                "Git check-attr returned a duplicate attribute".into(),
            ));
        }
        record_index += 1;
    }
    let mut records = BTreeMap::new();
    for (path_index, path) in expected_paths.iter().enumerate() {
        if path_index.is_multiple_of(256) {
            checkpoint()?;
        }
        let attributes = raw.remove(path).ok_or_else(|| {
            ResourceProtocolParseError::Malformed(format!("Git check-attr omitted {path}"))
        })?;
        if attributes.len() != 3 {
            return Err(ResourceProtocolParseError::Malformed(format!(
                "Git check-attr returned incomplete attributes for {path}"
            )));
        }
        records.insert(
            path.clone(),
            AttributeValues {
                text: required_attribute(&attributes, "text", path)
                    .map_err(ResourceProtocolParseError::Malformed)?,
                eol: required_attribute(&attributes, "eol", path)
                    .map_err(ResourceProtocolParseError::Malformed)?,
                filter: required_attribute(&attributes, "filter", path)
                    .map_err(ResourceProtocolParseError::Malformed)?,
            },
        );
    }
    Ok(records)
}

fn required_attribute(
    attributes: &BTreeMap<String, String>,
    name: &str,
    path: &str,
) -> Result<String, String> {
    attributes
        .get(name)
        .cloned()
        .ok_or_else(|| format!("Git check-attr omitted {name} for {path}"))
}

fn parse_eol_records(
    stdout: &str,
    expected_text_paths: &[String],
) -> Result<BTreeMap<String, EolValues>, String> {
    parse_eol_records_with_checkpoint(stdout, expected_text_paths, &mut || Ok(()))
        .map_err(ResourceProtocolParseError::into_reason)
}

fn parse_eol_records_controlled(
    stdout: &str,
    expected_text_paths: &[String],
    cancellation: &CancellationToken,
    deadline: ProviderDeadline,
) -> Result<BTreeMap<String, EolValues>, ResourceProtocolParseError> {
    parse_eol_records_with_checkpoint(stdout, expected_text_paths, &mut || {
        resource_protocol_checkpoint(cancellation, deadline)
    })
}

fn resource_protocol_checkpoint(
    cancellation: &CancellationToken,
    deadline: ProviderDeadline,
) -> Result<(), ResourceProtocolParseError> {
    if cancellation.is_cancelled() {
        Err(ResourceProtocolParseError::Cancelled)
    } else if deadline.remaining().is_zero() {
        Err(ResourceProtocolParseError::TimedOut)
    } else {
        Ok(())
    }
}

fn evaluate_attribute_policy_with_checkpoint(
    resources: &[RepositoryResource],
    staged: &BTreeMap<String, AttributeValues>,
    effective: &BTreeMap<String, AttributeValues>,
    checkpoint: &mut dyn FnMut() -> Result<(), ResourceProtocolParseError>,
) -> Result<AttributePolicyEvaluation, ResourceProtocolParseError> {
    let mut evaluation = AttributePolicyEvaluation {
        facts: Vec::new(),
        text_marked_binary: BTreeSet::new(),
    };
    for (resource_index, resource) in resources.iter().enumerate() {
        if resource_index.is_multiple_of(256) {
            checkpoint()?;
        }
        let attributes = staged.get(&resource.repo_path).ok_or_else(|| {
            ResourceProtocolParseError::Malformed(format!(
                "staged Git attribute policy omitted {}",
                resource.repo_path
            ))
        })?;
        let effective_attributes = effective.get(&resource.repo_path).ok_or_else(|| {
            ResourceProtocolParseError::Malformed(format!(
                "effective Git attribute policy omitted {}",
                resource.repo_path
            ))
        })?;
        let staged_policy_sufficient = policy_satisfied(resource.kind, attributes);
        if !staged_policy_sufficient && policy_satisfied(resource.kind, effective_attributes) {
            evaluation
                .facts
                .push(ProjectHealthFact::AttributesLocalOnly {
                    source_set: resource.source_set.clone(),
                    path: resource.repo_path.clone(),
                    evidence: vec![format!(
                        "local-only text={} eol={} filter={}",
                        effective_attributes.text,
                        effective_attributes.eol,
                        effective_attributes.filter
                    )],
                });
            continue;
        }
        match resource.kind {
            RepositoryResourceKind::Text if attributes.text == "unset" => {
                evaluation
                    .text_marked_binary
                    .insert(resource.repo_path.clone());
                evaluation
                    .facts
                    .push(ProjectHealthFact::TextResourceMarkedBinary {
                        source_set: resource.source_set.clone(),
                        path: resource.repo_path.clone(),
                    });
            }
            RepositoryResourceKind::Text if !text_policy_satisfied(attributes) => {
                evaluation.facts.push(ProjectHealthFact::TextPolicyMissing {
                    source_set: resource.source_set.clone(),
                    path: resource.repo_path.clone(),
                });
            }
            RepositoryResourceKind::Binary if attributes.text != "unset" => {
                evaluation
                    .facts
                    .push(ProjectHealthFact::BinaryPolicyMissing {
                        source_set: resource.source_set.clone(),
                        path: resource.repo_path.clone(),
                    });
            }
            RepositoryResourceKind::Text | RepositoryResourceKind::Binary => {}
        }
    }
    checkpoint()?;
    Ok(evaluation)
}

fn parse_eol_records_with_checkpoint(
    stdout: &str,
    expected_text_paths: &[String],
    checkpoint: &mut dyn FnMut() -> Result<(), ResourceProtocolParseError>,
) -> Result<BTreeMap<String, EolValues>, ResourceProtocolParseError> {
    checkpoint()?;
    if !stdout.is_empty() && !stdout.ends_with('\0') {
        return Err(ResourceProtocolParseError::Malformed(
            "git ls-files --eol output is missing its terminal NUL".into(),
        ));
    }
    let mut result = BTreeMap::new();
    for (record_index, record) in stdout.split_terminator('\0').enumerate() {
        if record_index % 256 == 0 {
            checkpoint()?;
        }
        let (metadata, path) = record.split_once('\t').ok_or_else(|| {
            ResourceProtocolParseError::Malformed(
                "git ls-files --eol record has no tab separator".into(),
            )
        })?;
        if path.is_empty() {
            return Err(ResourceProtocolParseError::Malformed(
                "git ls-files --eol record has an empty path".into(),
            ));
        }
        let fields = metadata.split_whitespace().collect::<Vec<_>>();
        if fields.len() < 2 || !fields[0].starts_with("i/") || !fields[1].starts_with("w/") {
            return Err(ResourceProtocolParseError::Malformed(
                "git ls-files --eol record has invalid metadata".into(),
            ));
        }
        let index = &fields[0][2..];
        let worktree = &fields[1][2..];
        if !matches!(index, "" | "lf" | "crlf" | "mixed" | "none" | "-text") {
            return Err(ResourceProtocolParseError::Malformed(format!(
                "git ls-files --eol returned unsupported index EOL metadata {index}"
            )));
        }
        if !matches!(worktree, "" | "lf" | "crlf" | "mixed" | "none" | "-text") {
            return Err(ResourceProtocolParseError::Malformed(format!(
                "git ls-files --eol returned unsupported worktree EOL metadata {worktree}"
            )));
        }
        if result
            .insert(
                path.into(),
                EolValues {
                    index: fields[0][2..].into(),
                    worktree: fields[1][2..].into(),
                },
            )
            .is_some()
        {
            return Err(ResourceProtocolParseError::Malformed(
                "git ls-files --eol returned a duplicate path".into(),
            ));
        }
    }
    for (path_index, path) in expected_text_paths.iter().enumerate() {
        if path_index.is_multiple_of(256) {
            checkpoint()?;
        }
        let observed = result.get(path).ok_or_else(|| {
            ResourceProtocolParseError::Malformed(format!("git ls-files --eol omitted {path}"))
        })?;
        if observed.index.is_empty() {
            return Err(ResourceProtocolParseError::Malformed(format!(
                "git ls-files --eol returned empty index EOL metadata for tracked text path {path}"
            )));
        }
    }
    Ok(result)
}

fn index_eol_facts_with_checkpoint(
    resources: &[RepositoryResource],
    eol: &BTreeMap<String, EolValues>,
    text_marked_binary: &BTreeSet<String>,
    checkpoint: &mut dyn FnMut() -> Result<(), ResourceProtocolParseError>,
) -> Result<Vec<ProjectHealthFact>, ResourceProtocolParseError> {
    let mut facts = Vec::new();
    for (resource_index, resource) in resources.iter().enumerate() {
        if resource_index.is_multiple_of(256) {
            checkpoint()?;
        }
        if resource.kind != RepositoryResourceKind::Text
            || text_marked_binary.contains(&resource.repo_path)
        {
            continue;
        }
        let observed = eol.get(&resource.repo_path).ok_or_else(|| {
            ResourceProtocolParseError::Malformed(format!(
                "git ls-files --eol omitted {}",
                resource.repo_path
            ))
        })?;
        if !matches!(observed.index.as_str(), "lf" | "none") {
            facts.push(ProjectHealthFact::IndexEolNotLf {
                source_set: resource.source_set.clone(),
                path: resource.repo_path.clone(),
                observed: observed.index.clone(),
            });
        }
    }
    checkpoint()?;
    Ok(facts)
}

fn lfs_facts_with_checkpoint(
    binary_by_source: BTreeMap<String, Vec<(String, u64)>>,
    checkpoint: &mut dyn FnMut() -> Result<(), ResourceProtocolParseError>,
) -> Result<Vec<ProjectHealthFact>, ResourceProtocolParseError> {
    let mut facts = Vec::new();
    for (source_index, (source_set, files)) in binary_by_source.into_iter().enumerate() {
        if source_index.is_multiple_of(256) {
            checkpoint()?;
        }
        let mut total_bytes = 0_u64;
        let mut largest = None::<&(String, u64)>;
        let mut sample_paths = BTreeSet::new();
        for (file_index, file) in files.iter().enumerate() {
            if file_index.is_multiple_of(256) {
                checkpoint()?;
            }
            total_bytes = total_bytes.saturating_add(file.1);
            if largest.is_none_or(|current| file.1 > current.1) {
                largest = Some(file);
            }
            sample_paths.insert(file.0.clone());
            if sample_paths.len() > crate::domain::project_health::MAX_PROJECT_DIAGNOSTIC_PATHS {
                sample_paths.pop_last();
            }
        }
        if let Some((largest_path, largest_bytes)) = largest.filter(|(_, largest)| {
            *largest >= LFS_SINGLE_FILE_THRESHOLD_BYTES
                || total_bytes >= LFS_AGGREGATE_THRESHOLD_BYTES
        }) {
            facts.push(ProjectHealthFact::LfsConsider {
                source_set,
                count: files.len(),
                total_bytes,
                largest_path: largest_path.clone(),
                largest_bytes: *largest_bytes,
                single_threshold_bytes: LFS_SINGLE_FILE_THRESHOLD_BYTES,
                aggregate_threshold_bytes: LFS_AGGREGATE_THRESHOLD_BYTES,
                paths: sample_paths.into_iter().collect(),
            });
        }
    }
    checkpoint()?;
    Ok(facts)
}

fn process_command(
    cwd: &Path,
    args: Vec<String>,
    cancellation: &CancellationToken,
    deadline: ProviderDeadline,
) -> ProcessCommand {
    ProcessCommand {
        program: PathBuf::from("git"),
        args,
        cwd: cwd.to_path_buf(),
        env: vec![
            (OsString::from("LC_ALL"), OsString::from("C")),
            (OsString::from("LANG"), OsString::from("C")),
            (OsString::from("GIT_NO_LAZY_FETCH"), OsString::from("1")),
            (
                OsString::from("GIT_NO_REPLACE_OBJECTS"),
                OsString::from("1"),
            ),
            (OsString::from("GIT_OPTIONAL_LOCKS"), OsString::from("0")),
            (OsString::from("GIT_TERMINAL_PROMPT"), OsString::from("0")),
            (OsString::from("GIT_CONFIG_COUNT"), OsString::from("1")),
            (
                OsString::from("GIT_CONFIG_KEY_0"),
                OsString::from("core.fsmonitor"),
            ),
            (
                OsString::from("GIT_CONFIG_VALUE_0"),
                OsString::from("false"),
            ),
            (OsString::from("GIT_ATTR_NOSYSTEM"), OsString::from("1")),
        ],
        env_remove: super::git::git_environment_removals(),
        capture_limits: Some((
            super::PROJECT_HEALTH_STDOUT_CAPTURE_LIMIT,
            crate::infrastructure::platform::STDERR_CAPTURE_LIMIT,
        )),
        timeout: Some(deadline.remaining()),
        cancellation: cancellation.clone(),
    }
}

fn isolated_git_command(
    cwd: &Path,
    index: &IsolatedGitIndex,
    args: Vec<String>,
    cancellation: &CancellationToken,
    deadline: ProviderDeadline,
) -> ProcessCommand {
    let mut command = process_command(cwd, args, cancellation, deadline);
    command.args.splice(
        0..0,
        [
            format!(
                "--git-dir={}",
                host_path_text(index.git_dir.display().to_string())
            ),
            format!(
                "--work-tree={}",
                host_path_text(index.worktree.display().to_string())
            ),
        ],
    );
    command.env.extend([
        (
            OsString::from("GIT_INDEX_FILE"),
            index.index.as_os_str().to_os_string(),
        ),
        (
            OsString::from("GIT_OBJECT_DIRECTORY"),
            index.object_directory.as_os_str().to_os_string(),
        ),
    ]);
    command
}

fn semantic_success(output: &ProcessOutput) -> bool {
    output.status_success
        && !output.timed_out
        && !output.cancelled
        && !output.stdout_truncated
        && !output.stderr_truncated
        && !output.stdout_had_invalid_utf8
        && !output.stderr_had_invalid_utf8
}

fn sticky_process_result(
    result: Result<ProcessOutput, String>,
    cancellation: &CancellationToken,
) -> Result<ProcessOutput, String> {
    if cancellation.is_cancelled() {
        return Ok(ProcessOutput {
            cancelled: true,
            status_success: false,
            status: "cancelled".into(),
            stdout: String::new(),
            stderr: String::new(),
            timed_out: false,
            stdout_truncated: false,
            stderr_truncated: false,
            stdout_had_invalid_utf8: false,
            stderr_had_invalid_utf8: false,
        });
    }
    result
}

fn output_reason(output: &ProcessOutput) -> String {
    if output.timed_out {
        "Git resource inspection timed out".into()
    } else if output.stdout_truncated || output.stderr_truncated {
        "Git resource inspection output was truncated".into()
    } else if output.stdout_had_invalid_utf8 || output.stderr_had_invalid_utf8 {
        "Git resource inspection output contained invalid UTF-8".into()
    } else if output.stderr.trim().is_empty() {
        format!("Git resource inspection exited with {}", output.status)
    } else {
        format!(
            "Git resource inspection exited with {}: {}",
            output.status,
            output.stderr.trim()
        )
    }
}

fn incomplete_policy(
    mut observations: Vec<ProjectCheckObservation>,
    check: ProjectCheckId,
    reason: String,
) -> RepositoryPolicyInspection {
    let started = resource_check_ids()
        .into_iter()
        .position(|candidate| candidate == check)
        .unwrap_or(0);
    for dependent in resource_check_ids().into_iter().skip(started) {
        mark_check_not_run(&mut observations, dependent, &reason);
    }
    RepositoryPolicyInspection {
        observations,
        facts: vec![ProjectHealthFact::GitInspectionIncomplete {
            check,
            source_set: None,
            reason,
        }],
    }
}

fn timeout_policy(
    mut observations: Vec<ProjectCheckObservation>,
    check: ProjectCheckId,
) -> RepositoryPolicyInspection {
    let reason = "Git resource protocol parsing exceeded the inspection deadline";
    let started = resource_check_ids()
        .into_iter()
        .position(|candidate| candidate == check)
        .unwrap_or(0);
    for dependent in resource_check_ids().into_iter().skip(started) {
        mark_check_not_run(&mut observations, dependent, reason);
    }
    RepositoryPolicyInspection {
        observations,
        facts: vec![ProjectHealthFact::GitInspectionTimeout {
            check,
            source_set: None,
        }],
    }
}

fn mark_check_completed(
    observations: &mut [ProjectCheckObservation],
    check: ProjectCheckId,
    incomplete_source_sets: &BTreeSet<String>,
) {
    for observation in observations.iter_mut().filter(|observation| {
        observation.id == check
            && match observation.source_set.as_ref() {
                Some(source_set) => !incomplete_source_sets.contains(source_set),
                None => incomplete_source_sets.is_empty(),
            }
    }) {
        if matches!(observation.outcome, ProjectCheckOutcome::NotRun { .. }) {
            observation.outcome = ProjectCheckOutcome::Completed;
        }
    }
}

fn mark_check_not_run(
    observations: &mut [ProjectCheckObservation],
    check: ProjectCheckId,
    reason: &str,
) {
    for observation in observations.iter_mut().filter(|observation| {
        observation.id == check
            && !matches!(
                observation.outcome,
                ProjectCheckOutcome::NotApplicable { .. }
            )
    }) {
        observation.outcome = ProjectCheckOutcome::NotRun {
            reason: reason.to_owned(),
        };
    }
}

fn mark_source_check_not_run(
    observations: &mut [ProjectCheckObservation],
    check: ProjectCheckId,
    source_set: &str,
    reason: &str,
) {
    for observation in observations.iter_mut().filter(|observation| {
        observation.id == check
            && observation.source_set.as_deref() == Some(source_set)
            && !matches!(
                observation.outcome,
                ProjectCheckOutcome::NotApplicable { .. }
            )
    }) {
        observation.outcome = ProjectCheckOutcome::NotRun {
            reason: reason.to_owned(),
        };
    }
}

fn resource_check_ids() -> [ProjectCheckId; 4] {
    [
        ProjectCheckId::RepositoryAttributes,
        ProjectCheckId::RepositoryIndexEol,
        ProjectCheckId::RepositoryWorkingEol,
        ProjectCheckId::RepositoryLfs,
    ]
}

pub(super) fn resource_observations<'a>(
    source_sets: impl IntoIterator<Item = &'a crate::domain::project_sources::ProjectSourceSet>,
    not_run_reason: &str,
    source_profiles_complete: bool,
) -> Vec<ProjectCheckObservation> {
    let mut source_sets = source_sets.into_iter().collect::<Vec<_>>();
    source_sets.sort_by(|left, right| left.name.cmp(&right.name));
    source_sets.dedup_by(|left, right| left.name == right.name);
    let has_platform_xml = source_sets
        .iter()
        .any(|source_set| source_set.source_format == SourceFormat::PlatformXml);
    let mut observations = Vec::new();
    for id in resource_check_ids() {
        observations.push(ProjectCheckObservation {
            id,
            scope: id.scope(),
            source_set: None,
            outcome: if source_profiles_complete && !has_platform_xml {
                ProjectCheckOutcome::NotApplicable {
                    reason: "no Platform XML source roots were proven".into(),
                }
            } else {
                ProjectCheckOutcome::NotRun {
                    reason: not_run_reason.into(),
                }
            },
        });
        observations.extend(
            source_sets
                .iter()
                .map(|source_set| ProjectCheckObservation {
                    id,
                    scope: id.scope(),
                    source_set: Some(source_set.name.clone()),
                    outcome: if source_set.source_format == SourceFormat::Edt {
                        ProjectCheckOutcome::NotApplicable {
                            reason: "EDT has no format-specific repository resource policy".into(),
                        }
                    } else {
                        ProjectCheckOutcome::NotRun {
                            reason: not_run_reason.into(),
                        }
                    },
                }),
        );
    }
    observations
}

fn text_policy_satisfied(attributes: &AttributeValues) -> bool {
    attributes.text != "unset"
        && (matches!(attributes.text.as_str(), "set" | "auto")
            || matches!(attributes.eol.as_str(), "lf" | "crlf"))
}

fn policy_satisfied(kind: RepositoryResourceKind, attributes: &AttributeValues) -> bool {
    match kind {
        RepositoryResourceKind::Text => text_policy_satisfied(attributes),
        RepositoryResourceKind::Binary => attributes.text == "unset",
    }
}

fn nul_input(paths: &[String]) -> Vec<u8> {
    let mut input = Vec::new();
    for path in paths {
        input.extend_from_slice(path.as_bytes());
        input.push(0);
    }
    input
}

fn repo_path(repository_root: &Path, path: &Path) -> Option<String> {
    let root = normalize_path_identity(repository_root).ok()?;
    let path = normalize_path_identity(path).ok()?;
    let relative = path.strip_prefix(root).ok()?;
    Some(host_path_text(relative.to_string_lossy().into_owned()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorkingEol {
    Supported,
    Mixed,
    BareCr,
}

#[derive(Debug, PartialEq, Eq)]
enum WorkingEolInspectionError {
    Cancelled,
    TimedOut,
    Incomplete(String),
}

impl std::fmt::Display for WorkingEolInspectionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cancelled => formatter.write_str("working EOL inspection was cancelled"),
            Self::TimedOut => formatter.write_str("deadline expired during working EOL inspection"),
            Self::Incomplete(reason) => formatter.write_str(reason),
        }
    }
}

fn inspect_working_eol(
    repository_root: &Path,
    repo_path: &Path,
    total_working_bytes: &mut u64,
    cancellation: &CancellationToken,
    deadline: ProviderDeadline,
) -> Result<Option<WorkingEol>, WorkingEolInspectionError> {
    let file = match open_repository_regular_file_nofollow(repository_root, repo_path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(WorkingEolInspectionError::Incomplete(error.to_string())),
    };
    let metadata = file
        .metadata()
        .map_err(|error| WorkingEolInspectionError::Incomplete(error.to_string()))?;
    if metadata.len() > MAX_WORKING_EOL_FILE_BYTES {
        return Err(WorkingEolInspectionError::Incomplete(format!(
            "working file exceeds {} bytes",
            MAX_WORKING_EOL_FILE_BYTES
        )));
    }
    inspect_working_eol_reader(
        file,
        metadata.len(),
        total_working_bytes,
        cancellation,
        deadline,
    )
}

fn inspect_working_eol_reader(
    mut file: impl Read,
    metadata_len: u64,
    total_working_bytes: &mut u64,
    cancellation: &CancellationToken,
    deadline: ProviderDeadline,
) -> Result<Option<WorkingEol>, WorkingEolInspectionError> {
    let mut buffer = [0_u8; 64 * 1024];
    let mut bytes_read = 0_u64;
    let mut previous_cr = false;
    let mut lf = false;
    let mut crlf = false;
    let mut bare_cr = false;
    loop {
        if cancellation.is_cancelled() {
            return Err(WorkingEolInspectionError::Cancelled);
        }
        if deadline.remaining().is_zero() {
            return Err(WorkingEolInspectionError::TimedOut);
        }
        let count = file
            .read(&mut buffer)
            .map_err(|error| WorkingEolInspectionError::Incomplete(error.to_string()))?;
        if count == 0 {
            break;
        }
        bytes_read = bytes_read.saturating_add(count as u64);
        if bytes_read > MAX_WORKING_EOL_FILE_BYTES {
            return Err(WorkingEolInspectionError::Incomplete(format!(
                "working file exceeds {} bytes",
                MAX_WORKING_EOL_FILE_BYTES
            )));
        }
        if total_working_bytes.saturating_add(bytes_read) > MAX_WORKING_EOL_TOTAL_BYTES {
            return Err(WorkingEolInspectionError::Incomplete(format!(
                "working text total exceeds {} bytes",
                MAX_WORKING_EOL_TOTAL_BYTES
            )));
        }
        for byte in &buffer[..count] {
            if previous_cr {
                if *byte == b'\n' {
                    crlf = true;
                    previous_cr = false;
                    continue;
                }
                bare_cr = true;
                previous_cr = false;
            }
            match *byte {
                b'\r' => previous_cr = true,
                b'\n' => lf = true,
                _ => {}
            }
        }
    }
    if previous_cr {
        bare_cr = true;
    }
    let accounted_bytes = metadata_len.max(bytes_read);
    *total_working_bytes = total_working_bytes.saturating_add(accounted_bytes);
    if *total_working_bytes > MAX_WORKING_EOL_TOTAL_BYTES {
        return Err(WorkingEolInspectionError::Incomplete(format!(
            "working text total exceeds {} bytes",
            MAX_WORKING_EOL_TOTAL_BYTES
        )));
    }
    let styles = usize::from(lf) + usize::from(crlf) + usize::from(bare_cr);
    Ok(Some(if styles > 1 {
        WorkingEol::Mixed
    } else if bare_cr {
        WorkingEol::BareCr
    } else {
        WorkingEol::Supported
    }))
}

fn open_repository_regular_file_nofollow(
    repository_root: &Path,
    repo_path: &Path,
) -> std::io::Result<File> {
    use std::path::Component;

    if !repository_root.is_absolute() || repo_path.is_absolute() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "repository root must be absolute and resource path relative",
        ));
    }
    let mut components = repo_path.components().peekable();
    let mut parent = open_absolute_directory_path_nofollow(repository_root)?;
    while let Some(component) = components.next() {
        let Component::Normal(name) = component else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "resource path contains a non-normal component",
            ));
        };
        if components.peek().is_some() {
            parent = open_directory_child_nofollow(&parent, name)?;
        } else {
            return open_regular_child_nofollow(&parent, name);
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::InvalidInput,
        "resource path is empty",
    ))
}

fn repository_regular_file_size(
    repository_root: &Path,
    repo_path: &Path,
) -> Result<Option<u64>, String> {
    let file = match open_repository_regular_file_nofollow(repository_root, repo_path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    let metadata = file.metadata().map_err(|error| error.to_string())?;
    if !metadata.is_file() {
        return Err("binary working path is not a regular file".into());
    }
    Ok(Some(metadata.len()))
}

#[cfg(test)]
mod tests {
    use super::{
        classify_platform_xml_relative_path, inspect_working_eol, parse_attribute_records,
        parse_eol_records, resource_check_ids, RepositoryResourceKind,
        SourceResourcePolicyInspector, LFS_SINGLE_FILE_THRESHOLD_BYTES,
    };
    use crate::domain::cancellation::CancellationToken;
    use crate::domain::code_intelligence::ProviderDeadline;
    use crate::domain::project_health::{
        ProjectCheckId, ProjectCheckOutcome, ProjectHealthFact, ProjectHealthInspectionError,
    };
    use crate::domain::project_sources::{ProjectSourceSet, SourceFormat, SourceSetKind};
    use crate::domain::workspace::WorkspaceContext;
    use crate::infrastructure::internal_adapters::{ProcessCommand, ProcessOutput, ProcessRunner};
    use crate::infrastructure::platform::testing::{
        create_directory_link_fixture_for_test, create_file_link_fixture_for_test,
        FileLinkFixtureOutcome,
    };
    use crate::infrastructure::project_health::git::{GitIndexEntry, GitRepositoryInspector};
    use crate::infrastructure::project_health::layout::{
        InspectedSourceRoot, SourceLayoutInspector,
    };
    use std::cell::{Cell, RefCell};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};
    use tempfile::TempDir;

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
    fn resource_owner_name_expansion_is_bounded_before_policy_facts() {
        let temp = TempDir::new().unwrap();
        let root = temp.path().to_path_buf();
        fs::create_dir_all(root.join("src")).unwrap();
        let roots = vec![InspectedSourceRoot {
            source_set: ProjectSourceSet {
                name: "owner-".to_string() + &"x".repeat(40_000),
                kind: SourceSetKind::Configuration,
                path: "src".into(),
                source_format: SourceFormat::PlatformXml,
                format_evidence: Vec::new(),
                format_probe_error: None,
            },
            path: root.join("src"),
        }];
        let entries = (0..256)
            .map(|index| GitIndexEntry {
                repo_path: format!("src/Module{index}.bsl"),
                blob_oid: Some(format!("{index:040x}")),
                mode: Some("100644".into()),
            })
            .collect::<Vec<_>>();

        let error =
            SourceResourcePolicyInspector::classify(&root, &roots, &entries, &Default::default())
                .unwrap_err();

        assert!(error.repo_path.contains("expansion budget"), "{error:?}");
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
        )
        .unwrap();

        let inspection = SourceResourcePolicyInspector::with_process_runner(&FailingRunner)
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
        )
        .unwrap();

        let inspection = SourceResourcePolicyInspector::with_process_runner(&FailingRunner)
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
        let eol =
            parse_eol_records("i/lf w/crlf attr/text eol=crlf\tsrc/A.xml\0", &expected).unwrap();
        assert_eq!(eol["src/A.xml"].index, "lf");
        assert_eq!(eol["src/A.xml"].worktree, "crlf");
        assert!(parse_eol_records("i/lf w/lf without-tab\0", &expected).is_err());
        assert!(parse_eol_records("i/unknown w/lf attr/text\tsrc/A.xml\0", &expected).is_err());
    }

    #[test]
    fn repository_policy_parsers_honor_cooperative_checkpoint() {
        let attributes =
            "src/A.xml\0text\0set\0src/A.xml\0eol\0lf\0src/A.xml\0filter\0unspecified\0";
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
        let attributes =
            "src/A.xml\0text\0set\0src/A.xml\0eol\0lf\0src/A.xml\0filter\0unspecified\0";
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
                worktree_path: PathBuf::from("src/A.xml"),
                kind: RepositoryResourceKind::Text,
                blob_oid: "a".repeat(40),
            },
            super::RepositoryResource {
                source_set: "main".into(),
                repo_path: "src/B.xml".into(),
                worktree_path: PathBuf::from("src/B.xml"),
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

        let result = super::index_eol_facts_with_checkpoint(
            &resources,
            &eol,
            &Default::default(),
            &mut || {
                checkpoints += 1;
                if checkpoints == 2 {
                    Err(super::ResourceProtocolParseError::TimedOut)
                } else {
                    Ok(())
                }
            },
        );

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
            worktree_path: PathBuf::from("src/Picture.bin"),
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
            worktree_path: PathBuf::from("src/A.xml"),
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
        let binaries = std::collections::BTreeMap::from([(
            "main".to_string(),
            vec![("src/Picture.bin".to_string(), 12 * 1024 * 1024)],
        )]);

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
        )
        .unwrap();
        let entries = vec![GitIndexEntry {
            repo_path: "src/A.xml".into(),
            blob_oid: Some("a".repeat(40)),
            mode: Some("100644".into()),
        }];

        let inspection = SourceResourcePolicyInspector::with_process_runner(&FailingRunner)
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
                            crate::domain::project_health::ProjectCheckOutcome::NotRun { .. }
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
        )
        .unwrap();

        let inspection = SourceResourcePolicyInspector::with_process_runner(&FailingRunner)
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
                crate::domain::project_health::ProjectCheckOutcome::NotApplicable { .. }
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
        )
        .unwrap();
        let entries = vec![GitIndexEntry {
            repo_path: "src/A.xml".into(),
            blob_oid: Some("a".repeat(40)),
            mode: Some("100644".into()),
        }];
        let runner = AttributeProbeRunner::default();

        SourceResourcePolicyInspector::with_process_runner(&runner)
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
        )
        .unwrap();
        let runner = LateCancellingAttributeRunner {
            inner: AttributeProbeRunner::default(),
            cancellation: cancellation.clone(),
        };

        let result = SourceResourcePolicyInspector::with_process_runner(&runner).inspect(
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
        )
        .unwrap();
        let runner = LateCancellingEolIndexRunner {
            inner: AttributeProbeRunner::default(),
            cancellation: cancellation.clone(),
            read_tree_calls: Cell::new(0),
        };

        let result = SourceResourcePolicyInspector::with_process_runner(&runner).inspect(
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
        )
        .unwrap();
        let entries = vec![GitIndexEntry {
            repo_path: "src/A.xml".into(),
            blob_oid: Some("a".repeat(40)),
            mode: Some("100644".into()),
        }];
        let runner = AttributeProbeRunner::default();

        SourceResourcePolicyInspector::with_process_runner(&runner)
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
            | FileLinkFixtureOutcome::WindowsPrivilegeUnavailable => return,
        }
        let physical_root = fs::canonicalize(temp.path()).unwrap();

        let error = inspect_working_eol(
            &physical_root,
            Path::new("link.xml"),
            &mut 0,
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

    static DEADLINE_CLOCK_TICKS: AtomicUsize = AtomicUsize::new(0);

    fn advancing_deadline_clock() -> Instant {
        let tick = DEADLINE_CLOCK_TICKS.fetch_add(1, Ordering::SeqCst) as u64;
        Instant::now() + Duration::from_millis(tick)
    }

    #[test]
    fn project_health_worktree_eol_checks_deadline_between_chunks() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("large.xml");
        fs::write(&path, vec![b'x'; 128 * 1024]).unwrap();
        let physical_root = fs::canonicalize(temp.path()).unwrap();
        DEADLINE_CLOCK_TICKS.store(0, Ordering::SeqCst);
        let deadline = ProviderDeadline::with_clock(
            advancing_deadline_clock() + Duration::from_millis(2),
            advancing_deadline_clock,
        );

        let error = inspect_working_eol(
            &physical_root,
            Path::new("large.xml"),
            &mut 0,
            &CancellationToken::new(),
            deadline,
        )
        .unwrap_err();
        let error = error.to_string();

        assert!(error.contains("deadline"), "{error}");
    }

    #[test]
    fn project_health_worktree_eol_counts_bytes_read_after_metadata_snapshot() {
        let bytes = vec![b'x'; super::MAX_WORKING_EOL_FILE_BYTES as usize + 1];
        let error = super::inspect_working_eol_reader(
            std::io::Cursor::new(bytes),
            0,
            &mut 0,
            &CancellationToken::new(),
            ProviderDeadline::from_budget(Duration::from_secs(2)),
        )
        .unwrap_err();

        assert!(error.to_string().contains("working file exceeds"));
    }

    #[test]
    fn project_health_worktree_eol_stops_at_aggregate_byte_budget() {
        let mut total = super::MAX_WORKING_EOL_TOTAL_BYTES - 1;
        let error = super::inspect_working_eol_reader(
            std::io::Cursor::new(b"xx"),
            2,
            &mut total,
            &CancellationToken::new(),
            ProviderDeadline::from_budget(Duration::from_secs(2)),
        )
        .unwrap_err();

        assert!(error.to_string().contains("working text total exceeds"));
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
            | FileLinkFixtureOutcome::WindowsPrivilegeUnavailable => return,
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
                    crate::domain::project_health::ProjectCheckOutcome::NotRun { .. }
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
            | FileLinkFixtureOutcome::WindowsPrivilegeUnavailable => return,
        }

        let inspection = inspect_policy(&fixture);

        assert!(inspection.facts.iter().any(|fact| matches!(
            fact,
            ProjectHealthFact::GitInspectionIncomplete {
                check: ProjectCheckId::RepositoryLfs,
                ..
            }
        )));
        assert!(!inspection.facts.iter().any(|fact| matches!(
            fact,
            ProjectHealthFact::LfsConsider { largest_path, .. }
                if largest_path == "src/Binaries/Picture.bin"
        )));
    }

    #[test]
    fn project_health_repository_policy_lfs_is_advisory_for_exact_large_binary() {
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
        )
        .unwrap();
        let started = Instant::now();
        RESOURCE_POLICY_NOW.with(|now| *now.borrow_mut() = Some(started));
        RESOURCE_POLICY_EXPIRED.with(|expired| expired.set(false));
        RESOURCE_POLICY_CHECKS_BEFORE_EXPIRY.with(|remaining| remaining.set(8));
        let runner = ResourceLoopCheckpointRunner {
            cancellation: None,
            attribute_calls: Cell::new(0),
        };
        let binary_entry = GitIndexEntry {
            repo_path: "src/Picture.bin".into(),
            blob_oid: Some("a".repeat(40)),
            mode: Some("100644".into()),
        };

        let inspection = SourceResourcePolicyInspector::with_process_runner(&runner)
            .inspect(
                &fixture.root,
                &layout.roots,
                &[binary_entry],
                &cancellation,
                ProviderDeadline::with_clock(
                    started + Duration::from_millis(10),
                    resource_policy_now,
                ),
            )
            .unwrap();

        assert!(inspection.observations.iter().any(|observation| {
            observation.id == ProjectCheckId::RepositoryLfs
                && matches!(
                    observation.outcome,
                    crate::domain::project_health::ProjectCheckOutcome::NotRun { .. }
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
        )
        .unwrap();
        let runner = ResourceLoopCheckpointRunner {
            cancellation: Some(cancellation.clone()),
            attribute_calls: Cell::new(0),
        };

        let result = SourceResourcePolicyInspector::with_process_runner(&runner).inspect(
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
            Err(crate::domain::project_health::ProjectHealthInspectionError::Cancelled)
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

        let report = crate::domain::project_health::evaluate_project_health(
            crate::domain::project_health::ProjectHealthSnapshot {
                workspace_root: fixture.context.workspace_root.display().to_string(),
                cache_root: fixture.context.cache_root.display().to_string(),
                repository_root: git.repository_root.map(|root| root.display().to_string()),
                source_sets: layout.source_sets,
                source_targets_complete: layout.source_targets_complete,
                observations,
                facts,
            },
        )
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
        root: PathBuf,
        context: WorkspaceContext,
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
}
