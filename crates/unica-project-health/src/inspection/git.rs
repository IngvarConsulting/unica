use crate::inspection::layout::{InspectedSourceRoot, SourceLayoutInspection};
use crate::inspection::{
    SourceRootOwnerIndex, SourceRootOwnerIndexError, SourceRootOwnerLookupError,
};
use crate::{
    ProjectCheckId, ProjectCheckObservation, ProjectCheckOutcome, ProjectHealthFact,
    ProjectHealthFacts, ProjectHealthInspectionError,
};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use tempfile::TempDir;
#[cfg(test)]
use unica_process::runner::system_process_runner;
use unica_process::runner::{ProcessCommand, ProcessOutput, ProcessRunner};
use unica_process::StreamControl;
use unica_source_contract::portable_path::PortablePathExt;
use unica_source_contract::workspace::WorkspaceContext;
use unica_source_contract::{SourceFormat, SourceSetKind};
use unica_source_designer::probe::{config_dump_info_xml_kind, ConfigDumpInfoXmlKind};
use unica_work_control::cancellation::CancellationToken;
use unica_work_control::provider_deadline::ProviderDeadline;
use unica_workspace::paths::normalize_path_identity;
use unica_workspace::paths::{host_path_text, path_starts_with_host_root};
use unica_workspace::read::{open_read_directory_nofollow, FileIdentity};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GitIndexEntry {
    pub(crate) repo_path: String,
    pub(crate) blob_oid: Option<String>,
    pub(crate) mode: Option<String>,
}

const MAX_AMBIGUOUS_OWNER_REASON_BYTES: usize = 512;

#[derive(Debug, Clone, PartialEq, Eq)]
struct IgnoreMatch {
    source: String,
    line: String,
    pattern: String,
    path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum StagedBlobReadSafety {
    LocalOnlyGuaranteed,
    Blocked(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum WorkingIgnoreInspection {
    Complete(Vec<IgnoreMatch>),
    TimedOut,
    Incomplete(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MaterializedGitPath {
    path: String,
    role: &'static str,
}

#[derive(Debug, Default)]
struct MaterializedGitDirectories {
    logical_paths: BTreeSet<PathBuf>,
    identities: BTreeMap<FileIdentity, MaterializedGitPath>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum StagedIgnoreMaterializationError {
    Cancelled,
    Incomplete(String),
    PathIdentityCollision {
        first: MaterializedGitPath,
        second: MaterializedGitPath,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum IgnoreInspectionError {
    Malformed(String),
    Cancelled,
    TimedOut,
}

pub(crate) struct GitRepositoryInspection {
    pub(crate) repository_root: Option<PathBuf>,
    pub(crate) entries: Vec<GitIndexEntry>,
    pub(crate) resource_inspection_blocker: Option<String>,
    pub(crate) staged_config_dump_info_kinds: BTreeMap<String, ConfigDumpInfoXmlKind>,
    pub(crate) observations: Vec<ProjectCheckObservation>,
    pub(crate) facts: Vec<ProjectHealthFact>,
}

pub(crate) struct ConfigDumpInfoIndexInspection {
    pub(crate) runtime_paths: Vec<String>,
    pub(crate) inconclusive_paths: Vec<String>,
    pub(crate) classifications: BTreeMap<String, ConfigDumpInfoXmlKind>,
}

#[derive(Debug)]
struct ConfigDumpInfoEvaluation {
    facts: Vec<ProjectHealthFact>,
    classifications: BTreeMap<String, ConfigDumpInfoXmlKind>,
}

impl std::ops::Deref for ConfigDumpInfoEvaluation {
    type Target = [ProjectHealthFact];

    fn deref(&self) -> &Self::Target {
        &self.facts
    }
}

#[derive(Debug)]
pub(crate) enum ConfigDumpInfoIndexInspectionError {
    Cancelled,
    TimedOut,
    Incomplete(String),
}

pub(crate) struct GitRepositoryInspector<'a> {
    runner: &'a dyn ProcessRunner,
}

#[derive(Debug, Clone)]
struct IgnoreCandidate {
    source_set: Option<String>,
    repo_path: String,
}

struct SourceRootOwners<'a> {
    index: SourceRootOwnerIndex<'a>,
}

#[derive(Debug)]
enum GeneratedPathInspectionError {
    Cancelled,
    TimedOut,
    Incomplete(String),
}

impl<'a> SourceRootOwners<'a> {
    fn new(
        repository_root: &Path,
        roots: &'a [InspectedSourceRoot],
        cancellation: &CancellationToken,
        deadline: ProviderDeadline,
    ) -> Result<Self, GeneratedPathInspectionError> {
        let index = SourceRootOwnerIndex::new_with_checkpoint(repository_root, roots, &mut || {
            if cancellation.is_cancelled() {
                Err(GeneratedPathInspectionError::Cancelled)
            } else if deadline.is_elapsed() {
                Err(GeneratedPathInspectionError::TimedOut)
            } else {
                Ok(())
            }
        })
        .map_err(|error| match error {
            SourceRootOwnerIndexError::Checkpoint(error) => error,
            SourceRootOwnerIndexError::Path(reason) => {
                GeneratedPathInspectionError::Incomplete(reason)
            }
        })?;
        Ok(Self { index })
    }

    fn owners_for_repo_path(
        &self,
        path: &str,
        cancellation: &CancellationToken,
        deadline: ProviderDeadline,
    ) -> Result<Option<&[&'a InspectedSourceRoot]>, GeneratedPathInspectionError> {
        let owners = self
            .index
            .deepest_owners_with_checkpoint(path, &mut || {
                if cancellation.is_cancelled() {
                    return Err(GeneratedPathInspectionError::Cancelled);
                }
                if deadline.is_elapsed() {
                    return Err(GeneratedPathInspectionError::TimedOut);
                }
                Ok(())
            })
            .map_err(|error| match error {
                SourceRootOwnerLookupError::Checkpoint(error) => error,
                SourceRootOwnerLookupError::Path(reason) => {
                    GeneratedPathInspectionError::Incomplete(reason)
                }
            })?;
        Ok(owners.map(|(owners, _depth)| owners))
    }
}

impl GitRepositoryInspector<'static> {
    #[cfg(test)]
    pub(crate) fn new() -> Self {
        Self {
            runner: system_process_runner(),
        }
    }
}

impl<'a> GitRepositoryInspector<'a> {
    pub(crate) fn with_process_runner(runner: &'a dyn ProcessRunner) -> Self {
        Self { runner }
    }

    #[cfg(test)]
    pub(crate) fn with_runner(runner: &'a dyn ProcessRunner) -> Self {
        Self::with_process_runner(runner)
    }

    pub(crate) fn inspect_base(
        &self,
        context: &WorkspaceContext,
        layout: &SourceLayoutInspection,
        cancellation: &CancellationToken,
        deadline: ProviderDeadline,
    ) -> Result<GitRepositoryInspection, ProjectHealthInspectionError> {
        if cancellation.is_cancelled() {
            return Err(ProjectHealthInspectionError::Cancelled);
        }

        let discovery = match self.run(
            &context.workspace_root,
            ["rev-parse", "--show-toplevel", "--is-inside-work-tree"],
            cancellation,
            deadline,
        ) {
            Ok(output) => output,
            Err(_) if cancellation.is_cancelled() => {
                return Err(ProjectHealthInspectionError::Cancelled)
            }
            Err(error) if git_executable_is_unavailable(&error) => {
                return Ok(discovery_failed(
                    ProjectHealthFact::GitExecutableUnavailable { reason: error },
                    "Git executable is unavailable",
                    &layout.roots,
                ));
            }
            Err(error) => {
                return Ok(discovery_failed(
                    ProjectHealthFact::GitInspectionIncomplete {
                        check: ProjectCheckId::RepositoryDiscovery,
                        source_set: None,
                        reason: error,
                    },
                    "Git discovery failed",
                    &layout.roots,
                ));
            }
        };
        if discovery.cancelled || cancellation.is_cancelled() {
            return Err(ProjectHealthInspectionError::Cancelled);
        }
        if discovery.timed_out {
            return Ok(discovery_failed(
                ProjectHealthFact::GitInspectionTimeout {
                    check: ProjectCheckId::RepositoryDiscovery,
                    source_set: None,
                },
                "Git discovery timed out",
                &layout.roots,
            ));
        }
        if output_incomplete(&discovery) {
            return Ok(discovery_failed(
                ProjectHealthFact::GitInspectionIncomplete {
                    check: ProjectCheckId::RepositoryDiscovery,
                    source_set: None,
                    reason: completeness_reason(&discovery),
                },
                "Git discovery output is incomplete",
                &layout.roots,
            ));
        }
        if !discovery.status_success {
            if discovery
                .stderr
                .trim_start()
                .starts_with("fatal: not a git repository")
            {
                return Ok(discovery_failed(
                    ProjectHealthFact::GitRepositoryAbsent,
                    "Workspace is not inside a Git work tree",
                    &layout.roots,
                ));
            }
            return Ok(discovery_failed(
                ProjectHealthFact::GitInspectionIncomplete {
                    check: ProjectCheckId::RepositoryDiscovery,
                    source_set: None,
                    reason: nonzero_reason(&discovery),
                },
                "Git discovery failed",
                &layout.roots,
            ));
        }
        let (root_text, inside) = if let Some(root) = discovery.stdout.strip_suffix("\ntrue\n") {
            (root, "true")
        } else if let Some(root) = discovery.stdout.strip_suffix("\nfalse\n") {
            (root, "false")
        } else {
            ("", "")
        };
        if inside == "false" {
            return Ok(discovery_failed(
                ProjectHealthFact::GitRepositoryAbsent,
                "Workspace is not inside a Git work tree",
                &layout.roots,
            ));
        }
        if root_text.is_empty() || inside != "true" {
            return Ok(discovery_failed(
                ProjectHealthFact::GitInspectionIncomplete {
                    check: ProjectCheckId::RepositoryDiscovery,
                    source_set: None,
                    reason: "git rev-parse returned an unrecognized response".into(),
                },
                "Git discovery returned an unrecognized response",
                &layout.roots,
            ));
        }
        let repository_root = match normalize_path_identity(Path::new(root_text)) {
            Ok(root) => root,
            Err(reason) => {
                return Ok(discovery_failed(
                    ProjectHealthFact::GitInspectionIncomplete {
                        check: ProjectCheckId::RepositoryDiscovery,
                        source_set: None,
                        reason,
                    },
                    "Git repository root is invalid",
                    &layout.roots,
                ));
            }
        };
        let workspace_root = normalize_path_identity(&context.workspace_root)
            .map_err(ProjectHealthInspectionError::Fatal)?;
        if !path_starts_with_host_root(&workspace_root, &repository_root) {
            return Ok(discovery_failed(
                ProjectHealthFact::GitInspectionIncomplete {
                    check: ProjectCheckId::RepositoryDiscovery,
                    source_set: None,
                    reason: "Git repository root does not contain the workspace".into(),
                },
                "Git repository root does not contain the workspace",
                &layout.roots,
            ));
        }

        let mut observations = vec![completed(ProjectCheckId::RepositoryDiscovery)];
        let mut facts = Vec::new();
        let mut entries = Vec::new();
        let index = read_git_index(
            self.runner,
            &repository_root,
            cancellation,
            deadline,
            &mut |entry| entries.push(entry),
        );
        if let Err(error) = index {
            let (observation_reason, fact, after_index) = match error {
                IndexReadError::Cancelled => return Err(ProjectHealthInspectionError::Cancelled),
                IndexReadError::Failed(reason) => (
                    reason.clone(),
                    ProjectHealthFact::GitInspectionIncomplete {
                        check: ProjectCheckId::RepositoryIndex,
                        source_set: None,
                        reason,
                    },
                    "Git index inspection failed",
                ),
                IndexReadError::Incomplete(reason) => (
                    reason.clone(),
                    ProjectHealthFact::GitInspectionIncomplete {
                        check: ProjectCheckId::RepositoryIndex,
                        source_set: None,
                        reason,
                    },
                    "Git index output is incomplete",
                ),
                IndexReadError::ProcessTimedOut => (
                    "Git index inspection timed out".into(),
                    ProjectHealthFact::GitInspectionTimeout {
                        check: ProjectCheckId::RepositoryIndex,
                        source_set: None,
                    },
                    "Git index inspection timed out",
                ),
                IndexReadError::ParseTimedOut => (
                    "Git index parsing exceeded the inspection deadline".into(),
                    ProjectHealthFact::GitInspectionTimeout {
                        check: ProjectCheckId::RepositoryIndex,
                        source_set: None,
                    },
                    "Git index parsing exceeded the inspection deadline",
                ),
                IndexReadError::Malformed(reason) => (
                    reason.clone(),
                    ProjectHealthFact::GitInspectionIncomplete {
                        check: ProjectCheckId::RepositoryIndex,
                        source_set: None,
                        reason,
                    },
                    "Git index records are malformed",
                ),
            };
            observations.push(not_run(
                ProjectCheckId::RepositoryIndex,
                &observation_reason,
            ));
            facts.push(fact);
            append_not_run_after_index(&mut observations, &layout.roots, after_index);
            return Ok(GitRepositoryInspection {
                repository_root: Some(repository_root),
                entries: Vec::new(),
                resource_inspection_blocker: None,
                staged_config_dump_info_kinds: BTreeMap::new(),
                observations,
                facts,
            });
        }
        observations.push(completed(ProjectCheckId::RepositoryIndex));

        if !layout.repository_targets_complete && layout.roots.is_empty() {
            append_not_run_after_index(
                &mut observations,
                &layout.roots,
                "source-set targets are incomplete",
            );
            return Ok(GitRepositoryInspection {
                repository_root: Some(repository_root),
                entries,
                resource_inspection_blocker: None,
                staged_config_dump_info_kinds: BTreeMap::new(),
                observations,
                facts,
            });
        }

        let blob_read_safety = if staged_blob_read_needed(&entries)
            || layout
                .roots
                .iter()
                .any(|root| root.source_set.source_format == SourceFormat::PlatformXml)
            || (!layout.roots.is_empty() && !entries.is_empty())
        {
            inspect_staged_blob_read_safety(self.runner, &repository_root, cancellation, deadline)?
        } else {
            StagedBlobReadSafety::LocalOnlyGuaranteed
        };
        let resource_inspection_blocker = match &blob_read_safety {
            StagedBlobReadSafety::LocalOnlyGuaranteed => None,
            StagedBlobReadSafety::Blocked(reason) => Some(reason.clone()),
        };

        let source_owners =
            match SourceRootOwners::new(&repository_root, &layout.roots, cancellation, deadline) {
                Ok(owners) => owners,
                Err(GeneratedPathInspectionError::Cancelled) => {
                    return Err(ProjectHealthInspectionError::Cancelled)
                }
                Err(GeneratedPathInspectionError::TimedOut) => {
                    let reason = "source-root ownership index construction timed out";
                    observations.push(not_run(ProjectCheckId::RepositoryGeneratedPaths, reason));
                    append_not_run_for_roots(
                        &mut observations,
                        ProjectCheckId::RepositoryGeneratedPaths,
                        &layout.roots,
                        reason,
                    );
                    observations.push(not_run(ProjectCheckId::RepositoryConfigDumpInfo, reason));
                    append_not_run_for_roots(
                        &mut observations,
                        ProjectCheckId::RepositoryConfigDumpInfo,
                        &layout.roots,
                        reason,
                    );
                    for check in [
                        ProjectCheckId::RepositoryGeneratedPaths,
                        ProjectCheckId::RepositoryConfigDumpInfo,
                        ProjectCheckId::RepositoryIgnore,
                    ] {
                        facts.push(ProjectHealthFact::GitInspectionTimeout {
                            check,
                            source_set: None,
                        });
                    }
                    observations.push(not_run(ProjectCheckId::RepositoryIgnore, reason));
                    append_not_run_for_roots(
                        &mut observations,
                        ProjectCheckId::RepositoryIgnore,
                        &layout.roots,
                        reason,
                    );
                    return Ok(GitRepositoryInspection {
                        repository_root: Some(repository_root),
                        entries,
                        resource_inspection_blocker,
                        staged_config_dump_info_kinds: BTreeMap::new(),
                        observations,
                        facts,
                    });
                }
                Err(GeneratedPathInspectionError::Incomplete(reason)) => {
                    for check in [
                        ProjectCheckId::RepositoryGeneratedPaths,
                        ProjectCheckId::RepositoryConfigDumpInfo,
                        ProjectCheckId::RepositoryIgnore,
                    ] {
                        observations.push(not_run(check, &reason));
                        append_not_run_for_roots(&mut observations, check, &layout.roots, &reason);
                        facts.push(ProjectHealthFact::GitInspectionIncomplete {
                            check,
                            source_set: None,
                            reason: reason.clone(),
                        });
                    }
                    return Ok(GitRepositoryInspection {
                        repository_root: Some(repository_root),
                        entries,
                        resource_inspection_blocker,
                        staged_config_dump_info_kinds: BTreeMap::new(),
                        observations,
                        facts,
                    });
                }
            };
        match tracked_generated_facts(
            &entries,
            &repository_root,
            context,
            &source_owners,
            cancellation,
            deadline,
        ) {
            Ok(generated_facts) => {
                facts.extend(generated_facts);
                observations.push(completed(ProjectCheckId::RepositoryGeneratedPaths));
                append_completed_for_roots(
                    &mut observations,
                    ProjectCheckId::RepositoryGeneratedPaths,
                    &layout.roots,
                );
            }
            Err(GeneratedPathInspectionError::Cancelled) => {
                return Err(ProjectHealthInspectionError::Cancelled)
            }
            Err(GeneratedPathInspectionError::TimedOut) => {
                let reason = "tracked generated-path inspection timed out";
                observations.push(not_run(ProjectCheckId::RepositoryGeneratedPaths, reason));
                append_not_run_for_roots(
                    &mut observations,
                    ProjectCheckId::RepositoryGeneratedPaths,
                    &layout.roots,
                    reason,
                );
                facts.push(ProjectHealthFact::GitInspectionTimeout {
                    check: ProjectCheckId::RepositoryGeneratedPaths,
                    source_set: None,
                });
            }
            Err(GeneratedPathInspectionError::Incomplete(reason)) => {
                observations.push(not_run(ProjectCheckId::RepositoryGeneratedPaths, &reason));
                append_not_run_for_roots(
                    &mut observations,
                    ProjectCheckId::RepositoryGeneratedPaths,
                    &layout.roots,
                    &reason,
                );
                facts.push(ProjectHealthFact::GitInspectionIncomplete {
                    check: ProjectCheckId::RepositoryGeneratedPaths,
                    source_set: None,
                    reason,
                });
            }
        }

        let mut staged_config_dump_info_kinds = BTreeMap::new();
        match self.inspect_config_dump_info(
            &entries,
            &repository_root,
            &source_owners,
            &blob_read_safety,
            cancellation,
            deadline,
        )? {
            Ok(config_dump_evaluation) => {
                observations.push(completed(ProjectCheckId::RepositoryConfigDumpInfo));
                append_completed_for_roots(
                    &mut observations,
                    ProjectCheckId::RepositoryConfigDumpInfo,
                    &layout.roots,
                );
                staged_config_dump_info_kinds = config_dump_evaluation.classifications;
                facts.extend(config_dump_evaluation.facts);
            }
            Err(ConfigDumpInfoIndexInspectionError::TimedOut) => {
                let reason = "staged ConfigDumpInfo inspection timed out";
                observations.push(not_run(ProjectCheckId::RepositoryConfigDumpInfo, reason));
                append_not_run_for_roots(
                    &mut observations,
                    ProjectCheckId::RepositoryConfigDumpInfo,
                    &layout.roots,
                    reason,
                );
                facts.push(ProjectHealthFact::GitInspectionTimeout {
                    check: ProjectCheckId::RepositoryConfigDumpInfo,
                    source_set: None,
                });
            }
            Err(ConfigDumpInfoIndexInspectionError::Incomplete(reason)) => {
                observations.push(not_run(ProjectCheckId::RepositoryConfigDumpInfo, &reason));
                append_not_run_for_roots(
                    &mut observations,
                    ProjectCheckId::RepositoryConfigDumpInfo,
                    &layout.roots,
                    &reason,
                );
                facts.push(ProjectHealthFact::GitInspectionIncomplete {
                    check: ProjectCheckId::RepositoryConfigDumpInfo,
                    source_set: None,
                    reason,
                });
            }
            Err(ConfigDumpInfoIndexInspectionError::Cancelled) => unreachable!(
                "cancellation is converted to ProjectHealthInspectionError before this match"
            ),
        }

        let incomplete_ignore_roots = layout
            .roots
            .iter()
            .filter(|root| {
                matches!(
                    root.source_set.kind,
                    SourceSetKind::Configuration | SourceSetKind::Extension
                ) && matches!(
                    root.source_set.source_format,
                    SourceFormat::Unknown | SourceFormat::Invalid
                )
            })
            .collect::<Vec<_>>();

        let candidates = ignore_candidates(&repository_root, context, &layout.roots);
        let mut input = Vec::new();
        for candidate in &candidates {
            input.extend_from_slice(candidate.repo_path.as_bytes());
            input.push(0);
        }
        let staged_ignore_root = match materialize_staged_ignore_files(
            self.runner,
            &repository_root,
            &entries,
            &candidates,
            &blob_read_safety,
            cancellation,
            deadline,
        ) {
            Ok(root) => root,
            Err(StagedIgnoreMaterializationError::Cancelled) => {
                return Err(ProjectHealthInspectionError::Cancelled)
            }
            Err(StagedIgnoreMaterializationError::PathIdentityCollision { first, second }) => {
                let reason = format!(
                    "staged Git ignore paths collide through host path identity: {} ({}) and {} ({})",
                    first.path, first.role, second.path, second.role
                );
                observations.push(not_run(ProjectCheckId::RepositoryIgnore, &reason));
                append_not_run_for_roots(
                    &mut observations,
                    ProjectCheckId::RepositoryIgnore,
                    &layout.roots,
                    &reason,
                );
                facts.push(ProjectHealthFact::GitPathIdentityCollision {
                    check: ProjectCheckId::RepositoryIgnore,
                    source_set: None,
                    reason,
                    first_path: first.path,
                    first_role: first.role.into(),
                    second_path: second.path,
                    second_role: second.role.into(),
                });
                return Ok(GitRepositoryInspection {
                    repository_root: Some(repository_root),
                    entries,
                    resource_inspection_blocker,
                    staged_config_dump_info_kinds,
                    observations,
                    facts,
                });
            }
            Err(StagedIgnoreMaterializationError::Incomplete(reason)) => {
                observations.push(not_run(ProjectCheckId::RepositoryIgnore, &reason));
                append_not_run_for_roots(
                    &mut observations,
                    ProjectCheckId::RepositoryIgnore,
                    &layout.roots,
                    &reason,
                );
                facts.push(ProjectHealthFact::GitInspectionIncomplete {
                    check: ProjectCheckId::RepositoryIgnore,
                    source_set: None,
                    reason,
                });
                return Ok(GitRepositoryInspection {
                    repository_root: Some(repository_root),
                    entries,
                    resource_inspection_blocker,
                    staged_config_dump_info_kinds,
                    observations,
                    facts,
                });
            }
        };
        let work_tree_arg = format!(
            "--work-tree={}",
            staged_ignore_root.path().to_string_lossy()
        );
        let ignore_command = process_command_vec(
            &repository_root,
            vec![
                work_tree_arg,
                "check-ignore".into(),
                "-v".into(),
                "-z".into(),
                "--no-index".into(),
                "--stdin".into(),
            ],
            cancellation,
            deadline,
        );
        let ignore = match sticky_process_result(
            self.runner.run_with_input(&ignore_command, &input),
            cancellation,
        ) {
            Ok(output) => output,
            Err(_) if cancellation.is_cancelled() => {
                return Err(ProjectHealthInspectionError::Cancelled)
            }
            Err(reason) => {
                observations.push(not_run(ProjectCheckId::RepositoryIgnore, &reason));
                append_not_run_for_roots(
                    &mut observations,
                    ProjectCheckId::RepositoryIgnore,
                    &layout.roots,
                    &reason,
                );
                facts.push(ProjectHealthFact::GitInspectionIncomplete {
                    check: ProjectCheckId::RepositoryIgnore,
                    source_set: None,
                    reason,
                });
                return Ok(GitRepositoryInspection {
                    repository_root: Some(repository_root),
                    entries,
                    resource_inspection_blocker,
                    staged_config_dump_info_kinds,
                    observations,
                    facts,
                });
            }
        };
        if ignore.cancelled || cancellation.is_cancelled() {
            return Err(ProjectHealthInspectionError::Cancelled);
        }
        if ignore.timed_out {
            let reason = "Git ignore inspection timed out";
            observations.push(not_run(ProjectCheckId::RepositoryIgnore, reason));
            append_not_run_for_roots(
                &mut observations,
                ProjectCheckId::RepositoryIgnore,
                &layout.roots,
                reason,
            );
            facts.push(ProjectHealthFact::GitInspectionTimeout {
                check: ProjectCheckId::RepositoryIgnore,
                source_set: None,
            });
        } else if let Some(reason) = ignore_failure_reason(&ignore) {
            observations.push(not_run(ProjectCheckId::RepositoryIgnore, &reason));
            append_not_run_for_roots(
                &mut observations,
                ProjectCheckId::RepositoryIgnore,
                &layout.roots,
                &reason,
            );
            facts.push(ProjectHealthFact::GitInspectionIncomplete {
                check: ProjectCheckId::RepositoryIgnore,
                source_set: None,
                reason,
            });
        } else {
            match parse_check_ignore_verbose_z_controlled(&ignore.stdout, cancellation, deadline) {
                Ok(matches) => {
                    let working_matches = if matches.len() < candidates.len() {
                        inspect_working_ignore_matches(
                            self.runner,
                            &repository_root,
                            &input,
                            cancellation,
                            deadline,
                        )?
                    } else {
                        WorkingIgnoreInspection::Complete(Vec::new())
                    };
                    match working_matches {
                        WorkingIgnoreInspection::Complete(working_matches) => {
                            let ignore_fact_result = ignore_facts_with_checkpoint(
                                &candidates,
                                &matches,
                                &working_matches,
                                &entries,
                                staged_ignore_root.path(),
                                &mut || ignore_inspection_checkpoint(cancellation, deadline),
                            );
                            match ignore_fact_result {
                                Ok(mut ignore_facts) => {
                                    if !incomplete_ignore_roots.is_empty() {
                                        ignore_facts.retain(|fact| match fact {
                                            ProjectHealthFact::IgnoreRuleMissing {
                                                source_set: Some(source_set),
                                                ..
                                            }
                                            | ProjectHealthFact::IgnoreRuleLocalOnly {
                                                source_set: Some(source_set),
                                                ..
                                            } => !incomplete_ignore_roots
                                                .iter()
                                                .any(|root| root.source_set.name == *source_set),
                                            _ => true,
                                        });
                                    }
                                    observations.push(
                                        if incomplete_ignore_roots.is_empty()
                                            || !ignore_facts.is_empty()
                                        {
                                            completed(ProjectCheckId::RepositoryIgnore)
                                        } else {
                                            not_run(
                                                ProjectCheckId::RepositoryIgnore,
                                                "source format is incomplete, so format-dependent ignore targets are unknown",
                                            )
                                        },
                                    );
                                    append_completed_for_roots(
                                        &mut observations,
                                        ProjectCheckId::RepositoryIgnore,
                                        layout.roots.iter().filter(|root| {
                                            !incomplete_ignore_roots
                                                .iter()
                                                .any(|incomplete| std::ptr::eq(*incomplete, *root))
                                        }),
                                    );
                                    append_not_run_for_roots(
                                        &mut observations,
                                        ProjectCheckId::RepositoryIgnore,
                                        incomplete_ignore_roots.iter().copied(),
                                        "source format is incomplete, so format-dependent ignore targets are unknown",
                                    );
                                    facts.extend(ignore_facts);
                                }
                                Err(IgnoreInspectionError::Cancelled) => {
                                    return Err(ProjectHealthInspectionError::Cancelled)
                                }
                                Err(IgnoreInspectionError::TimedOut) => {
                                    let reason = "Git ignore fact composition timed out";
                                    observations
                                        .push(not_run(ProjectCheckId::RepositoryIgnore, reason));
                                    append_not_run_for_roots(
                                        &mut observations,
                                        ProjectCheckId::RepositoryIgnore,
                                        &layout.roots,
                                        reason,
                                    );
                                    facts.push(ProjectHealthFact::GitInspectionTimeout {
                                        check: ProjectCheckId::RepositoryIgnore,
                                        source_set: None,
                                    });
                                }
                                Err(IgnoreInspectionError::Malformed(reason)) => {
                                    observations
                                        .push(not_run(ProjectCheckId::RepositoryIgnore, &reason));
                                    append_not_run_for_roots(
                                        &mut observations,
                                        ProjectCheckId::RepositoryIgnore,
                                        &layout.roots,
                                        &reason,
                                    );
                                    facts.push(ProjectHealthFact::GitInspectionIncomplete {
                                        check: ProjectCheckId::RepositoryIgnore,
                                        source_set: None,
                                        reason,
                                    });
                                }
                            }
                        }
                        WorkingIgnoreInspection::TimedOut => {
                            let reason = "working-tree Git ignore provenance inspection timed out";
                            observations.push(not_run(ProjectCheckId::RepositoryIgnore, reason));
                            append_not_run_for_roots(
                                &mut observations,
                                ProjectCheckId::RepositoryIgnore,
                                &layout.roots,
                                reason,
                            );
                            facts.push(ProjectHealthFact::GitInspectionTimeout {
                                check: ProjectCheckId::RepositoryIgnore,
                                source_set: None,
                            });
                        }
                        WorkingIgnoreInspection::Incomplete(reason) => {
                            observations.push(not_run(ProjectCheckId::RepositoryIgnore, &reason));
                            append_not_run_for_roots(
                                &mut observations,
                                ProjectCheckId::RepositoryIgnore,
                                &layout.roots,
                                &reason,
                            );
                            facts.push(ProjectHealthFact::GitInspectionIncomplete {
                                check: ProjectCheckId::RepositoryIgnore,
                                source_set: None,
                                reason,
                            });
                        }
                    }
                }
                Err(IgnoreInspectionError::Cancelled) => {
                    return Err(ProjectHealthInspectionError::Cancelled)
                }
                Err(IgnoreInspectionError::TimedOut) => {
                    let reason = "Git ignore protocol parsing timed out";
                    observations.push(not_run(ProjectCheckId::RepositoryIgnore, reason));
                    append_not_run_for_roots(
                        &mut observations,
                        ProjectCheckId::RepositoryIgnore,
                        &layout.roots,
                        reason,
                    );
                    facts.push(ProjectHealthFact::GitInspectionTimeout {
                        check: ProjectCheckId::RepositoryIgnore,
                        source_set: None,
                    });
                }
                Err(IgnoreInspectionError::Malformed(reason)) => {
                    observations.push(not_run(ProjectCheckId::RepositoryIgnore, &reason));
                    append_not_run_for_roots(
                        &mut observations,
                        ProjectCheckId::RepositoryIgnore,
                        &layout.roots,
                        &reason,
                    );
                    facts.push(ProjectHealthFact::GitInspectionIncomplete {
                        check: ProjectCheckId::RepositoryIgnore,
                        source_set: None,
                        reason,
                    });
                }
            }
        }

        Ok(GitRepositoryInspection {
            repository_root: Some(repository_root),
            entries,
            resource_inspection_blocker,
            staged_config_dump_info_kinds,
            observations,
            facts,
        })
    }

    fn inspect_config_dump_info(
        &self,
        entries: &[GitIndexEntry],
        repository_root: &Path,
        source_owners: &SourceRootOwners<'_>,
        blob_read_safety: &StagedBlobReadSafety,
        cancellation: &CancellationToken,
        deadline: ProviderDeadline,
    ) -> Result<
        Result<ConfigDumpInfoEvaluation, ConfigDumpInfoIndexInspectionError>,
        ProjectHealthInspectionError,
    > {
        if entries.iter().any(is_config_dump_info_entry) {
            if let StagedBlobReadSafety::Blocked(reason) = blob_read_safety {
                return Ok(Err(ConfigDumpInfoIndexInspectionError::Incomplete(
                    reason.clone(),
                )));
            }
        }
        let inspection = classify_staged_config_dump_info(
            self.runner,
            repository_root,
            entries,
            cancellation,
            deadline,
        );
        let inspection = match inspection {
            Ok(inspection) => inspection,
            Err(ConfigDumpInfoIndexInspectionError::Cancelled) => {
                return Err(ProjectHealthInspectionError::Cancelled)
            }
            Err(error) => return Ok(Err(error)),
        };
        match config_dump_info_facts(inspection, source_owners, cancellation, deadline) {
            Err(ConfigDumpInfoIndexInspectionError::Cancelled) => {
                Err(ProjectHealthInspectionError::Cancelled)
            }
            evaluation => Ok(evaluation),
        }
    }

    fn run<const N: usize>(
        &self,
        cwd: &Path,
        args: [&str; N],
        cancellation: &CancellationToken,
        deadline: ProviderDeadline,
    ) -> Result<ProcessOutput, String> {
        if cancellation.is_cancelled() {
            return Err("cancelled: project health Git inspection".into());
        }
        if deadline.is_elapsed() {
            return Ok(timeout_output());
        }
        let result = self
            .runner
            .run(&process_command(cwd, args, cancellation, deadline));
        sticky_process_result(result, cancellation)
    }
}

fn config_dump_info_facts(
    inspection: ConfigDumpInfoIndexInspection,
    source_owners: &SourceRootOwners<'_>,
    cancellation: &CancellationToken,
    deadline: ProviderDeadline,
) -> Result<ConfigDumpInfoEvaluation, ConfigDumpInfoIndexInspectionError> {
    let mut facts = ProjectHealthFacts::default();
    let ConfigDumpInfoIndexInspection {
        runtime_paths,
        inconclusive_paths,
        classifications,
    } = inspection;
    for (path, classification) in &classifications {
        let descriptor_kind = match classification {
            ConfigDumpInfoXmlKind::ExternalProcessor => SourceSetKind::ExternalProcessor,
            ConfigDumpInfoXmlKind::ExternalReport => SourceSetKind::ExternalReport,
            ConfigDumpInfoXmlKind::RuntimeSidecar
            | ConfigDumpInfoXmlKind::MetadataDescriptor
            | ConfigDumpInfoXmlKind::Other => continue,
        };
        let owners = match source_owners.owners_for_repo_path(path, cancellation, deadline) {
            Ok(owners) => owners,
            Err(GeneratedPathInspectionError::Cancelled) => {
                return Err(ConfigDumpInfoIndexInspectionError::Cancelled)
            }
            Err(GeneratedPathInspectionError::TimedOut) => {
                return Err(ConfigDumpInfoIndexInspectionError::TimedOut)
            }
            Err(GeneratedPathInspectionError::Incomplete(reason)) => {
                return Err(ConfigDumpInfoIndexInspectionError::Incomplete(reason))
            }
        };
        let Some(owners) = owners else {
            continue;
        };
        for (owner_index, root) in owners.iter().enumerate() {
            if owner_index.is_multiple_of(256) {
                if cancellation.is_cancelled() {
                    return Err(ConfigDumpInfoIndexInspectionError::Cancelled);
                }
                if deadline.is_elapsed() {
                    return Err(ConfigDumpInfoIndexInspectionError::TimedOut);
                }
            }
            if !matches!(
                root.source_set.kind,
                SourceSetKind::ExternalProcessor | SourceSetKind::ExternalReport
            ) || root.source_set.kind == descriptor_kind
            {
                continue;
            }
            let reason = format!(
                "staged {} descriptor does not match declared {} source-set kind",
                source_set_kind_label(descriptor_kind),
                source_set_kind_label(root.source_set.kind)
            );
            facts
                .push(ProjectHealthFact::ConfigDumpInfoUnclassified {
                    source_set: Some(root.source_set.name.clone()),
                    path: path.clone(),
                    reason,
                })
                .map_err(ConfigDumpInfoIndexInspectionError::Incomplete)?;
        }
    }
    for path in runtime_paths {
        let owners = match source_owners.owners_for_repo_path(&path, cancellation, deadline) {
            Ok(owners) => owners,
            Err(GeneratedPathInspectionError::Cancelled) => {
                return Err(ConfigDumpInfoIndexInspectionError::Cancelled)
            }
            Err(GeneratedPathInspectionError::TimedOut) => {
                return Err(ConfigDumpInfoIndexInspectionError::TimedOut)
            }
            Err(GeneratedPathInspectionError::Incomplete(reason)) => {
                return Err(ConfigDumpInfoIndexInspectionError::Incomplete(reason))
            }
        };
        if let Some([root]) = owners {
            facts
                .push(ProjectHealthFact::RuntimeSidecarTracked {
                    source_set: root.source_set.name.clone(),
                    path,
                })
                .map_err(ConfigDumpInfoIndexInspectionError::Incomplete)?;
        } else {
            let ambiguous_owners = owners.filter(|owners| owners.len() > 1);
            let reason = ambiguous_owners
                .map(ambiguous_owner_reason)
                .unwrap_or_else(|| "runtime sidecar is outside a proven source root".into());
            facts
                .push(ProjectHealthFact::ConfigDumpInfoUnclassified {
                    source_set: None,
                    path: path.clone(),
                    reason: reason.clone(),
                })
                .map_err(ConfigDumpInfoIndexInspectionError::Incomplete)?;
            if let Some(owners) = ambiguous_owners {
                for (owner_index, root) in owners.iter().enumerate() {
                    if owner_index.is_multiple_of(256) {
                        if cancellation.is_cancelled() {
                            return Err(ConfigDumpInfoIndexInspectionError::Cancelled);
                        }
                        if deadline.is_elapsed() {
                            return Err(ConfigDumpInfoIndexInspectionError::TimedOut);
                        }
                    }
                    facts
                        .push(ProjectHealthFact::ConfigDumpInfoUnclassified {
                            source_set: Some(root.source_set.name.clone()),
                            path: path.clone(),
                            reason: reason.clone(),
                        })
                        .map_err(ConfigDumpInfoIndexInspectionError::Incomplete)?;
                }
            }
        }
    }
    for path in inconclusive_paths {
        let owners = match source_owners.owners_for_repo_path(&path, cancellation, deadline) {
            Ok(owners) => owners,
            Err(GeneratedPathInspectionError::Cancelled) => {
                return Err(ConfigDumpInfoIndexInspectionError::Cancelled)
            }
            Err(GeneratedPathInspectionError::TimedOut) => {
                return Err(ConfigDumpInfoIndexInspectionError::TimedOut)
            }
            Err(GeneratedPathInspectionError::Incomplete(reason)) => {
                return Err(ConfigDumpInfoIndexInspectionError::Incomplete(reason))
            }
        };
        let reason = "staged blob classification is inconclusive";
        match owners {
            Some([root]) => facts
                .push(ProjectHealthFact::ConfigDumpInfoUnclassified {
                    source_set: Some(root.source_set.name.clone()),
                    path,
                    reason: reason.into(),
                })
                .map_err(ConfigDumpInfoIndexInspectionError::Incomplete)?,
            Some(owners) => {
                facts
                    .push(ProjectHealthFact::ConfigDumpInfoUnclassified {
                        source_set: None,
                        path: path.clone(),
                        reason: reason.into(),
                    })
                    .map_err(ConfigDumpInfoIndexInspectionError::Incomplete)?;
                for (owner_index, root) in owners.iter().enumerate() {
                    if owner_index.is_multiple_of(256) {
                        if cancellation.is_cancelled() {
                            return Err(ConfigDumpInfoIndexInspectionError::Cancelled);
                        }
                        if deadline.is_elapsed() {
                            return Err(ConfigDumpInfoIndexInspectionError::TimedOut);
                        }
                    }
                    facts
                        .push(ProjectHealthFact::ConfigDumpInfoUnclassified {
                            source_set: Some(root.source_set.name.clone()),
                            path: path.clone(),
                            reason: reason.into(),
                        })
                        .map_err(ConfigDumpInfoIndexInspectionError::Incomplete)?;
                }
            }
            None => facts
                .push(ProjectHealthFact::ConfigDumpInfoUnclassified {
                    source_set: None,
                    path,
                    reason: reason.into(),
                })
                .map_err(ConfigDumpInfoIndexInspectionError::Incomplete)?,
        }
    }
    Ok(ConfigDumpInfoEvaluation {
        facts: facts.into_facts(),
        classifications,
    })
}

fn sticky_process_result(
    result: Result<ProcessOutput, String>,
    cancellation: &CancellationToken,
) -> Result<ProcessOutput, String> {
    if cancellation.is_cancelled() {
        Err("cancelled: project health Git inspection".into())
    } else {
        result
    }
}

fn staged_blob_read_needed(entries: &[GitIndexEntry]) -> bool {
    entries.iter().any(|entry| {
        is_config_dump_info_entry(entry)
            || Path::new(&entry.repo_path)
                .file_name()
                .and_then(|name| name.to_str())
                == Some(".gitignore")
    })
}

fn is_config_dump_info_entry(entry: &GitIndexEntry) -> bool {
    Path::new(&entry.repo_path)
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case("ConfigDumpInfo.xml"))
}

fn inspect_staged_blob_read_safety(
    runner: &dyn ProcessRunner,
    repository_root: &Path,
    cancellation: &CancellationToken,
    deadline: ProviderDeadline,
) -> Result<StagedBlobReadSafety, ProjectHealthInspectionError> {
    let partial_clone = process_command(
        repository_root,
        [
            "config",
            "-z",
            "--get-regexp",
            "^(extensions\\.[Pp]artial[Cc]lone|remote\\..*\\.[Pp]romisor)$",
        ],
        cancellation,
        deadline,
    );
    let partial_clone = match sticky_process_result(runner.run(&partial_clone), cancellation) {
        Ok(output) => output,
        Err(_) if cancellation.is_cancelled() => {
            return Err(ProjectHealthInspectionError::Cancelled)
        }
        Err(reason) => {
            return Ok(StagedBlobReadSafety::Blocked(format!(
                "cannot prove that staged blob reads are local-only: {reason}"
            )))
        }
    };
    if partial_clone.cancelled || cancellation.is_cancelled() {
        return Err(ProjectHealthInspectionError::Cancelled);
    }
    if partial_clone.timed_out || output_incomplete(&partial_clone) {
        let reason = if partial_clone.timed_out {
            "partial-clone configuration inspection timed out".into()
        } else {
            completeness_reason(&partial_clone)
        };
        return Ok(StagedBlobReadSafety::Blocked(format!(
            "cannot prove that staged blob reads are local-only: {reason}"
        )));
    }
    if !partial_clone.status_success {
        return if status_is_no_match(&partial_clone) {
            Ok(StagedBlobReadSafety::LocalOnlyGuaranteed)
        } else {
            Ok(StagedBlobReadSafety::Blocked(format!(
                "cannot prove that staged blob reads are local-only: {}",
                nonzero_reason(&partial_clone)
            )))
        };
    }
    if !partial_clone_config_is_enabled(&partial_clone.stdout) {
        return Ok(StagedBlobReadSafety::LocalOnlyGuaranteed);
    }

    let version = process_command(repository_root, ["version"], cancellation, deadline);
    let version = match sticky_process_result(runner.run(&version), cancellation) {
        Ok(output) => output,
        Err(_) if cancellation.is_cancelled() => {
            return Err(ProjectHealthInspectionError::Cancelled)
        }
        Err(reason) => {
            return Ok(StagedBlobReadSafety::Blocked(format!(
                "partial clone detected but Git version could not be inspected: {reason}"
            )))
        }
    };
    if version.cancelled || cancellation.is_cancelled() {
        return Err(ProjectHealthInspectionError::Cancelled);
    }
    if version.timed_out || output_incomplete(&version) || !version.status_success {
        let reason = if version.timed_out {
            "Git version inspection timed out".into()
        } else if output_incomplete(&version) {
            completeness_reason(&version)
        } else {
            nonzero_reason(&version)
        };
        return Ok(StagedBlobReadSafety::Blocked(format!(
            "partial clone detected but staged blob reads cannot be proven local-only: {reason}"
        )));
    }
    let version_text = version.stdout.trim();
    if git_version_has_no_lazy_fetch_guard(version_text) {
        Ok(StagedBlobReadSafety::LocalOnlyGuaranteed)
    } else {
        Ok(StagedBlobReadSafety::Blocked(format!(
            "Git {} in a partial clone cannot guarantee local-only staged blob reads; Git 2.46 or newer is required",
            version_text.strip_prefix("git version ").unwrap_or(version_text)
        )))
    }
}

fn partial_clone_config_is_enabled(config: &str) -> bool {
    if !config.is_empty() && !config.ends_with('\0') {
        return true;
    }
    config.split_terminator('\0').any(|record| {
        let Some((key, value)) = record.split_once('\n') else {
            return true;
        };
        if key.eq_ignore_ascii_case("extensions.partialclone") {
            return !value.trim().is_empty();
        }
        key.to_ascii_lowercase().ends_with(".promisor")
            && matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "true" | "yes" | "on" | "1"
            )
    })
}

fn git_version_has_no_lazy_fetch_guard(version: &str) -> bool {
    let Some(version) = version.strip_prefix("git version ") else {
        return false;
    };
    let mut components = version.split('.');
    let Some(major) = components
        .next()
        .and_then(|value| value.parse::<u64>().ok())
    else {
        return false;
    };
    let Some(minor) = components
        .next()
        .and_then(|value| value.parse::<u64>().ok())
    else {
        return false;
    };
    major > 2 || (major == 2 && minor >= 46)
}

pub(crate) fn classify_staged_config_dump_info(
    runner: &dyn ProcessRunner,
    repository_root: &Path,
    entries: &[GitIndexEntry],
    cancellation: &CancellationToken,
    deadline: ProviderDeadline,
) -> Result<ConfigDumpInfoIndexInspection, ConfigDumpInfoIndexInspectionError> {
    let mut runtime_paths = Vec::new();
    let mut inconclusive_paths = Vec::new();
    let mut classifications = BTreeMap::new();
    let mut blob_cache = BTreeMap::<String, Option<ConfigDumpInfoXmlKind>>::new();
    for entry in entries
        .iter()
        .filter(|entry| is_config_dump_info_entry(entry))
    {
        if cancellation.is_cancelled() {
            return Err(ConfigDumpInfoIndexInspectionError::Cancelled);
        }
        let Some(oid) = entry.blob_oid.as_ref() else {
            inconclusive_paths.push(entry.repo_path.clone());
            continue;
        };
        if deadline.is_elapsed() {
            return Err(ConfigDumpInfoIndexInspectionError::TimedOut);
        }
        let classification = if let Some(cached) = blob_cache.get(oid) {
            *cached
        } else {
            let command = process_command(
                repository_root,
                ["--no-replace-objects", "cat-file", "blob", oid.as_str()],
                cancellation,
                deadline,
            );
            let classified = match sticky_process_result(runner.run(&command), cancellation) {
                Ok(output) => {
                    if output.cancelled || cancellation.is_cancelled() {
                        return Err(ConfigDumpInfoIndexInspectionError::Cancelled);
                    }
                    if output.timed_out {
                        return Err(ConfigDumpInfoIndexInspectionError::TimedOut);
                    }
                    if output_incomplete(&output) {
                        return Err(ConfigDumpInfoIndexInspectionError::Incomplete(
                            completeness_reason(&output),
                        ));
                    }
                    if !output.status_success {
                        return Err(ConfigDumpInfoIndexInspectionError::Incomplete(
                            nonzero_reason(&output),
                        ));
                    }
                    Some(config_dump_info_xml_kind(output.stdout.as_bytes()))
                }
                Err(_) if cancellation.is_cancelled() => {
                    return Err(ConfigDumpInfoIndexInspectionError::Cancelled);
                }
                Err(reason) => return Err(ConfigDumpInfoIndexInspectionError::Incomplete(reason)),
            };
            blob_cache.insert(oid.clone(), classified);
            classified
        };
        if let Some(classification) = classification {
            classifications.insert(entry.repo_path.clone(), classification);
        }
        match classification {
            Some(ConfigDumpInfoXmlKind::RuntimeSidecar) => {
                runtime_paths.push(entry.repo_path.clone())
            }
            Some(
                ConfigDumpInfoXmlKind::ExternalProcessor
                | ConfigDumpInfoXmlKind::ExternalReport
                | ConfigDumpInfoXmlKind::MetadataDescriptor,
            ) => {}
            Some(ConfigDumpInfoXmlKind::Other) | None => {
                inconclusive_paths.push(entry.repo_path.clone())
            }
        }
    }
    runtime_paths.sort();
    runtime_paths.dedup();
    inconclusive_paths.sort();
    inconclusive_paths.dedup();
    Ok(ConfigDumpInfoIndexInspection {
        runtime_paths,
        inconclusive_paths,
        classifications,
    })
}

fn process_command<const N: usize>(
    cwd: &Path,
    args: [&str; N],
    cancellation: &CancellationToken,
    deadline: ProviderDeadline,
) -> ProcessCommand {
    process_command_vec(
        cwd,
        args.into_iter().map(str::to_owned).collect(),
        cancellation,
        deadline,
    )
}

fn process_command_vec(
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
        env_remove: git_environment_removals(),
        capture_limits: Some((
            super::PROJECT_HEALTH_STDOUT_CAPTURE_LIMIT,
            unica_process::STDERR_CAPTURE_LIMIT,
        )),
        timeout: deadline.remaining(),
        cancellation: cancellation.clone(),
    }
}

pub(super) fn git_environment_removals() -> Vec<OsString> {
    git_environment_removals_from(std::env::vars_os())
}

fn git_environment_removals_from(
    variables: impl IntoIterator<Item = (OsString, OsString)>,
) -> Vec<OsString> {
    let mut names = variables
        .into_iter()
        .map(|(name, _)| name)
        .filter(|name| {
            name.to_string_lossy()
                .to_ascii_uppercase()
                .starts_with("GIT_")
        })
        .collect::<Vec<_>>();
    names.sort();
    names.dedup();
    names
}

fn timeout_output() -> ProcessOutput {
    ProcessOutput {
        status_success: false,
        status: "timeout".into(),
        stdout: String::new(),
        stderr: String::new(),
        timed_out: true,
        cancelled: false,
        stdout_truncated: false,
        stderr_truncated: false,
        stdout_had_invalid_utf8: false,
        stderr_had_invalid_utf8: false,
    }
}

fn output_incomplete(output: &ProcessOutput) -> bool {
    output.stdout_truncated
        || output.stderr_truncated
        || output.stdout_had_invalid_utf8
        || output.stderr_had_invalid_utf8
}

/// Why a `check-ignore -v` answer cannot be trusted, if it cannot.
///
/// The command echoes the matching pattern, which keeps the encoding of its
/// ignore file (often cp1251). Only the pattern text can be non-UTF-8: the
/// source and the path are paths Unica wrote, and the pattern is reported and
/// only tested for its ASCII `!`, so its lossy text is enough. A truncated
/// stream is not.
/// Git skips a pattern file of 100 MiB or more with a warning; the answer
/// then comes from part of the policy.
fn ignore_failure_reason(output: &ProcessOutput) -> Option<String> {
    if output.stdout_truncated || output.stderr_truncated {
        return Some(completeness_reason(output));
    }
    if !output.status_success && !status_is_no_match(output) {
        return Some(nonzero_reason(output));
    }
    output
        .stderr
        .lines()
        .find(|line| line.starts_with("warning: ignoring excessively"))
        .map(|warning| format!("Git ignored part of the ignore policy: {warning}"))
}

fn completeness_reason(output: &ProcessOutput) -> String {
    let mut reasons = Vec::new();
    if output.stdout_truncated {
        reasons.push("stdout was truncated");
    }
    if output.stderr_truncated {
        reasons.push("stderr was truncated");
    }
    if output.stdout_had_invalid_utf8 {
        reasons.push("stdout contained invalid UTF-8");
    }
    if output.stderr_had_invalid_utf8 {
        reasons.push("stderr contained invalid UTF-8");
    }
    reasons.join("; ")
}

fn nonzero_reason(output: &ProcessOutput) -> String {
    stream_nonzero_reason(&output.status, &output.stderr)
}

fn status_is_no_match(output: &ProcessOutput) -> bool {
    output.stdout.is_empty()
        && output.stderr.is_empty()
        && matches!(output.status.as_str(), "exit status: 1" | "exit code: 1")
}

fn git_executable_is_unavailable(error: &str) -> bool {
    error.starts_with("process_failed:")
        && (error.contains("No such file or directory")
            || error.contains("os error 2")
            || error.contains("program not found"))
}

fn completed(id: ProjectCheckId) -> ProjectCheckObservation {
    ProjectCheckObservation {
        id,
        scope: id.scope(),
        source_set: None,
        outcome: ProjectCheckOutcome::Completed,
    }
}

fn append_completed_for_roots<'a>(
    observations: &mut Vec<ProjectCheckObservation>,
    id: ProjectCheckId,
    roots: impl IntoIterator<Item = &'a InspectedSourceRoot>,
) {
    let mut source_sets = roots
        .into_iter()
        .map(|root| root.source_set.name.clone())
        .collect::<Vec<_>>();
    source_sets.sort();
    source_sets.dedup();
    observations.extend(
        source_sets
            .into_iter()
            .map(|source_set| ProjectCheckObservation {
                id,
                scope: id.scope(),
                source_set: Some(source_set),
                outcome: ProjectCheckOutcome::Completed,
            }),
    );
}

fn append_not_run_for_roots<'a>(
    observations: &mut Vec<ProjectCheckObservation>,
    id: ProjectCheckId,
    roots: impl IntoIterator<Item = &'a InspectedSourceRoot>,
    reason: &str,
) {
    let mut source_sets = roots
        .into_iter()
        .map(|root| root.source_set.name.clone())
        .collect::<Vec<_>>();
    source_sets.sort();
    source_sets.dedup();
    observations.extend(
        source_sets
            .into_iter()
            .map(|source_set| ProjectCheckObservation {
                id,
                scope: id.scope(),
                source_set: Some(source_set),
                outcome: ProjectCheckOutcome::NotRun {
                    reason: reason.to_owned(),
                },
            }),
    );
}

fn not_run(id: ProjectCheckId, reason: &str) -> ProjectCheckObservation {
    ProjectCheckObservation {
        id,
        scope: id.scope(),
        source_set: None,
        outcome: ProjectCheckOutcome::NotRun {
            reason: reason.into(),
        },
    }
}

fn discovery_failed(
    fact: ProjectHealthFact,
    reason: &str,
    roots: &[InspectedSourceRoot],
) -> GitRepositoryInspection {
    let discovery = if matches!(
        fact,
        ProjectHealthFact::GitInspectionTimeout { .. }
            | ProjectHealthFact::GitInspectionIncomplete { .. }
    ) {
        not_run(ProjectCheckId::RepositoryDiscovery, reason)
    } else {
        completed(ProjectCheckId::RepositoryDiscovery)
    };
    let mut observations = vec![discovery];
    for id in [
        ProjectCheckId::RepositoryIndex,
        ProjectCheckId::RepositoryIgnore,
        ProjectCheckId::RepositoryGeneratedPaths,
        ProjectCheckId::RepositoryConfigDumpInfo,
    ] {
        observations.push(not_run(id, reason));
        if id != ProjectCheckId::RepositoryIndex {
            append_not_run_for_roots(&mut observations, id, roots, reason);
        }
    }
    GitRepositoryInspection {
        repository_root: None,
        entries: Vec::new(),
        resource_inspection_blocker: None,
        staged_config_dump_info_kinds: BTreeMap::new(),
        observations,
        facts: vec![fact],
    }
}

fn append_not_run_after_index(
    observations: &mut Vec<ProjectCheckObservation>,
    roots: &[InspectedSourceRoot],
    reason: &str,
) {
    for id in [
        ProjectCheckId::RepositoryIgnore,
        ProjectCheckId::RepositoryGeneratedPaths,
        ProjectCheckId::RepositoryConfigDumpInfo,
    ] {
        observations.push(not_run(id, reason));
        append_not_run_for_roots(observations, id, roots, reason);
    }
}

#[cfg(test)]
pub(crate) fn parse_git_index_entries(stdout: &str) -> Result<Vec<GitIndexEntry>, String> {
    let Some(records) = stdout
        .strip_suffix('\0')
        .or(stdout.is_empty().then_some(""))
    else {
        return Err("Git index output is missing its terminal NUL".into());
    };
    let mut parser = GitIndexRecords::default();
    let mut entries = Vec::new();
    for record in records.split_terminator('\0') {
        entries.extend(parser.push(record.as_bytes())?);
    }
    entries.extend(parser.finish());
    Ok(entries)
}

/// Why the staged index could not be read completely.
pub(crate) enum IndexReadError {
    Cancelled,
    /// Git could not be run.
    Failed(String),
    /// Git exited unsuccessfully or its output stream is incomplete.
    Incomplete(String),
    /// The Git process exceeded the inspection deadline.
    ProcessTimedOut,
    /// Reading the records reached the inspection deadline.
    ParseTimedOut,
    Malformed(String),
}

/// Merges the stage records of one path from `git ls-files --stage -z` into
/// one entry. Git lists the index in byte order of paths, so the records of a
/// path are adjacent and only one pending path is held.
#[derive(Default)]
pub(crate) struct GitIndexRecords {
    pending: Option<PendingIndexPath>,
}

struct PendingIndexPath {
    repo_path: String,
    records: usize,
    blob: Option<(String, String)>,
}

impl GitIndexRecords {
    /// Accept one record; returns the previous path once it is complete.
    pub(crate) fn push(&mut self, record: &[u8]) -> Result<Option<GitIndexEntry>, String> {
        let record = std::str::from_utf8(record)
            .map_err(|_| "Git index output contained invalid UTF-8".to_string())?;
        let (metadata, path) = record
            .split_once('\t')
            .ok_or_else(|| "Git index record has no tab separator".to_string())?;
        if path.is_empty() {
            return Err("Git index record has an empty path".into());
        }
        let mut fields = metadata.split_whitespace();
        let (Some(mode), Some(oid), Some(stage), None) =
            (fields.next(), fields.next(), fields.next(), fields.next())
        else {
            return Err("Git index record has invalid metadata".into());
        };
        let usable = matches!(mode, "100644" | "100755")
            && stage == "0"
            && !oid.is_empty()
            && oid.bytes().all(|byte| byte.is_ascii_hexdigit())
            && oid.bytes().any(|byte| byte != b'0');
        if let Some(pending) = self.pending.as_mut() {
            match path.cmp(pending.repo_path.as_str()) {
                std::cmp::Ordering::Equal => {
                    pending.records += 1;
                    pending.blob = None;
                    return Ok(None);
                }
                std::cmp::Ordering::Less => {
                    return Err("Git index records are not in path order".into());
                }
                std::cmp::Ordering::Greater => {}
            }
        }
        let next = PendingIndexPath {
            repo_path: path.to_owned(),
            records: 1,
            blob: usable.then(|| (oid.to_owned(), mode.to_owned())),
        };
        Ok(self.pending.replace(next).map(PendingIndexPath::into_entry))
    }

    /// The last path, after the stream ended.
    pub(crate) fn finish(self) -> Option<GitIndexEntry> {
        self.pending.map(PendingIndexPath::into_entry)
    }
}

impl PendingIndexPath {
    fn into_entry(self) -> GitIndexEntry {
        let (blob_oid, mode) = match self.blob.filter(|_| self.records == 1) {
            Some((oid, mode)) => (Some(oid), Some(mode)),
            None => (None, None),
        };
        GitIndexEntry {
            repo_path: self.repo_path,
            blob_oid,
            mode,
        }
    }
}

/// SHA-256 over parsed index entries in order. Equal digests prove the same
/// staged index without keeping a second copy of its entries.
pub(crate) struct GitIndexDigest(sha2::Sha256);

impl Default for GitIndexDigest {
    fn default() -> Self {
        use sha2::Digest;
        let mut hasher = sha2::Sha256::new();
        hasher.update(b"unica-git-index-entries-v1\0");
        Self(hasher)
    }
}

impl GitIndexDigest {
    pub(crate) fn update(&mut self, entry: &GitIndexEntry) {
        use sha2::Digest;
        for field in [
            Some(entry.repo_path.as_str()),
            entry.blob_oid.as_deref(),
            entry.mode.as_deref(),
        ] {
            match field {
                Some(field) => {
                    self.0.update([1]);
                    self.0.update((field.len() as u64).to_be_bytes());
                    self.0.update(field.as_bytes());
                }
                None => self.0.update([0]),
            }
        }
    }

    pub(crate) fn finish(self) -> [u8; 32] {
        use sha2::Digest;
        self.0.finalize().into()
    }
}

/// Stream the staged index of `repository_root` to `on_entry`, one merged
/// entry per path in path order. Entries may already have been delivered when
/// an error is returned; a caller keeps them only after `Ok`.
pub(crate) fn read_git_index(
    runner: &dyn ProcessRunner,
    repository_root: &Path,
    cancellation: &CancellationToken,
    deadline: ProviderDeadline,
    on_entry: &mut dyn FnMut(GitIndexEntry),
) -> Result<(), IndexReadError> {
    if cancellation.is_cancelled() {
        return Err(IndexReadError::Cancelled);
    }
    if deadline.is_elapsed() {
        return Err(IndexReadError::ProcessTimedOut);
    }
    let command = process_command(
        repository_root,
        ["ls-files", "--cached", "--stage", "-z"],
        cancellation,
        deadline,
    );
    let mut parser = GitIndexRecords::default();
    let mut stopped = None;
    let mut records = 0_usize;
    let output = runner.run_streaming_records(&command, &[], 0, &mut |record| {
        if records.is_multiple_of(256) {
            if cancellation.is_cancelled() {
                stopped = Some(IndexReadError::Cancelled);
                return StreamControl::Stop;
            }
            if deadline.is_elapsed() {
                stopped = Some(IndexReadError::ParseTimedOut);
                return StreamControl::Stop;
            }
        }
        records += 1;
        match parser.push(record) {
            Ok(entry) => {
                entry.map(&mut *on_entry);
                StreamControl::Continue
            }
            Err(reason) => {
                stopped = Some(IndexReadError::Malformed(reason));
                StreamControl::Stop
            }
        }
    });
    if cancellation.is_cancelled() {
        return Err(IndexReadError::Cancelled);
    }
    let output = output.map_err(IndexReadError::Failed)?;
    if let Some(error) = stopped {
        return Err(error);
    }
    if output.cancelled {
        return Err(IndexReadError::Cancelled);
    }
    if output.timed_out {
        return Err(IndexReadError::ProcessTimedOut);
    }
    match output.line_error {
        Some((_, error)) if error == unica_process::STREAM_RECORD_UNTERMINATED_ERROR => {
            return Err(IndexReadError::Malformed(
                "Git index output is missing its terminal NUL".into(),
            ));
        }
        Some((_, error)) => return Err(IndexReadError::Incomplete(error)),
        None => {}
    }
    if !output.status_success {
        return Err(IndexReadError::Incomplete(stream_nonzero_reason(
            &output.status,
            &output.stderr,
        )));
    }
    if deadline.is_elapsed() {
        return Err(IndexReadError::ParseTimedOut);
    }
    if let Some(entry) = parser.finish() {
        on_entry(entry);
    }
    Ok(())
}

fn stream_nonzero_reason(status: &str, stderr: &str) -> String {
    if stderr.trim().is_empty() {
        format!("Git exited with {status}")
    } else {
        format!("Git exited with {status}: {}", stderr.trim())
    }
}

#[cfg(test)]
fn parse_check_ignore_verbose_z(stdout: &str) -> Result<Vec<IgnoreMatch>, String> {
    parse_check_ignore_verbose_z_with_checkpoint(stdout, &mut || Ok(())).map_err(
        |error| match error {
            IgnoreInspectionError::Malformed(reason) => reason,
            IgnoreInspectionError::Cancelled => "Git ignore parsing was cancelled".into(),
            IgnoreInspectionError::TimedOut => "Git ignore parsing timed out".into(),
        },
    )
}

fn parse_check_ignore_verbose_z_controlled(
    stdout: &str,
    cancellation: &CancellationToken,
    deadline: ProviderDeadline,
) -> Result<Vec<IgnoreMatch>, IgnoreInspectionError> {
    parse_check_ignore_verbose_z_with_checkpoint(stdout, &mut || {
        ignore_inspection_checkpoint(cancellation, deadline)
    })
}

fn parse_check_ignore_verbose_z_with_checkpoint(
    stdout: &str,
    checkpoint: &mut dyn FnMut() -> Result<(), IgnoreInspectionError>,
) -> Result<Vec<IgnoreMatch>, IgnoreInspectionError> {
    checkpoint()?;
    if !stdout.is_empty() && !stdout.ends_with('\0') {
        return Err(IgnoreInspectionError::Malformed(
            "Git check-ignore output is missing its terminal NUL".into(),
        ));
    }
    let mut fields = stdout.split_terminator('\0');
    let mut matches = Vec::new();
    let mut record_index = 0_usize;
    loop {
        if record_index.is_multiple_of(256) {
            checkpoint()?;
        }
        let Some(source) = fields.next() else { break };
        let line = fields.next().ok_or_else(|| {
            IgnoreInspectionError::Malformed(
                "Git check-ignore output is not a sequence of quadruples".into(),
            )
        })?;
        let pattern = fields.next().ok_or_else(|| {
            IgnoreInspectionError::Malformed(
                "Git check-ignore output is not a sequence of quadruples".into(),
            )
        })?;
        let path = fields.next().ok_or_else(|| {
            IgnoreInspectionError::Malformed(
                "Git check-ignore output is not a sequence of quadruples".into(),
            )
        })?;
        matches.push(IgnoreMatch {
            source: source.into(),
            line: line.into(),
            pattern: pattern.into(),
            path: path.into(),
        });
        record_index += 1;
    }
    checkpoint()?;
    Ok(matches)
}

fn ignore_candidates(
    repository_root: &Path,
    context: &WorkspaceContext,
    roots: &[InspectedSourceRoot],
) -> Vec<IgnoreCandidate> {
    let mut candidates = Vec::new();
    if let Some(path) = repo_path(repository_root, &context.cache_root) {
        candidates.push(IgnoreCandidate {
            source_set: None,
            repo_path: path,
        });
    }
    for root in roots {
        if let Some(path) = declared_source_repo_path(
            repository_root,
            context,
            root,
            Path::new(".build/.unica-health-probe"),
        ) {
            candidates.push(IgnoreCandidate {
                source_set: Some(root.source_set.name.clone()),
                repo_path: path,
            });
        }
        if root.source_set.source_format == SourceFormat::PlatformXml
            && matches!(
                root.source_set.kind,
                SourceSetKind::Configuration | SourceSetKind::Extension
            )
        {
            for name in ["ConfigDumpInfo.xml", "DumpFilesIndex.txt"] {
                if let Some(path) =
                    declared_source_repo_path(repository_root, context, root, Path::new(name))
                {
                    candidates.push(IgnoreCandidate {
                        source_set: Some(root.source_set.name.clone()),
                        repo_path: path,
                    });
                }
            }
        }
    }
    candidates.sort_by(|left, right| left.repo_path.cmp(&right.repo_path));
    candidates.dedup_by(|left, right| {
        left.repo_path == right.repo_path && left.source_set == right.source_set
    });
    candidates
}

fn declared_source_repo_path(
    repository_root: &Path,
    context: &WorkspaceContext,
    root: &InspectedSourceRoot,
    child: &Path,
) -> Option<String> {
    let workspace_prefix = repo_path(repository_root, &context.workspace_root)?;
    let mut result = PathBuf::from(workspace_prefix);
    let workspace_depth = result.components().count();
    for component in Path::new(&root.source_set.path)
        .components()
        .chain(child.components())
    {
        match component {
            Component::Normal(component) => result.push(component),
            Component::CurDir => {}
            Component::ParentDir if result.components().count() > workspace_depth => {
                result.pop();
            }
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    Some(host_path_text(result.to_string_lossy().into_owned()))
}

fn ignore_facts_with_checkpoint(
    candidates: &[IgnoreCandidate],
    staged_matches: &[IgnoreMatch],
    working_matches: &[IgnoreMatch],
    entries: &[GitIndexEntry],
    staged_ignore_root: &Path,
    checkpoint: &mut dyn FnMut() -> Result<(), IgnoreInspectionError>,
) -> Result<Vec<ProjectHealthFact>, IgnoreInspectionError> {
    // `check-ignore -v` reports the deciding pattern, and a negated one
    // (`!pattern`) decides that the path is not ignored.
    let mut staged_by_path = BTreeMap::new();
    for (match_index, matched) in staged_matches.iter().enumerate() {
        if match_index.is_multiple_of(256) {
            checkpoint()?;
        }
        if !matched.pattern.starts_with('!') {
            staged_by_path.insert(matched.path.as_str(), matched);
        }
    }
    let mut working_by_path = BTreeMap::new();
    for (match_index, matched) in working_matches.iter().enumerate() {
        if match_index.is_multiple_of(256) {
            checkpoint()?;
        }
        if !matched.pattern.starts_with('!') {
            working_by_path.insert(matched.path.as_str(), matched);
        }
    }
    let mut tracked = BTreeSet::new();
    for (entry_index, entry) in entries.iter().enumerate() {
        if entry_index.is_multiple_of(256) {
            checkpoint()?;
        }
        tracked.insert(entry.repo_path.as_str());
    }
    let mut facts = Vec::new();
    for (candidate_index, candidate) in candidates.iter().enumerate() {
        if candidate_index.is_multiple_of(256) {
            checkpoint()?;
        }
        let Some(matched) = staged_by_path.get(candidate.repo_path.as_str()) else {
            if let Some(local) = working_by_path.get(candidate.repo_path.as_str()) {
                facts.push(ProjectHealthFact::IgnoreRuleLocalOnly {
                    source_set: candidate.source_set.clone(),
                    path: candidate.repo_path.clone(),
                    origin: format!("{}:{}:{}", local.source, local.line, local.pattern),
                });
                continue;
            }
            facts.push(ProjectHealthFact::IgnoreRuleMissing {
                source_set: candidate.source_set.clone(),
                path: candidate.repo_path.clone(),
            });
            continue;
        };
        let portable_source = portable_ignore_source(staged_ignore_root, &matched.source);
        if portable_source
            .as_deref()
            .is_none_or(|source| !tracked.contains(source))
        {
            facts.push(ProjectHealthFact::IgnoreRuleLocalOnly {
                source_set: candidate.source_set.clone(),
                path: candidate.repo_path.clone(),
                origin: format!("{}:{}:{}", matched.source, matched.line, matched.pattern),
            });
        }
    }
    checkpoint()?;
    Ok(facts)
}

fn ignore_inspection_checkpoint(
    cancellation: &CancellationToken,
    deadline: ProviderDeadline,
) -> Result<(), IgnoreInspectionError> {
    if cancellation.is_cancelled() {
        Err(IgnoreInspectionError::Cancelled)
    } else if deadline.is_elapsed() {
        Err(IgnoreInspectionError::TimedOut)
    } else {
        Ok(())
    }
}

fn materialize_staged_ignore_files(
    runner: &dyn ProcessRunner,
    repository_root: &Path,
    entries: &[GitIndexEntry],
    candidates: &[IgnoreCandidate],
    blob_read_safety: &StagedBlobReadSafety,
    cancellation: &CancellationToken,
    deadline: ProviderDeadline,
) -> Result<TempDir, StagedIgnoreMaterializationError> {
    let root = TempDir::new().map_err(|error| {
        StagedIgnoreMaterializationError::Incomplete(format!(
            "create staged ignore inspection directory: {error}"
        ))
    })?;
    let mut staged_ignore_files = Vec::new();
    for entry in entries {
        if Path::new(&entry.repo_path)
            .file_name()
            .and_then(|name| name.to_str())
            != Some(".gitignore")
        {
            continue;
        }
        let Some(oid) = entry.blob_oid.as_deref().filter(|_| {
            entry
                .mode
                .as_deref()
                .is_some_and(|mode| matches!(mode, "100644" | "100755"))
        }) else {
            return Err(StagedIgnoreMaterializationError::Incomplete(format!(
                "staged .gitignore path does not have one regular stage-0 blob: {}",
                entry.repo_path
            )));
        };
        staged_ignore_files.push((entry.repo_path.as_str(), oid));
    }
    if staged_ignore_files.is_empty() {
        return Ok(root);
    }
    if let StagedBlobReadSafety::Blocked(reason) = blob_read_safety {
        return Err(StagedIgnoreMaterializationError::Incomplete(reason.clone()));
    }
    let mut materialized_directories = MaterializedGitDirectories::default();
    for (path, oid) in &staged_ignore_files {
        let relative = Path::new(path);
        if relative.is_absolute()
            || relative
                .components()
                .any(|component| !matches!(component, Component::Normal(_)))
        {
            return Err(StagedIgnoreMaterializationError::Incomplete(format!(
                "staged .gitignore path is not a safe repository-relative path: {path}"
            )));
        }
        if !matches!(oid.len(), 40 | 64) || !oid.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(StagedIgnoreMaterializationError::Incomplete(format!(
                "staged .gitignore path does not have a valid Git blob object id: {path}"
            )));
        }
        let relative_parent = relative.parent().ok_or_else(|| {
            StagedIgnoreMaterializationError::Incomplete(format!(
                "staged .gitignore path has no parent: {path}"
            ))
        })?;
        let policy_path = MaterializedGitPath {
            path: (*path).into(),
            role: "staged ignore policy",
        };
        materialize_exact_git_directories(
            root.path(),
            relative_parent,
            &mut materialized_directories,
            &policy_path,
            cancellation,
            deadline,
        )?;
    }
    write_staged_ignore_blobs(
        runner,
        repository_root,
        &staged_ignore_files,
        root.path(),
        cancellation,
        deadline,
    )?;
    for candidate in candidates {
        let candidate = Path::new(&candidate.repo_path);
        let parent = candidate.parent().ok_or_else(|| {
            StagedIgnoreMaterializationError::Incomplete(format!(
                "Git ignore candidate has no parent: {}",
                candidate.display()
            ))
        })?;
        let candidate_path = MaterializedGitPath {
            path: candidate.to_string_lossy().into_owned(),
            role: "required ignore candidate",
        };
        materialize_exact_git_directories(
            root.path(),
            parent,
            &mut materialized_directories,
            &candidate_path,
            cancellation,
            deadline,
        )?;
    }
    Ok(root)
}

/// Write the staged blob of every `(path, oid)` in `files` to that path
/// below `target_root` as `git cat-file --batch` streams it, holding no more
/// than the current piece. Paths of one blob get a copy of the first.
fn write_staged_ignore_blobs(
    runner: &dyn ProcessRunner,
    repository_root: &Path,
    files: &[(&str, &str)],
    target_root: &Path,
    cancellation: &CancellationToken,
    deadline: ProviderDeadline,
) -> Result<(), StagedIgnoreMaterializationError> {
    let mut paths_by_oid = BTreeMap::<&str, Vec<&str>>::new();
    for (path, oid) in files {
        paths_by_oid.entry(oid).or_default().push(path);
    }
    let mut input = Vec::new();
    for oid in paths_by_oid.keys() {
        input.extend_from_slice(oid.as_bytes());
        input.push(b'\n');
    }
    let incomplete = StagedIgnoreMaterializationError::Incomplete;
    let mut sizes = Vec::with_capacity(paths_by_oid.len());
    let mut expected = paths_by_oid.keys();
    stream_staged_ignore_batch(
        runner,
        repository_root,
        "--batch-check=%(objectname) %(objecttype) %(objectsize)",
        &input,
        Some(b'\n'),
        cancellation,
        deadline,
        &mut |line| {
            let Some(oid) = expected.next() else {
                return Err(incomplete(
                    "Git returned unexpected staged .gitignore blob metadata".into(),
                ));
            };
            let line = std::str::from_utf8(line).map_err(|_| {
                incomplete(format!(
                    "Git returned invalid staged .gitignore blob metadata for {oid}"
                ))
            })?;
            let mut fields = line.split(' ');
            let (Some(actual_oid), Some("blob"), Some(raw_size), None) =
                (fields.next(), fields.next(), fields.next(), fields.next())
            else {
                return Err(incomplete(format!(
                    "Git returned invalid staged .gitignore blob metadata for {oid}"
                )));
            };
            if actual_oid != *oid {
                return Err(incomplete(format!(
                    "Git returned the wrong staged .gitignore blob object id for {oid}"
                )));
            }
            let size = raw_size.parse::<u64>().map_err(|_| {
                incomplete(format!(
                    "Git returned an invalid staged .gitignore blob size for {oid}"
                ))
            })?;
            sizes.push(size);
            Ok(())
        },
    )?;
    if sizes.len() != paths_by_oid.len() {
        return Err(incomplete(
            "Git returned incomplete staged .gitignore blob metadata".into(),
        ));
    }

    enum Batch {
        Header(Vec<u8>),
        Body(u64, std::fs::File),
        Trailer,
        Done,
    }
    let blobs = paths_by_oid.iter().zip(&sizes).collect::<Vec<_>>();
    let create = |path: &str| {
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(target_root.join(path))
            .map_err(|error| {
                incomplete(if error.kind() == std::io::ErrorKind::AlreadyExists {
                    format!(
                        "staged .gitignore path {path} collides with another host path identity"
                    )
                } else {
                    format!("create staged .gitignore blob {path}: {error}")
                })
            })
    };
    let mut next = 0_usize;
    let mut state = Batch::Header(Vec::new());
    stream_staged_ignore_batch(
        runner,
        repository_root,
        "--batch",
        &input,
        None,
        cancellation,
        deadline,
        &mut |mut chunk| {
            while !chunk.is_empty() {
                if matches!(state, Batch::Done) {
                    return Err(incomplete(
                        "Git returned unexpected trailing staged .gitignore blob bytes".into(),
                    ));
                }
                let ((oid, paths), size) = blobs[next];
                state = match std::mem::replace(&mut state, Batch::Done) {
                    Batch::Header(mut header) => {
                        let expected = format!("{oid} blob {size}\n");
                        let taken = chunk.len().min(expected.len() - header.len());
                        header.extend_from_slice(&chunk[..taken]);
                        chunk = &chunk[taken..];
                        if !expected.as_bytes().starts_with(&header) {
                            return Err(incomplete(format!(
                                "Git returned invalid staged .gitignore batch framing for {oid}"
                            )));
                        }
                        if header.len() < expected.len() {
                            Batch::Header(header)
                        } else if *size == 0 {
                            create(paths[0])?;
                            Batch::Trailer
                        } else {
                            Batch::Body(*size, create(paths[0])?)
                        }
                    }
                    Batch::Body(remaining, mut file) => {
                        let taken = chunk
                            .len()
                            .min(usize::try_from(remaining).unwrap_or(usize::MAX));
                        file.write_all(&chunk[..taken]).map_err(|error| {
                            incomplete(format!(
                                "write staged .gitignore blob {}: {error}",
                                paths[0]
                            ))
                        })?;
                        chunk = &chunk[taken..];
                        let remaining = remaining - taken as u64;
                        if remaining == 0 {
                            Batch::Trailer
                        } else {
                            Batch::Body(remaining, file)
                        }
                    }
                    Batch::Trailer => {
                        if chunk[0] != b'\n' {
                            return Err(incomplete(format!(
                                "Git returned invalid staged .gitignore batch framing for {oid}"
                            )));
                        }
                        chunk = &chunk[1..];
                        for path in &paths[1..] {
                            let mut copy = create(path)?;
                            let mut first = std::fs::File::open(target_root.join(paths[0]))
                                .map_err(|error| {
                                    incomplete(format!(
                                        "read staged .gitignore blob {}: {error}",
                                        paths[0]
                                    ))
                                })?;
                            std::io::copy(&mut first, &mut copy).map_err(|error| {
                                incomplete(format!("write staged .gitignore blob {path}: {error}"))
                            })?;
                        }
                        next += 1;
                        if next == blobs.len() {
                            Batch::Done
                        } else {
                            Batch::Header(Vec::new())
                        }
                    }
                    Batch::Done => unreachable!("a finished batch takes no more bytes"),
                };
            }
            Ok(())
        },
    )?;
    if !matches!(state, Batch::Done) {
        let oid = blobs.get(next).map_or("", |((oid, _), _)| oid);
        return Err(incomplete(format!(
            "Git returned incomplete staged .gitignore blob bytes for {oid}"
        )));
    }
    Ok(())
}

/// Stream one `git cat-file` batch over `input`: `delimiter`-terminated
/// records, or with no delimiter raw bytes in pieces.
#[allow(clippy::too_many_arguments)]
fn stream_staged_ignore_batch(
    runner: &dyn ProcessRunner,
    repository_root: &Path,
    mode: &str,
    input: &[u8],
    delimiter: Option<u8>,
    cancellation: &CancellationToken,
    deadline: ProviderDeadline,
    on_record: &mut dyn FnMut(&[u8]) -> Result<(), StagedIgnoreMaterializationError>,
) -> Result<(), StagedIgnoreMaterializationError> {
    if cancellation.is_cancelled() {
        return Err(StagedIgnoreMaterializationError::Cancelled);
    }
    if deadline.is_elapsed() {
        return Err(StagedIgnoreMaterializationError::Incomplete(
            "staged .gitignore blob inspection timed out".into(),
        ));
    }
    let command = process_command_vec(
        repository_root,
        vec![
            "--no-replace-objects".into(),
            "cat-file".into(),
            mode.into(),
        ],
        cancellation,
        deadline,
    );
    let mut failed = None;
    let mut consume = |record: &[u8]| {
        if cancellation.is_cancelled() {
            failed = Some(StagedIgnoreMaterializationError::Cancelled);
            return StreamControl::Stop;
        }
        match on_record(record) {
            Ok(()) => StreamControl::Continue,
            Err(error) => {
                failed = Some(error);
                StreamControl::Stop
            }
        }
    };
    let output = match delimiter {
        Some(delimiter) => runner.run_streaming_records(&command, input, delimiter, &mut consume),
        None => runner.run_streaming_bytes(&command, input, &mut consume),
    };
    if cancellation.is_cancelled() {
        return Err(StagedIgnoreMaterializationError::Cancelled);
    }
    let output = output.map_err(StagedIgnoreMaterializationError::Incomplete)?;
    if let Some(error) = failed {
        return Err(error);
    }
    if output.cancelled {
        return Err(StagedIgnoreMaterializationError::Cancelled);
    }
    if output.timed_out {
        return Err(StagedIgnoreMaterializationError::Incomplete(
            "staged .gitignore blob inspection timed out".into(),
        ));
    }
    if let Some((_, error)) = output.line_error {
        return Err(StagedIgnoreMaterializationError::Incomplete(format!(
            "Git returned incomplete staged .gitignore blob output: {error}"
        )));
    }
    if !output.status_success {
        return Err(StagedIgnoreMaterializationError::Incomplete(
            stream_nonzero_reason(&output.status, &output.stderr),
        ));
    }
    Ok(())
}

fn materialize_exact_git_directories(
    root: &Path,
    relative: &Path,
    materialized: &mut MaterializedGitDirectories,
    subject: &MaterializedGitPath,
    cancellation: &CancellationToken,
    deadline: ProviderDeadline,
) -> Result<(), StagedIgnoreMaterializationError> {
    let mut logical = PathBuf::new();
    let mut physical = root.to_path_buf();
    for component in relative.components() {
        if cancellation.is_cancelled() {
            return Err(StagedIgnoreMaterializationError::Cancelled);
        }
        if deadline.is_elapsed() {
            return Err(StagedIgnoreMaterializationError::Incomplete(
                "staged Git ignore path identity inspection timed out".into(),
            ));
        }
        let Component::Normal(component) = component else {
            return Err(StagedIgnoreMaterializationError::Incomplete(format!(
                "staged Git ignore path is not repository-relative: {}",
                relative.portable()
            )));
        };
        logical.push(component);
        physical.push(component);
        if materialized.logical_paths.contains(&logical) {
            continue;
        }
        match std::fs::create_dir(&physical) {
            Ok(()) => {
                let directory = open_read_directory_nofollow(&physical).map_err(|error| {
                    StagedIgnoreMaterializationError::Incomplete(format!(
                        "open staged Git ignore path {} after creation: {error}",
                        logical.portable()
                    ))
                })?;
                let identity = directory.identity().map_err(|error| {
                    StagedIgnoreMaterializationError::Incomplete(format!(
                        "identify staged Git ignore path {}: {error}",
                        logical.portable()
                    ))
                })?;
                if let Some(first) = materialized.identities.insert(identity, subject.clone()) {
                    return Err(StagedIgnoreMaterializationError::PathIdentityCollision {
                        first,
                        second: subject.clone(),
                    });
                }
                materialized.logical_paths.insert(logical.clone());
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let directory = open_read_directory_nofollow(&physical).map_err(|error| {
                    StagedIgnoreMaterializationError::Incomplete(format!(
                        "open colliding staged Git ignore path {}: {error}",
                        logical.portable()
                    ))
                })?;
                let identity = directory.identity().map_err(|error| {
                    StagedIgnoreMaterializationError::Incomplete(format!(
                        "identify colliding staged Git ignore path {}: {error}",
                        logical.portable()
                    ))
                })?;
                let Some(first) = materialized.identities.get(&identity).cloned() else {
                    return Err(StagedIgnoreMaterializationError::Incomplete(format!(
                        "staged Git ignore path {} unexpectedly resolves to an untracked host identity",
                        logical.portable()
                    )));
                };
                return Err(StagedIgnoreMaterializationError::PathIdentityCollision {
                    first,
                    second: subject.clone(),
                });
            }
            Err(error) => {
                return Err(StagedIgnoreMaterializationError::Incomplete(format!(
                    "create staged Git ignore path {}: {error}",
                    logical.portable()
                )));
            }
        }
    }
    Ok(())
}

fn inspect_working_ignore_matches(
    runner: &dyn ProcessRunner,
    repository_root: &Path,
    input: &[u8],
    cancellation: &CancellationToken,
    deadline: ProviderDeadline,
) -> Result<WorkingIgnoreInspection, ProjectHealthInspectionError> {
    let command = process_command(
        repository_root,
        ["check-ignore", "-v", "-z", "--no-index", "--stdin"],
        cancellation,
        deadline,
    );
    let output = match sticky_process_result(runner.run_with_input(&command, input), cancellation) {
        Ok(output) => output,
        Err(_) if cancellation.is_cancelled() => {
            return Err(ProjectHealthInspectionError::Cancelled)
        }
        Err(reason) => return Ok(WorkingIgnoreInspection::Incomplete(reason)),
    };
    if output.cancelled || cancellation.is_cancelled() {
        return Err(ProjectHealthInspectionError::Cancelled);
    }
    if output.timed_out {
        return Ok(WorkingIgnoreInspection::TimedOut);
    }
    if let Some(reason) = ignore_failure_reason(&output) {
        return Ok(WorkingIgnoreInspection::Incomplete(reason));
    }
    match parse_check_ignore_verbose_z_controlled(&output.stdout, cancellation, deadline) {
        Ok(matches) => Ok(WorkingIgnoreInspection::Complete(matches)),
        Err(IgnoreInspectionError::Cancelled) => Err(ProjectHealthInspectionError::Cancelled),
        Err(IgnoreInspectionError::TimedOut) => Ok(WorkingIgnoreInspection::TimedOut),
        Err(IgnoreInspectionError::Malformed(reason)) => {
            Ok(WorkingIgnoreInspection::Incomplete(reason))
        }
    }
}

fn portable_ignore_source(staged_ignore_root: &Path, source: &str) -> Option<String> {
    let source_path = Path::new(source);
    let repo_path = if source_path.is_absolute() {
        repo_path(staged_ignore_root, source_path)?
    } else {
        if source_path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
        {
            return None;
        }
        let materialized = staged_ignore_root.join(source_path);
        let metadata = std::fs::symlink_metadata(&materialized).ok()?;
        if !metadata.is_file()
            || unica_workspace::paths::metadata_is_link_or_reparse_point(&metadata)
        {
            return None;
        }
        source.to_owned()
    };
    (Path::new(&repo_path)
        .file_name()
        .and_then(|name| name.to_str())
        == Some(".gitignore")
        && !repo_path.starts_with(".git/"))
    .then_some(repo_path)
}

fn tracked_generated_facts(
    entries: &[GitIndexEntry],
    repository_root: &Path,
    context: &WorkspaceContext,
    source_owners: &SourceRootOwners<'_>,
    cancellation: &CancellationToken,
    deadline: ProviderDeadline,
) -> Result<Vec<ProjectHealthFact>, GeneratedPathInspectionError> {
    let cache = repo_path(repository_root, &context.cache_root);
    let mut facts = ProjectHealthFacts::default();
    for (entry_index, entry) in entries.iter().enumerate() {
        if entry_index % 256 == 0 {
            if cancellation.is_cancelled() {
                return Err(GeneratedPathInspectionError::Cancelled);
            }
            if deadline.is_elapsed() {
                return Err(GeneratedPathInspectionError::TimedOut);
            }
        }
        if Path::new(&entry.repo_path)
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.eq_ignore_ascii_case("ConfigDumpInfo.xml"))
        {
            continue;
        }
        let generated = Path::new(&entry.repo_path).components().any(|component| {
            component
                .as_os_str()
                .to_str()
                .is_some_and(|component| component.eq_ignore_ascii_case(".build"))
        }) || Path::new(&entry.repo_path)
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.eq_ignore_ascii_case("DumpFilesIndex.txt"))
            || cache.as_ref().is_some_and(|cache| {
                entry.repo_path == *cache
                    || entry
                        .repo_path
                        .strip_prefix(cache)
                        .is_some_and(|suffix| suffix.starts_with('/'))
            });
        if generated {
            match source_owners.owners_for_repo_path(&entry.repo_path, cancellation, deadline)? {
                Some(owners) if !owners.is_empty() => {
                    for (owner_index, root) in owners.iter().enumerate() {
                        if owner_index.is_multiple_of(256) {
                            if cancellation.is_cancelled() {
                                return Err(GeneratedPathInspectionError::Cancelled);
                            }
                            if deadline.is_elapsed() {
                                return Err(GeneratedPathInspectionError::TimedOut);
                            }
                        }
                        facts
                            .push(ProjectHealthFact::GeneratedPathTracked {
                                source_set: Some(root.source_set.name.clone()),
                                path: entry.repo_path.clone(),
                            })
                            .map_err(GeneratedPathInspectionError::Incomplete)?;
                    }
                }
                _ => facts
                    .push(ProjectHealthFact::GeneratedPathTracked {
                        source_set: None,
                        path: entry.repo_path.clone(),
                    })
                    .map_err(GeneratedPathInspectionError::Incomplete)?,
            }
        }
    }
    Ok(facts.into_facts())
}

fn ambiguous_owner_reason(owners: &[&InspectedSourceRoot]) -> String {
    const PREFIX: &str = "runtime sidecar has ambiguous equal-depth source-set owners";
    let mut names = owners
        .iter()
        .map(|root| root.source_set.name.as_str())
        .collect::<Vec<_>>();
    names.sort_unstable();
    names.dedup();
    let joined_bytes = names.iter().fold(0_usize, |total, name| {
        total.saturating_add(name.len()).saturating_add(2)
    });
    if PREFIX.len().saturating_add(2).saturating_add(joined_bytes)
        <= MAX_AMBIGUOUS_OWNER_REASON_BYTES
    {
        format!("{PREFIX}: {}", names.join(", "))
    } else {
        format!(
            "{PREFIX}: {} owners; names omitted because the bounded diagnostic limit was reached",
            names.len()
        )
    }
}

fn source_set_kind_label(kind: SourceSetKind) -> &'static str {
    match kind {
        SourceSetKind::Configuration => "configuration",
        SourceSetKind::Extension => "extension",
        SourceSetKind::ExternalProcessor => "external processor",
        SourceSetKind::ExternalReport => "external report",
    }
}

fn repo_path(repository_root: &Path, path: &Path) -> Option<String> {
    let path = normalize_path_identity(path).ok()?;
    let relative = path.strip_prefix(repository_root).ok()?;
    Some(host_path_text(relative.to_string_lossy().into_owned()))
}

#[cfg(test)]
#[path = "../../tests/platform/project_health_git.rs"]
mod tests;
