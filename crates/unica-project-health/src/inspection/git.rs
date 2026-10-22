use crate::inspection::layout::{InspectedSourceRoot, SourceLayoutInspection};
use crate::inspection::{
    SourceRootOwnerIndex, SourceRootOwnerIndexError, SourceRootOwnerLookupError,
};
use crate::{
    ProjectCheckId, ProjectCheckObservation, ProjectCheckOutcome, ProjectHealthFact,
    ProjectHealthInspectionError,
};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use tempfile::TempDir;
#[cfg(test)]
use unica_process::runner::system_process_runner;
use unica_process::runner::{ProcessCommand, ProcessOutput, ProcessRunner};
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

const MAX_STAGED_IGNORE_FILES: usize = 1024;
const MAX_STAGED_IGNORE_TOTAL_BYTES: usize = 8 * 1024 * 1024;
const MAX_STAGED_IGNORE_BATCH_METADATA_BYTES: usize = MAX_STAGED_IGNORE_FILES * 128;
const MAX_EXPANDED_REPOSITORY_FACTS: usize = 4096;
const MAX_EXPANDED_REPOSITORY_FACT_BYTES: usize = 8 * 1024 * 1024;
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
        let index = match self.run(
            &repository_root,
            ["ls-files", "--cached", "--stage", "-z"],
            cancellation,
            deadline,
        ) {
            Ok(output) => output,
            Err(_) if cancellation.is_cancelled() => {
                return Err(ProjectHealthInspectionError::Cancelled)
            }
            Err(reason) => {
                observations.push(not_run(ProjectCheckId::RepositoryIndex, &reason));
                facts.push(ProjectHealthFact::GitInspectionIncomplete {
                    check: ProjectCheckId::RepositoryIndex,
                    source_set: None,
                    reason,
                });
                append_not_run_after_index(
                    &mut observations,
                    &layout.roots,
                    "Git index inspection failed",
                );
                return Ok(GitRepositoryInspection {
                    repository_root: Some(repository_root),
                    entries: Vec::new(),
                    resource_inspection_blocker: None,
                    staged_config_dump_info_kinds: BTreeMap::new(),
                    observations,
                    facts,
                });
            }
        };
        if index.cancelled || cancellation.is_cancelled() {
            return Err(ProjectHealthInspectionError::Cancelled);
        }
        if index.timed_out {
            observations.push(not_run(
                ProjectCheckId::RepositoryIndex,
                "Git index inspection timed out",
            ));
            facts.push(ProjectHealthFact::GitInspectionTimeout {
                check: ProjectCheckId::RepositoryIndex,
                source_set: None,
            });
            append_not_run_after_index(
                &mut observations,
                &layout.roots,
                "Git index inspection timed out",
            );
            return Ok(GitRepositoryInspection {
                repository_root: Some(repository_root),
                entries: Vec::new(),
                resource_inspection_blocker: None,
                staged_config_dump_info_kinds: BTreeMap::new(),
                observations,
                facts,
            });
        }
        if !index.status_success || output_incomplete(&index) {
            let reason = if output_incomplete(&index) {
                completeness_reason(&index)
            } else {
                nonzero_reason(&index)
            };
            observations.push(not_run(ProjectCheckId::RepositoryIndex, &reason));
            facts.push(ProjectHealthFact::GitInspectionIncomplete {
                check: ProjectCheckId::RepositoryIndex,
                source_set: None,
                reason,
            });
            append_not_run_after_index(
                &mut observations,
                &layout.roots,
                "Git index output is incomplete",
            );
            return Ok(GitRepositoryInspection {
                repository_root: Some(repository_root),
                entries: Vec::new(),
                resource_inspection_blocker: None,
                staged_config_dump_info_kinds: BTreeMap::new(),
                observations,
                facts,
            });
        }
        let entries =
            match parse_git_index_entries_controlled(&index.stdout, cancellation, deadline) {
                Ok(entries) => entries,
                Err(IndexParseError::Cancelled) => {
                    return Err(ProjectHealthInspectionError::Cancelled)
                }
                Err(IndexParseError::TimedOut) => {
                    observations.push(not_run(
                        ProjectCheckId::RepositoryIndex,
                        "Git index parsing exceeded the inspection deadline",
                    ));
                    facts.push(ProjectHealthFact::GitInspectionTimeout {
                        check: ProjectCheckId::RepositoryIndex,
                        source_set: None,
                    });
                    append_not_run_after_index(
                        &mut observations,
                        &layout.roots,
                        "Git index parsing exceeded the inspection deadline",
                    );
                    return Ok(GitRepositoryInspection {
                        repository_root: Some(repository_root),
                        entries: Vec::new(),
                        resource_inspection_blocker: None,
                        staged_config_dump_info_kinds: BTreeMap::new(),
                        observations,
                        facts,
                    });
                }
                Err(IndexParseError::Malformed(reason)) => {
                    observations.push(not_run(ProjectCheckId::RepositoryIndex, &reason));
                    facts.push(ProjectHealthFact::GitInspectionIncomplete {
                        check: ProjectCheckId::RepositoryIndex,
                        source_set: None,
                        reason,
                    });
                    append_not_run_after_index(
                        &mut observations,
                        &layout.roots,
                        "Git index records are malformed",
                    );
                    return Ok(GitRepositoryInspection {
                        repository_root: Some(repository_root),
                        entries: Vec::new(),
                        resource_inspection_blocker: None,
                        staged_config_dump_info_kinds: BTreeMap::new(),
                        observations,
                        facts,
                    });
                }
            };
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
        } else if output_incomplete(&ignore)
            || (!ignore.status_success && !status_is_no_match(&ignore))
        {
            let reason = if output_incomplete(&ignore) {
                completeness_reason(&ignore)
            } else {
                nonzero_reason(&ignore)
            };
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
        let mut facts = Vec::new();
        let mut fact_bytes = 0_usize;
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
                    return Err(ProjectHealthInspectionError::Cancelled)
                }
                Err(GeneratedPathInspectionError::TimedOut) => {
                    return Ok(Err(ConfigDumpInfoIndexInspectionError::TimedOut))
                }
                Err(GeneratedPathInspectionError::Incomplete(reason)) => {
                    return Ok(Err(ConfigDumpInfoIndexInspectionError::Incomplete(reason)))
                }
            };
            let Some(owners) = owners else {
                continue;
            };
            for (owner_index, root) in owners.iter().enumerate() {
                if owner_index.is_multiple_of(256) {
                    if cancellation.is_cancelled() {
                        return Err(ProjectHealthInspectionError::Cancelled);
                    }
                    if deadline.is_elapsed() {
                        return Ok(Err(ConfigDumpInfoIndexInspectionError::TimedOut));
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
                fact_bytes = match reserve_expanded_repository_fact_budget(
                    facts.len(),
                    fact_bytes,
                    1,
                    path.len()
                        .saturating_add(root.source_set.name.len())
                        .saturating_add(reason.len()),
                    "ConfigDumpInfo",
                ) {
                    Ok(bytes) => bytes,
                    Err(reason) => {
                        return Ok(Err(ConfigDumpInfoIndexInspectionError::Incomplete(reason)))
                    }
                };
                facts.push(ProjectHealthFact::ConfigDumpInfoUnclassified {
                    source_set: Some(root.source_set.name.clone()),
                    path: path.clone(),
                    reason,
                });
            }
        }
        for path in runtime_paths {
            let owners = match source_owners.owners_for_repo_path(&path, cancellation, deadline) {
                Ok(owners) => owners,
                Err(GeneratedPathInspectionError::Cancelled) => {
                    return Err(ProjectHealthInspectionError::Cancelled)
                }
                Err(GeneratedPathInspectionError::TimedOut) => {
                    return Ok(Err(ConfigDumpInfoIndexInspectionError::TimedOut))
                }
                Err(GeneratedPathInspectionError::Incomplete(reason)) => {
                    return Ok(Err(ConfigDumpInfoIndexInspectionError::Incomplete(reason)))
                }
            };
            if let Some([root]) = owners {
                fact_bytes = match reserve_expanded_repository_fact_budget(
                    facts.len(),
                    fact_bytes,
                    1,
                    path.len().saturating_add(root.source_set.name.len()),
                    "ConfigDumpInfo",
                ) {
                    Ok(bytes) => bytes,
                    Err(reason) => {
                        return Ok(Err(ConfigDumpInfoIndexInspectionError::Incomplete(reason)))
                    }
                };
                facts.push(ProjectHealthFact::RuntimeSidecarTracked {
                    source_set: root.source_set.name.clone(),
                    path,
                });
            } else {
                let ambiguous_owners = owners.filter(|owners| owners.len() > 1);
                let reason = ambiguous_owners
                    .map(ambiguous_owner_reason)
                    .unwrap_or_else(|| "runtime sidecar is outside a proven source root".into());
                let additional_count =
                    1_usize.saturating_add(ambiguous_owners.map_or(0, |owners| owners.len()));
                let additional_bytes = path.len().saturating_add(reason.len()).saturating_add(
                    ambiguous_owners.map_or(0, |owners| {
                        owners.iter().fold(0_usize, |total, root| {
                            total
                                .saturating_add(path.len())
                                .saturating_add(reason.len())
                                .saturating_add(root.source_set.name.len())
                        })
                    }),
                );
                fact_bytes = match reserve_expanded_repository_fact_budget(
                    facts.len(),
                    fact_bytes,
                    additional_count,
                    additional_bytes,
                    "ConfigDumpInfo",
                ) {
                    Ok(bytes) => bytes,
                    Err(reason) => {
                        return Ok(Err(ConfigDumpInfoIndexInspectionError::Incomplete(reason)))
                    }
                };
                facts.push(ProjectHealthFact::ConfigDumpInfoUnclassified {
                    source_set: None,
                    path: path.clone(),
                    reason: reason.clone(),
                });
                if let Some(owners) = ambiguous_owners {
                    for (owner_index, root) in owners.iter().enumerate() {
                        if owner_index.is_multiple_of(256) {
                            if cancellation.is_cancelled() {
                                return Err(ProjectHealthInspectionError::Cancelled);
                            }
                            if deadline.is_elapsed() {
                                return Ok(Err(ConfigDumpInfoIndexInspectionError::TimedOut));
                            }
                        }
                        facts.push(ProjectHealthFact::ConfigDumpInfoUnclassified {
                            source_set: Some(root.source_set.name.clone()),
                            path: path.clone(),
                            reason: reason.clone(),
                        });
                    }
                }
            }
        }
        for path in inconclusive_paths {
            let owners = match source_owners.owners_for_repo_path(&path, cancellation, deadline) {
                Ok(owners) => owners,
                Err(GeneratedPathInspectionError::Cancelled) => {
                    return Err(ProjectHealthInspectionError::Cancelled)
                }
                Err(GeneratedPathInspectionError::TimedOut) => {
                    return Ok(Err(ConfigDumpInfoIndexInspectionError::TimedOut))
                }
                Err(GeneratedPathInspectionError::Incomplete(reason)) => {
                    return Ok(Err(ConfigDumpInfoIndexInspectionError::Incomplete(reason)))
                }
            };
            let reason = "staged blob classification is inconclusive";
            let additional_count = match owners {
                Some([_]) | None => 1,
                Some(owners) => 1_usize.saturating_add(owners.len()),
            };
            let owner_bytes = match owners {
                Some([root]) => root.source_set.name.len(),
                Some(owners) => owners.iter().fold(0_usize, |total, root| {
                    total
                        .saturating_add(path.len())
                        .saturating_add(reason.len())
                        .saturating_add(root.source_set.name.len())
                }),
                None => 0,
            };
            fact_bytes = match reserve_expanded_repository_fact_budget(
                facts.len(),
                fact_bytes,
                additional_count,
                path.len()
                    .saturating_add(reason.len())
                    .saturating_add(owner_bytes),
                "ConfigDumpInfo",
            ) {
                Ok(bytes) => bytes,
                Err(reason) => {
                    return Ok(Err(ConfigDumpInfoIndexInspectionError::Incomplete(reason)))
                }
            };
            match owners {
                Some([root]) => facts.push(ProjectHealthFact::ConfigDumpInfoUnclassified {
                    source_set: Some(root.source_set.name.clone()),
                    path,
                    reason: reason.into(),
                }),
                Some(owners) => {
                    facts.push(ProjectHealthFact::ConfigDumpInfoUnclassified {
                        source_set: None,
                        path: path.clone(),
                        reason: reason.into(),
                    });
                    for (owner_index, root) in owners.iter().enumerate() {
                        if owner_index.is_multiple_of(256) {
                            if cancellation.is_cancelled() {
                                return Err(ProjectHealthInspectionError::Cancelled);
                            }
                            if deadline.is_elapsed() {
                                return Ok(Err(ConfigDumpInfoIndexInspectionError::TimedOut));
                            }
                        }
                        facts.push(ProjectHealthFact::ConfigDumpInfoUnclassified {
                            source_set: Some(root.source_set.name.clone()),
                            path: path.clone(),
                            reason: reason.into(),
                        });
                    }
                }
                None => facts.push(ProjectHealthFact::ConfigDumpInfoUnclassified {
                    source_set: None,
                    path,
                    reason: reason.into(),
                }),
            }
        }
        Ok(Ok(ConfigDumpInfoEvaluation {
            facts,
            classifications,
        }))
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
    if output.stderr.trim().is_empty() {
        format!("Git exited with {}", output.status)
    } else {
        format!(
            "Git exited with {}: {}",
            output.status,
            output.stderr.trim()
        )
    }
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
    parse_git_index_entries_with_checkpoint(stdout, &mut || Ok(())).map_err(|error| match error {
        IndexParseError::Malformed(reason) => reason,
        IndexParseError::Cancelled => "Git index parsing was cancelled".into(),
        IndexParseError::TimedOut => "Git index parsing timed out".into(),
    })
}

pub(super) enum IndexParseError {
    Malformed(String),
    Cancelled,
    TimedOut,
}

pub(super) fn parse_git_index_entries_controlled(
    stdout: &str,
    cancellation: &CancellationToken,
    deadline: ProviderDeadline,
) -> Result<Vec<GitIndexEntry>, IndexParseError> {
    parse_git_index_entries_with_checkpoint(stdout, &mut || {
        if cancellation.is_cancelled() {
            Err(IndexParseError::Cancelled)
        } else if deadline.is_elapsed() {
            Err(IndexParseError::TimedOut)
        } else {
            Ok(())
        }
    })
}

fn parse_git_index_entries_with_checkpoint(
    stdout: &str,
    checkpoint: &mut dyn FnMut() -> Result<(), IndexParseError>,
) -> Result<Vec<GitIndexEntry>, IndexParseError> {
    #[derive(Default)]
    struct State {
        records: usize,
        blob_oid: Option<String>,
        mode: Option<String>,
    }
    if !stdout.is_empty() && !stdout.ends_with('\0') {
        return Err(IndexParseError::Malformed(
            "Git index output is missing its terminal NUL".into(),
        ));
    }
    let mut entries = BTreeMap::<String, State>::new();
    checkpoint()?;
    for (record_index, record) in stdout.split_terminator('\0').enumerate() {
        if record_index % 256 == 0 {
            checkpoint()?;
        }
        let (metadata, path) = record.split_once('\t').ok_or_else(|| {
            IndexParseError::Malformed("Git index record has no tab separator".into())
        })?;
        if path.is_empty() {
            return Err(IndexParseError::Malformed(
                "Git index record has an empty path".into(),
            ));
        }
        let fields = metadata.split_whitespace().collect::<Vec<_>>();
        if fields.len() != 3 {
            return Err(IndexParseError::Malformed(
                "Git index record has invalid metadata".into(),
            ));
        }
        let (mode, oid, stage) = (fields[0], fields[1], fields[2]);
        let usable = matches!(mode, "100644" | "100755")
            && stage == "0"
            && !oid.is_empty()
            && oid.bytes().all(|byte| byte.is_ascii_hexdigit())
            && oid.bytes().any(|byte| byte != b'0');
        let state = entries.entry(path.into()).or_default();
        state.records += 1;
        if state.records == 1 && usable {
            state.blob_oid = Some(oid.into());
            state.mode = Some(mode.into());
        } else {
            state.blob_oid = None;
            state.mode = None;
        }
    }
    checkpoint()?;
    Ok(entries
        .into_iter()
        .map(|(repo_path, state)| GitIndexEntry {
            repo_path,
            blob_oid: state.blob_oid,
            mode: state.mode,
        })
        .collect())
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
    let mut staged_by_path = BTreeMap::new();
    for (match_index, matched) in staged_matches.iter().enumerate() {
        if match_index.is_multiple_of(256) {
            checkpoint()?;
        }
        staged_by_path.insert(matched.path.as_str(), matched);
    }
    let mut working_by_path = BTreeMap::new();
    for (match_index, matched) in working_matches.iter().enumerate() {
        if match_index.is_multiple_of(256) {
            checkpoint()?;
        }
        working_by_path.insert(matched.path.as_str(), matched);
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
    if staged_ignore_files.len() > MAX_STAGED_IGNORE_FILES {
        return Err(StagedIgnoreMaterializationError::Incomplete(format!(
            "staged ignore policy contains {} .gitignore files; inspection supports at most {MAX_STAGED_IGNORE_FILES}",
            staged_ignore_files.len()
        )));
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
    let blobs = read_staged_ignore_blobs(
        runner,
        repository_root,
        &staged_ignore_files,
        cancellation,
        deadline,
    )?;
    for (path, oid) in staged_ignore_files {
        if cancellation.is_cancelled() {
            return Err(StagedIgnoreMaterializationError::Cancelled);
        }
        if deadline.is_elapsed() {
            return Err(StagedIgnoreMaterializationError::Incomplete(
                "staged .gitignore blob inspection timed out".into(),
            ));
        }
        let relative = Path::new(path);
        let target = root.path().join(relative);
        let mut target_file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)
            .map_err(|error| {
                let reason = if error.kind() == std::io::ErrorKind::AlreadyExists {
                    format!(
                        "staged .gitignore path {path} collides with another host path identity"
                    )
                } else {
                    format!("create staged .gitignore blob {path}: {error}")
                };
                StagedIgnoreMaterializationError::Incomplete(reason)
            })?;
        target_file.write_all(&blobs[oid]).map_err(|error| {
            StagedIgnoreMaterializationError::Incomplete(format!(
                "write staged .gitignore blob {path}: {error}"
            ))
        })?;
    }
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

fn read_staged_ignore_blobs(
    runner: &dyn ProcessRunner,
    repository_root: &Path,
    files: &[(&str, &str)],
    cancellation: &CancellationToken,
    deadline: ProviderDeadline,
) -> Result<BTreeMap<String, Vec<u8>>, StagedIgnoreMaterializationError> {
    let oids = files.iter().map(|(_, oid)| *oid).collect::<BTreeSet<_>>();
    let mut input = Vec::new();
    for oid in &oids {
        input.extend_from_slice(oid.as_bytes());
        input.push(b'\n');
    }
    let check = run_staged_ignore_batch(
        runner,
        repository_root,
        "--batch-check=%(objectname) %(objecttype) %(objectsize)",
        &input,
        MAX_STAGED_IGNORE_BATCH_METADATA_BYTES + 1,
        cancellation,
        deadline,
    )?;
    let mut sizes = BTreeMap::<&str, usize>::new();
    let lines = check.stdout.lines().collect::<Vec<_>>();
    if lines.len() != oids.len() || !check.stdout.ends_with('\n') {
        return Err(StagedIgnoreMaterializationError::Incomplete(
            "Git returned incomplete staged .gitignore blob metadata".into(),
        ));
    }
    for (oid, line) in oids.iter().zip(lines) {
        let mut fields = line.split(' ');
        let (Some(actual_oid), Some("blob"), Some(raw_size), None) =
            (fields.next(), fields.next(), fields.next(), fields.next())
        else {
            return Err(StagedIgnoreMaterializationError::Incomplete(format!(
                "Git returned invalid staged .gitignore blob metadata for {oid}"
            )));
        };
        if actual_oid != *oid {
            return Err(StagedIgnoreMaterializationError::Incomplete(format!(
                "Git returned the wrong staged .gitignore blob object id for {oid}"
            )));
        }
        let size = raw_size.parse::<usize>().map_err(|_| {
            StagedIgnoreMaterializationError::Incomplete(format!(
                "Git returned an invalid staged .gitignore blob size for {oid}"
            ))
        })?;
        sizes.insert(oid, size);
    }
    let mut total_bytes = 0_usize;
    for (_, oid) in files {
        total_bytes = total_bytes
            .checked_add(sizes[oid])
            .unwrap_or(MAX_STAGED_IGNORE_TOTAL_BYTES + 1);
        if total_bytes > MAX_STAGED_IGNORE_TOTAL_BYTES {
            return Err(StagedIgnoreMaterializationError::Incomplete(format!(
                "staged .gitignore policy exceeds {MAX_STAGED_IGNORE_TOTAL_BYTES} bytes"
            )));
        }
    }
    let mut expected_batch_bytes = 0_usize;
    for oid in &oids {
        expected_batch_bytes += oid.len() + " blob ".len() + sizes[oid].to_string().len() + 1;
        expected_batch_bytes += sizes[oid] + 1;
    }
    let batch = run_staged_ignore_batch(
        runner,
        repository_root,
        "--batch",
        &input,
        expected_batch_bytes + 1,
        cancellation,
        deadline,
    )?;
    let output = batch.stdout.as_bytes();
    let mut cursor = 0_usize;
    let mut blobs = BTreeMap::new();
    for oid in oids {
        if cancellation.is_cancelled() {
            return Err(StagedIgnoreMaterializationError::Cancelled);
        }
        if deadline.is_elapsed() {
            return Err(StagedIgnoreMaterializationError::Incomplete(
                "staged .gitignore blob inspection timed out".into(),
            ));
        }
        let size = sizes[oid];
        let header = format!("{oid} blob {size}\n");
        if !output[cursor..].starts_with(header.as_bytes()) {
            return Err(StagedIgnoreMaterializationError::Incomplete(format!(
                "Git returned invalid staged .gitignore batch framing for {oid}"
            )));
        }
        cursor += header.len();
        let end = cursor.checked_add(size).ok_or_else(|| {
            StagedIgnoreMaterializationError::Incomplete(
                "Git returned an oversized staged .gitignore blob".into(),
            )
        })?;
        if output.get(end) != Some(&b'\n') {
            return Err(StagedIgnoreMaterializationError::Incomplete(format!(
                "Git returned incomplete staged .gitignore blob bytes for {oid}"
            )));
        }
        blobs.insert(oid.to_owned(), output[cursor..end].to_vec());
        cursor = end + 1;
    }
    if cursor != output.len() {
        return Err(StagedIgnoreMaterializationError::Incomplete(
            "Git returned unexpected trailing staged .gitignore blob bytes".into(),
        ));
    }
    Ok(blobs)
}

#[allow(clippy::too_many_arguments)]
fn run_staged_ignore_batch(
    runner: &dyn ProcessRunner,
    repository_root: &Path,
    mode: &str,
    input: &[u8],
    capture_limit: usize,
    cancellation: &CancellationToken,
    deadline: ProviderDeadline,
) -> Result<ProcessOutput, StagedIgnoreMaterializationError> {
    if cancellation.is_cancelled() {
        return Err(StagedIgnoreMaterializationError::Cancelled);
    }
    if deadline.is_elapsed() {
        return Err(StagedIgnoreMaterializationError::Incomplete(
            "staged .gitignore blob inspection timed out".into(),
        ));
    }
    let mut command = process_command_vec(
        repository_root,
        vec![
            "--no-replace-objects".into(),
            "cat-file".into(),
            mode.into(),
        ],
        cancellation,
        deadline,
    );
    command.capture_limits = Some((capture_limit, unica_process::STDERR_CAPTURE_LIMIT));
    let output = match sticky_process_result(runner.run_with_input(&command, input), cancellation) {
        Ok(output) => output,
        Err(_) if cancellation.is_cancelled() => {
            return Err(StagedIgnoreMaterializationError::Cancelled)
        }
        Err(reason) => return Err(StagedIgnoreMaterializationError::Incomplete(reason)),
    };
    if output.cancelled || cancellation.is_cancelled() {
        return Err(StagedIgnoreMaterializationError::Cancelled);
    }
    if !output.status_success || output.timed_out || output_incomplete(&output) {
        let reason = if output.timed_out {
            "staged .gitignore blob inspection timed out".into()
        } else if output_incomplete(&output) {
            completeness_reason(&output)
        } else {
            nonzero_reason(&output)
        };
        return Err(StagedIgnoreMaterializationError::Incomplete(reason));
    }
    Ok(output)
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
                        logical.display()
                    ))
                })?;
                let identity = directory.identity().map_err(|error| {
                    StagedIgnoreMaterializationError::Incomplete(format!(
                        "identify staged Git ignore path {}: {error}",
                        logical.display()
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
                        logical.display()
                    ))
                })?;
                let identity = directory.identity().map_err(|error| {
                    StagedIgnoreMaterializationError::Incomplete(format!(
                        "identify colliding staged Git ignore path {}: {error}",
                        logical.display()
                    ))
                })?;
                let Some(first) = materialized.identities.get(&identity).cloned() else {
                    return Err(StagedIgnoreMaterializationError::Incomplete(format!(
                        "staged Git ignore path {} unexpectedly resolves to an untracked host identity",
                        logical.display()
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
                    logical.display()
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
    if output_incomplete(&output) {
        return Ok(WorkingIgnoreInspection::Incomplete(completeness_reason(
            &output,
        )));
    }
    if !output.status_success && !status_is_no_match(&output) {
        return Ok(WorkingIgnoreInspection::Incomplete(nonzero_reason(&output)));
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
    let mut facts = Vec::new();
    let mut fact_bytes = 0_usize;
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
                    let additional_bytes = owners.iter().fold(0_usize, |total, root| {
                        total
                            .saturating_add(entry.repo_path.len())
                            .saturating_add(root.source_set.name.len())
                    });
                    fact_bytes = reserve_expanded_repository_fact_budget(
                        facts.len(),
                        fact_bytes,
                        owners.len(),
                        additional_bytes,
                        "generated path",
                    )
                    .map_err(GeneratedPathInspectionError::Incomplete)?;
                    for (owner_index, root) in owners.iter().enumerate() {
                        if owner_index.is_multiple_of(256) {
                            if cancellation.is_cancelled() {
                                return Err(GeneratedPathInspectionError::Cancelled);
                            }
                            if deadline.is_elapsed() {
                                return Err(GeneratedPathInspectionError::TimedOut);
                            }
                        }
                        facts.push(ProjectHealthFact::GeneratedPathTracked {
                            source_set: Some(root.source_set.name.clone()),
                            path: entry.repo_path.clone(),
                        });
                    }
                }
                _ => {
                    fact_bytes = reserve_expanded_repository_fact_budget(
                        facts.len(),
                        fact_bytes,
                        1,
                        entry.repo_path.len(),
                        "generated path",
                    )
                    .map_err(GeneratedPathInspectionError::Incomplete)?;
                    facts.push(ProjectHealthFact::GeneratedPathTracked {
                        source_set: None,
                        path: entry.repo_path.clone(),
                    });
                }
            }
        }
    }
    Ok(facts)
}

fn reserve_expanded_repository_fact_budget(
    current_count: usize,
    current_bytes: usize,
    additional_count: usize,
    additional_bytes: usize,
    fact_kind: &str,
) -> Result<usize, String> {
    let next_count = current_count.saturating_add(additional_count);
    let next_bytes = current_bytes.saturating_add(additional_bytes);
    if next_count > MAX_EXPANDED_REPOSITORY_FACTS || next_bytes > MAX_EXPANDED_REPOSITORY_FACT_BYTES
    {
        return Err(format!(
            "{fact_kind} fact budget exceeded while expanding source-set ownership"
        ));
    }
    Ok(next_bytes)
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
