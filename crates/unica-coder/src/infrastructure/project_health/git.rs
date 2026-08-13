use crate::domain::cancellation::CancellationToken;
use crate::domain::code_intelligence::ProviderDeadline;
use crate::domain::project_health::{
    ProjectCheckId, ProjectCheckObservation, ProjectCheckOutcome, ProjectHealthFact,
    ProjectHealthInspectionError,
};
use crate::domain::project_sources::{
    config_dump_info_xml_kind, ConfigDumpInfoXmlKind, SourceFormat, SourceSetKind,
};
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::internal_adapters::{
    system_process_runner, ProcessCommand, ProcessOutput, ProcessRunner,
};
use crate::infrastructure::platform::filesystem::{host_path_text, path_starts_with_host_root};
use crate::infrastructure::project_health::layout::{InspectedSourceRoot, SourceLayoutInspection};
use crate::infrastructure::project_health::SourceRootOwnerIndex;
use crate::infrastructure::source_roots::normalize_path_identity;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};
use tempfile::TempDir;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GitIndexEntry {
    pub(crate) repo_path: String,
    pub(crate) blob_oid: Option<String>,
    pub(crate) mode: Option<String>,
}

const MAX_STAGED_IGNORE_FILES: usize = 1024;
const MAX_STAGED_IGNORE_TOTAL_BYTES: usize = 8 * 1024 * 1024;

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
enum IgnoreInspectionError {
    Malformed(String),
    Cancelled,
    TimedOut,
}

pub(crate) struct GitRepositoryInspection {
    pub(crate) repository_root: Option<PathBuf>,
    pub(crate) entries: Vec<GitIndexEntry>,
    pub(crate) resource_inspection_blocker: Option<String>,
    pub(crate) observations: Vec<ProjectCheckObservation>,
    pub(crate) facts: Vec<ProjectHealthFact>,
}

pub(crate) struct ConfigDumpInfoIndexInspection {
    pub(crate) runtime_paths: Vec<String>,
    pub(crate) inconclusive_paths: Vec<String>,
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
            } else if deadline.remaining().is_zero() {
                Err(GeneratedPathInspectionError::TimedOut)
            } else {
                Ok(())
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
        let owners = self.index.deepest_owners_with_checkpoint(path, &mut || {
            if cancellation.is_cancelled() {
                return Err(GeneratedPathInspectionError::Cancelled);
            }
            if deadline.remaining().is_zero() {
                return Err(GeneratedPathInspectionError::TimedOut);
            }
            Ok(())
        })?;
        Ok(owners.map(|(owners, _depth)| owners))
    }
}

impl GitRepositoryInspector<'static> {
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
                        observations,
                        facts,
                    });
                }
            };
        observations.push(completed(ProjectCheckId::RepositoryIndex));

        if !layout.repository_targets_complete {
            append_not_run_after_index(
                &mut observations,
                &layout.roots,
                "source-set targets are incomplete",
            );
            return Ok(GitRepositoryInspection {
                repository_root: Some(repository_root),
                entries,
                resource_inspection_blocker: None,
                observations,
                facts,
            });
        }

        let blob_read_safety = if staged_blob_read_needed(&entries)
            || layout
                .roots
                .iter()
                .any(|root| root.source_set.source_format == SourceFormat::PlatformXml)
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
        }

        match self.inspect_config_dump_info(
            &entries,
            &repository_root,
            &source_owners,
            &blob_read_safety,
            cancellation,
            deadline,
        )? {
            Ok(config_dump_facts) => {
                observations.push(completed(ProjectCheckId::RepositoryConfigDumpInfo));
                append_completed_for_roots(
                    &mut observations,
                    ProjectCheckId::RepositoryConfigDumpInfo,
                    &layout.roots,
                );
                facts.extend(config_dump_facts);
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
            &blob_read_safety,
            cancellation,
            deadline,
        ) {
            Ok(root) => root,
            Err(ProjectHealthInspectionError::Cancelled) => {
                return Err(ProjectHealthInspectionError::Cancelled)
            }
            Err(ProjectHealthInspectionError::Fatal(reason)) => {
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
                                    observations.push(if incomplete_ignore_roots.is_empty() {
                                        completed(ProjectCheckId::RepositoryIgnore)
                                    } else {
                                        not_run(
                                            ProjectCheckId::RepositoryIgnore,
                                            "source format is incomplete, so format-dependent ignore targets are unknown",
                                        )
                                    });
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
                                    if !incomplete_ignore_roots.is_empty() {
                                        ignore_facts.retain(|fact| match fact {
                                            ProjectHealthFact::IgnoreRuleMissing {
                                                source_set,
                                                ..
                                            }
                                            | ProjectHealthFact::IgnoreRuleLocalOnly {
                                                source_set,
                                                ..
                                            } => source_set.as_deref().is_some_and(|source_set| {
                                                !incomplete_ignore_roots
                                                    .iter()
                                                    .any(|root| root.source_set.name == source_set)
                                            }),
                                            _ => true,
                                        });
                                    }
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
        Result<Vec<ProjectHealthFact>, ConfigDumpInfoIndexInspectionError>,
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
        for path in inspection.runtime_paths {
            let owners = match source_owners.owners_for_repo_path(&path, cancellation, deadline) {
                Ok(owners) => owners,
                Err(GeneratedPathInspectionError::Cancelled) => {
                    return Err(ProjectHealthInspectionError::Cancelled)
                }
                Err(GeneratedPathInspectionError::TimedOut) => {
                    return Ok(Err(ConfigDumpInfoIndexInspectionError::TimedOut))
                }
            };
            if let Some([root]) = owners {
                facts.push(ProjectHealthFact::RuntimeSidecarTracked {
                    source_set: root.source_set.name.clone(),
                    path,
                });
            } else {
                let reason = owners
                    .filter(|owners| owners.len() > 1)
                    .map(|owners| {
                        let mut names = owners
                            .iter()
                            .map(|root| root.source_set.name.clone())
                            .collect::<Vec<_>>();
                        names.sort();
                        names.dedup();
                        format!(
                            "runtime sidecar has ambiguous equal-depth source-set owners: {}",
                            names.join(", ")
                        )
                    })
                    .unwrap_or_else(|| "runtime sidecar is outside a proven source root".into());
                facts.push(ProjectHealthFact::ConfigDumpInfoUnclassified {
                    source_set: None,
                    path,
                    reason,
                });
            }
        }
        for path in inspection.inconclusive_paths {
            let source_set = match source_owners.owners_for_repo_path(&path, cancellation, deadline)
            {
                Ok(Some([root])) => Some(root.source_set.name.clone()),
                Ok(_) => None,
                Err(GeneratedPathInspectionError::Cancelled) => {
                    return Err(ProjectHealthInspectionError::Cancelled)
                }
                Err(GeneratedPathInspectionError::TimedOut) => {
                    return Ok(Err(ConfigDumpInfoIndexInspectionError::TimedOut))
                }
            };
            facts.push(ProjectHealthFact::ConfigDumpInfoUnclassified {
                source_set,
                path,
                reason: "staged blob classification is inconclusive".into(),
            });
        }
        Ok(Ok(facts))
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
        if deadline.remaining().is_zero() {
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
        if deadline.remaining().is_zero() {
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
            crate::infrastructure::platform::STDERR_CAPTURE_LIMIT,
        )),
        timeout: Some(deadline.remaining()),
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

pub(crate) fn parse_git_index_entries(stdout: &str) -> Result<Vec<GitIndexEntry>, String> {
    parse_git_index_entries_with_checkpoint(stdout, &mut || Ok(())).map_err(|error| match error {
        IndexParseError::Malformed(reason) => reason,
        IndexParseError::Cancelled => "Git index parsing was cancelled".into(),
        IndexParseError::TimedOut => "Git index parsing timed out".into(),
    })
}

enum IndexParseError {
    Malformed(String),
    Cancelled,
    TimedOut,
}

fn parse_git_index_entries_controlled(
    stdout: &str,
    cancellation: &CancellationToken,
    deadline: ProviderDeadline,
) -> Result<Vec<GitIndexEntry>, IndexParseError> {
    parse_git_index_entries_with_checkpoint(stdout, &mut || {
        if cancellation.is_cancelled() {
            Err(IndexParseError::Cancelled)
        } else if deadline.remaining().is_zero() {
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
        if let Some(path) = repo_path(
            repository_root,
            &root.path.join(".build/.unica-health-probe"),
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
                if let Some(path) = repo_path(repository_root, &root.path.join(name)) {
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
    } else if deadline.remaining().is_zero() {
        Err(IgnoreInspectionError::TimedOut)
    } else {
        Ok(())
    }
}

fn materialize_staged_ignore_files(
    runner: &dyn ProcessRunner,
    repository_root: &Path,
    entries: &[GitIndexEntry],
    blob_read_safety: &StagedBlobReadSafety,
    cancellation: &CancellationToken,
    deadline: ProviderDeadline,
) -> Result<TempDir, ProjectHealthInspectionError> {
    let root = TempDir::new().map_err(|error| {
        ProjectHealthInspectionError::Fatal(format!(
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
            return Err(ProjectHealthInspectionError::Fatal(format!(
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
        return Err(ProjectHealthInspectionError::Fatal(format!(
            "staged ignore policy contains {} .gitignore files; inspection supports at most {MAX_STAGED_IGNORE_FILES}",
            staged_ignore_files.len()
        )));
    }
    if let StagedBlobReadSafety::Blocked(reason) = blob_read_safety {
        return Err(ProjectHealthInspectionError::Fatal(reason.clone()));
    }
    let mut total_bytes = 0_usize;
    for (path, oid) in staged_ignore_files {
        if cancellation.is_cancelled() {
            return Err(ProjectHealthInspectionError::Cancelled);
        }
        if deadline.remaining().is_zero() {
            return Err(ProjectHealthInspectionError::Fatal(
                "staged .gitignore blob inspection timed out".into(),
            ));
        }
        let command = process_command_vec(
            repository_root,
            vec![
                "--no-replace-objects".into(),
                "cat-file".into(),
                "blob".into(),
                oid.into(),
            ],
            cancellation,
            deadline,
        );
        let remaining = MAX_STAGED_IGNORE_TOTAL_BYTES.saturating_sub(total_bytes);
        let mut command = command;
        command.capture_limits = Some((
            remaining.saturating_add(1),
            crate::infrastructure::platform::STDERR_CAPTURE_LIMIT,
        ));
        let output = match sticky_process_result(runner.run(&command), cancellation) {
            Ok(output) => output,
            Err(_) if cancellation.is_cancelled() => {
                return Err(ProjectHealthInspectionError::Cancelled)
            }
            Err(reason) => return Err(ProjectHealthInspectionError::Fatal(reason)),
        };
        if output.cancelled || cancellation.is_cancelled() {
            return Err(ProjectHealthInspectionError::Cancelled);
        }
        if !output.status_success || output.timed_out || output_incomplete(&output) {
            let reason = if output.timed_out {
                "staged .gitignore blob inspection timed out".into()
            } else if output_incomplete(&output) {
                completeness_reason(&output)
            } else {
                nonzero_reason(&output)
            };
            return Err(ProjectHealthInspectionError::Fatal(reason));
        }
        total_bytes = total_bytes.saturating_add(output.stdout.len());
        if total_bytes > MAX_STAGED_IGNORE_TOTAL_BYTES {
            return Err(ProjectHealthInspectionError::Fatal(format!(
                "staged .gitignore policy exceeds {MAX_STAGED_IGNORE_TOTAL_BYTES} bytes"
            )));
        }
        let relative = Path::new(path);
        if relative.is_absolute()
            || relative
                .components()
                .any(|component| !matches!(component, Component::Normal(_)))
        {
            return Err(ProjectHealthInspectionError::Fatal(format!(
                "staged .gitignore path is not a safe repository-relative path: {path}"
            )));
        }
        let target = root.path().join(relative);
        let parent = target.parent().ok_or_else(|| {
            ProjectHealthInspectionError::Fatal(format!(
                "staged .gitignore path has no parent: {path}"
            ))
        })?;
        std::fs::create_dir_all(parent).map_err(|error| {
            ProjectHealthInspectionError::Fatal(format!(
                "create staged .gitignore parent for {path}: {error}"
            ))
        })?;
        std::fs::write(&target, output.stdout.as_bytes()).map_err(|error| {
            ProjectHealthInspectionError::Fatal(format!(
                "write staged .gitignore blob {path}: {error}"
            ))
        })?;
    }
    Ok(root)
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
            || crate::infrastructure::platform::filesystem::metadata_is_link_or_reparse_point(
                &metadata,
            )
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
    for (entry_index, entry) in entries.iter().enumerate() {
        if entry_index % 256 == 0 {
            if cancellation.is_cancelled() {
                return Err(GeneratedPathInspectionError::Cancelled);
            }
            if deadline.remaining().is_zero() {
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
        let generated = entry.repo_path == ".build"
            || entry.repo_path.starts_with(".build/")
            || entry.repo_path.contains("/.build/")
            || Path::new(&entry.repo_path)
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
            facts.push(ProjectHealthFact::GeneratedPathTracked {
                source_set: source_owners
                    .owners_for_repo_path(&entry.repo_path, cancellation, deadline)?
                    .and_then(|owners| {
                        (owners.len() == 1).then(|| owners[0].source_set.name.clone())
                    }),
                path: entry.repo_path.clone(),
            });
        }
    }
    Ok(facts)
}

fn repo_path(repository_root: &Path, path: &Path) -> Option<String> {
    let path = normalize_path_identity(path).ok()?;
    let relative = path.strip_prefix(repository_root).ok()?;
    Some(host_path_text(relative.to_string_lossy().into_owned()))
}

#[cfg(test)]
mod tests {
    use super::{
        git_environment_removals_from, materialize_staged_ignore_files,
        parse_check_ignore_verbose_z, parse_git_index_entries, partial_clone_config_is_enabled,
        GitIndexEntry, GitRepositoryInspector, IgnoreMatch, StagedBlobReadSafety,
    };
    use crate::domain::cancellation::CancellationToken;
    use crate::domain::code_intelligence::ProviderDeadline;
    use crate::domain::project_health::{
        ProjectCheckId, ProjectCheckOutcome, ProjectHealthFact, ProjectHealthInspectionError,
    };
    use crate::domain::project_sources::SourceFormat;
    use crate::domain::workspace::WorkspaceContext;
    use crate::infrastructure::internal_adapters::{ProcessCommand, ProcessOutput, ProcessRunner};
    use crate::infrastructure::project_health::layout::SourceLayoutInspector;
    use std::cell::{Cell, RefCell};
    use std::ffi::OsString;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};
    use tempfile::TempDir;

    static CONFIG_DUMP_OWNERSHIP_CLOCK_TICKS: AtomicUsize = AtomicUsize::new(0);

    fn advancing_config_dump_ownership_clock() -> Instant {
        let tick = CONFIG_DUMP_OWNERSHIP_CLOCK_TICKS.fetch_add(1, Ordering::SeqCst) as u64;
        Instant::now() + Duration::from_millis(tick)
    }

    static OWNER_INDEX_CLOCK_TICKS: AtomicUsize = AtomicUsize::new(0);

    fn advancing_owner_index_clock() -> Instant {
        let tick = OWNER_INDEX_CLOCK_TICKS.fetch_add(1, Ordering::SeqCst) as u64;
        Instant::now() + Duration::from_millis(tick)
    }

    #[test]
    fn source_owner_index_timeout_marks_ignore_not_run_for_every_source_set() {
        let fixture = health_fixture(false);
        let layout = inspect_layout(&fixture.context);
        let mut timed_out_inspection = None;

        for budget_millis in 1..64 {
            OWNER_INDEX_CLOCK_TICKS.store(0, Ordering::SeqCst);
            let deadline = ProviderDeadline::with_clock(
                advancing_owner_index_clock() + Duration::from_millis(budget_millis),
                advancing_owner_index_clock,
            );
            let mut no_partial_clone = process_output(false, "");
            no_partial_clone.status = "exit status: 1".into();
            let runner = SequenceRunner::outputs(vec![
                process_output(
                    true,
                    &format!("{}\ntrue\n", fixture.context.workspace_root.display()),
                ),
                process_output(true, ""),
                no_partial_clone,
            ]);
            let inspection = GitRepositoryInspector::with_runner(&runner)
                .inspect_base(
                    &fixture.context,
                    &layout,
                    &CancellationToken::new(),
                    deadline,
                )
                .unwrap();
            if inspection.observations.iter().any(|observation| {
                observation.id == ProjectCheckId::RepositoryIgnore
                    && observation.source_set.is_none()
                    && matches!(
                        &observation.outcome,
                        ProjectCheckOutcome::NotRun { reason }
                            if reason == "source-root ownership index construction timed out"
                    )
            }) {
                timed_out_inspection = Some(inspection);
                break;
            }
        }

        let inspection = timed_out_inspection.expect("fixture must expire while building owners");
        assert!(inspection.observations.iter().any(|observation| {
            observation.id == ProjectCheckId::RepositoryIgnore
                && observation.source_set.as_deref() == Some("main")
                && matches!(observation.outcome, ProjectCheckOutcome::NotRun { .. })
        }));
    }

    #[test]
    fn config_dump_info_ownership_deadline_is_typed_not_fatal() {
        let fixture = health_fixture(false);
        let path = format!("{}/ConfigDumpInfo.xml", "deep/".repeat(600));
        let entries = vec![GitIndexEntry {
            repo_path: path.clone(),
            blob_oid: Some("a".repeat(40)),
            mode: Some("100644".into()),
        }];
        let runner = SequenceRunner::outputs(vec![process_output(true, "<ConfigDumpInfo/>")]);
        let layout = inspect_layout(&fixture.context);
        let owners = super::SourceRootOwners::new(
            fixture.context.workspace_root.as_path(),
            &layout.roots,
            &CancellationToken::new(),
            ProviderDeadline::from_budget(Duration::from_secs(1)),
        )
        .unwrap();
        CONFIG_DUMP_OWNERSHIP_CLOCK_TICKS.store(0, Ordering::SeqCst);
        let deadline = ProviderDeadline::with_clock(
            advancing_config_dump_ownership_clock() + Duration::from_millis(4),
            advancing_config_dump_ownership_clock,
        );

        let inspection = GitRepositoryInspector::with_runner(&runner)
            .inspect_config_dump_info(
                &entries,
                fixture.context.workspace_root.as_path(),
                &owners,
                &StagedBlobReadSafety::LocalOnlyGuaranteed,
                &CancellationToken::new(),
                deadline,
            )
            .expect("deadline is reported inside the typed config-dump outcome");

        assert!(matches!(
            inspection,
            Err(super::ConfigDumpInfoIndexInspectionError::TimedOut)
        ));
    }

    #[test]
    fn generated_path_timeout_discards_partial_ordinary_facts() {
        let fixture = health_fixture(false);
        let layout = inspect_layout(&fixture.context);
        let cancellation = CancellationToken::new();
        let owners = super::SourceRootOwners::new(
            fixture.context.workspace_root.as_path(),
            &layout.roots,
            &cancellation,
            ProviderDeadline::from_budget(Duration::from_secs(1)),
        )
        .unwrap();
        let mut entries = vec![GitIndexEntry {
            repo_path: "src/.build/first.bin".into(),
            blob_oid: Some("a".repeat(40)),
            mode: Some("100644".into()),
        }];
        entries.extend((1..=256).map(|index| GitIndexEntry {
            repo_path: format!("ordinary/{index}.txt"),
            blob_oid: Some(format!("{index:040x}")),
            mode: Some("100644".into()),
        }));
        CONFIG_DUMP_OWNERSHIP_CLOCK_TICKS.store(0, Ordering::SeqCst);
        let deadline = ProviderDeadline::with_clock(
            advancing_config_dump_ownership_clock() + Duration::from_millis(3),
            advancing_config_dump_ownership_clock,
        );

        let result = super::tracked_generated_facts(
            &entries,
            fixture.context.workspace_root.as_path(),
            &fixture.context,
            &owners,
            &cancellation,
            deadline,
        );

        assert!(matches!(
            result,
            Err(super::GeneratedPathInspectionError::TimedOut)
        ));
    }

    #[test]
    fn equal_root_runtime_sidecar_reports_ambiguous_owners() {
        let fixture = health_fixture(false);
        fs::write(
            fixture.context.workspace_root.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: first\n    type: CONFIGURATION\n    path: src\n  - name: second\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        let layout = inspect_layout(&fixture.context);
        let cancellation = CancellationToken::new();
        let owners = super::SourceRootOwners::new(
            fixture.context.workspace_root.as_path(),
            &layout.roots,
            &cancellation,
            ProviderDeadline::from_budget(Duration::from_secs(1)),
        )
        .unwrap();
        let entries = vec![GitIndexEntry {
            repo_path: "src/ConfigDumpInfo.xml".into(),
            blob_oid: Some("a".repeat(40)),
            mode: Some("100644".into()),
        }];
        let runner = SequenceRunner::outputs(vec![process_output(true, "<ConfigDumpInfo/>")]);

        let facts = GitRepositoryInspector::with_runner(&runner)
            .inspect_config_dump_info(
                &entries,
                fixture.context.workspace_root.as_path(),
                &owners,
                &StagedBlobReadSafety::LocalOnlyGuaranteed,
                &cancellation,
                ProviderDeadline::from_budget(Duration::from_secs(1)),
            )
            .unwrap()
            .unwrap();

        assert!(matches!(
            facts.as_slice(),
            [ProjectHealthFact::ConfigDumpInfoUnclassified { reason, .. }]
                if reason.contains("ambiguous") && reason.contains("first") && reason.contains("second")
        ));
    }

    #[test]
    fn project_health_git_index_parser_groups_stages_and_preserves_unusual_paths() {
        let entries = parse_git_index_entries(concat!(
            "100644 aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa 1\tconflict/ConfigDumpInfo.xml\0",
            "100644 bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb 2\tconflict/ConfigDumpInfo.xml\0",
            "120000 cccccccccccccccccccccccccccccccccccccccc 0\tlinked\0",
            "100644 0000000000000000000000000000000000000000 0\tnew\0",
            "100755 dddddddddddddddddddddddddddddddddddddddd 0\tline\nbreak,ok\0",
        ))
        .unwrap();

        assert_eq!(entries.len(), 4);
        assert_eq!(entries[0].repo_path, "conflict/ConfigDumpInfo.xml");
        assert_eq!(entries[0].blob_oid, None);
        assert_eq!(entries[1].repo_path, "line\nbreak,ok");
        assert_eq!(
            entries[1].blob_oid.as_deref(),
            Some("dddddddddddddddddddddddddddddddddddddddd")
        );
        assert_eq!(entries[1].mode.as_deref(), Some("100755"));
        assert_eq!(entries[2].blob_oid, None);
        assert_eq!(entries[3].blob_oid, None);
    }

    #[test]
    fn partial_clone_parser_preserves_whitespace_inside_remote_name() {
        assert!(partial_clone_config_is_enabled(
            "remote.foo bar.promisor\ntrue\0"
        ));
        assert!(!partial_clone_config_is_enabled(
            "remote.foo bar.promisor\nfalse\0"
        ));
    }

    #[test]
    fn project_health_git_ignore_rejects_index_entry_without_unique_blob() {
        let repository = TempDir::new().unwrap();
        let runner = SequenceRunner::outputs(Vec::new());
        let error = materialize_staged_ignore_files(
            &runner,
            repository.path(),
            &[GitIndexEntry {
                repo_path: ".gitignore".into(),
                blob_oid: None,
                mode: None,
            }],
            &StagedBlobReadSafety::LocalOnlyGuaranteed,
            &CancellationToken::new(),
            ProviderDeadline::from_budget(Duration::from_secs(1)),
        )
        .unwrap_err();

        assert!(
            matches!(
                error,
                crate::domain::project_health::ProjectHealthInspectionError::Fatal(ref reason)
                    if reason.contains("does not have one regular stage-0 blob")
            ),
            "{error:?}"
        );
        assert!(runner.commands.borrow().is_empty());
    }

    #[test]
    fn project_health_git_ignore_bounds_staged_policy_file_count() {
        let repository = TempDir::new().unwrap();
        let runner = SequenceRunner::outputs(Vec::new());
        let entries = (0..=super::MAX_STAGED_IGNORE_FILES)
            .map(|index| GitIndexEntry {
                repo_path: format!("nested/{index}/.gitignore"),
                blob_oid: Some(format!("{index:040x}")),
                mode: Some("100644".into()),
            })
            .collect::<Vec<_>>();

        let error = materialize_staged_ignore_files(
            &runner,
            repository.path(),
            &entries,
            &StagedBlobReadSafety::LocalOnlyGuaranteed,
            &CancellationToken::new(),
            ProviderDeadline::from_budget(Duration::from_secs(1)),
        )
        .unwrap_err();

        assert!(
            matches!(
                error,
                crate::domain::project_health::ProjectHealthInspectionError::Fatal(ref reason)
                    if reason.contains("at most 1024")
            ),
            "{error:?}"
        );
        assert!(runner.commands.borrow().is_empty());
    }

    #[test]
    fn project_health_git_ignore_bounds_total_staged_policy_bytes() {
        let repository = TempDir::new().unwrap();
        let runner = SequenceRunner::outputs(vec![process_output(
            true,
            &"x".repeat(super::MAX_STAGED_IGNORE_TOTAL_BYTES + 1),
        )]);

        let error = materialize_staged_ignore_files(
            &runner,
            repository.path(),
            &[GitIndexEntry {
                repo_path: ".gitignore".into(),
                blob_oid: Some("a".repeat(40)),
                mode: Some("100644".into()),
            }],
            &StagedBlobReadSafety::LocalOnlyGuaranteed,
            &CancellationToken::new(),
            ProviderDeadline::from_budget(Duration::from_secs(1)),
        )
        .unwrap_err();

        assert!(
            matches!(
                error,
                crate::domain::project_health::ProjectHealthInspectionError::Fatal(ref reason)
                    if reason.contains("exceeds 8388608 bytes")
            ),
            "{error:?}"
        );
        assert_eq!(
            runner.commands.borrow()[0].capture_limits,
            Some((super::MAX_STAGED_IGNORE_TOTAL_BYTES + 1, 256 * 1024))
        );
    }

    #[test]
    fn project_health_git_removes_every_inherited_git_variable() {
        assert_eq!(
            git_environment_removals_from([
                (OsString::from("PATH"), OsString::from("/bin")),
                (OsString::from("GIT_DIR"), OsString::from("elsewhere")),
                (
                    OsString::from("GIT_INDEX_FILE"),
                    OsString::from("alternate-index"),
                ),
                (
                    OsString::from("GIT_OBJECT_DIRECTORY"),
                    OsString::from("alternate-objects"),
                ),
                (OsString::from("GIT_FUTURE_OVERRIDE"), OsString::from("1")),
                (OsString::from("git_dir"), OsString::from("mixed-case")),
            ]),
            vec![
                OsString::from("GIT_DIR"),
                OsString::from("GIT_FUTURE_OVERRIDE"),
                OsString::from("GIT_INDEX_FILE"),
                OsString::from("GIT_OBJECT_DIRECTORY"),
                OsString::from("git_dir"),
            ]
        );
    }

    #[test]
    fn project_health_git_commands_use_large_but_bounded_capture() {
        let command = super::process_command(
            Path::new("."),
            ["status"],
            &CancellationToken::new(),
            ProviderDeadline::from_budget(Duration::from_secs(1)),
        );

        assert_eq!(
            command.capture_limits,
            Some((
                super::super::PROJECT_HEALTH_STDOUT_CAPTURE_LIMIT,
                256 * 1024
            ))
        );
        assert_eq!(
            super::super::PROJECT_HEALTH_STDOUT_CAPTURE_LIMIT,
            64 * 1024 * 1024
        );
        assert!(
            command
                .env
                .iter()
                .any(|(name, value)| { name == "GIT_NO_LAZY_FETCH" && value == "1" }),
            "{:?}",
            command.env
        );
        for (name, value) in [
            ("GIT_NO_REPLACE_OBJECTS", "1"),
            ("GIT_TERMINAL_PROMPT", "0"),
        ] {
            assert!(
                command
                    .env
                    .iter()
                    .any(|(actual_name, actual_value)| actual_name == name && actual_value == value),
                "{name}: {:?}",
                command.env
            );
        }
        assert!(
            command
                .env
                .iter()
                .any(|(name, value)| { name == "GIT_OPTIONAL_LOCKS" && value == "0" }),
            "{:?}",
            command.env
        );
    }

    #[test]
    fn project_health_git_index_parser_rejects_malformed_nul_record() {
        assert!(parse_git_index_entries("100644 oid 0 without-tab\0").is_err());
        assert!(parse_git_index_entries("100644 abc 0\tpath-without-terminal-nul").is_err());
    }

    #[test]
    fn project_health_git_ignore_parser_reads_verbose_z_quadruples() {
        assert_eq!(
            parse_check_ignore_verbose_z(concat!(
                ".gitignore\0",
                "007\0",
                "**/.build/\0",
                "workspace/src/.build/probe\0"
            ))
            .unwrap(),
            vec![IgnoreMatch {
                source: ".gitignore".into(),
                line: "007".into(),
                pattern: "**/.build/".into(),
                path: "workspace/src/.build/probe".into(),
            }]
        );
        assert!(parse_check_ignore_verbose_z(concat!(".gitignore\0", "1\0", "pattern\0")).is_err());
    }

    #[test]
    fn project_health_git_ignore_parser_honors_cooperative_checkpoint() {
        let mut checkpoints = 0;
        let error = super::parse_check_ignore_verbose_z_with_checkpoint(
            concat!(
                ".gitignore\0",
                "1\0",
                "**/.build/\0",
                ".build/unica/probe\0"
            ),
            &mut || {
                checkpoints += 1;
                if checkpoints == 2 {
                    Err(super::IgnoreInspectionError::TimedOut)
                } else {
                    Ok(())
                }
            },
        )
        .unwrap_err();

        assert!(matches!(error, super::IgnoreInspectionError::TimedOut));
    }

    #[test]
    fn project_health_git_ignore_fact_composition_is_all_or_nothing() {
        let candidates = vec![super::IgnoreCandidate {
            source_set: Some("main".into()),
            repo_path: "src/.build/.unica-health-probe".into(),
        }];
        let entries = vec![GitIndexEntry {
            repo_path: ".gitignore".into(),
            blob_oid: Some("a".repeat(40)),
            mode: Some("100644".into()),
        }];

        let result = super::ignore_facts_with_checkpoint(
            &candidates,
            &[],
            &[],
            &entries,
            Path::new("/staged"),
            &mut || Err(super::IgnoreInspectionError::TimedOut),
        );

        assert!(matches!(
            result,
            Err(super::IgnoreInspectionError::TimedOut)
        ));
    }

    #[test]
    fn git_no_match_requires_exact_exit_code_one() {
        let output = |status: &str| ProcessOutput {
            status: status.into(),
            ..process_output(false, "")
        };

        assert!(super::status_is_no_match(&output("exit status: 1")));
        for status in ["exit status: 128", "exit code: 10", "signal: 1 (SIGHUP)"] {
            assert!(!super::status_is_no_match(&output(status)), "{status}");
        }
    }

    #[test]
    fn project_health_git_missing_executable_is_typed_and_stops_discovery() {
        let fixture = health_fixture(false);
        let runner = SequenceRunner::errors(vec![
            "process_failed: No such file or directory (os error 2)",
        ]);
        let layout = inspect_layout(&fixture.context);

        let inspection = GitRepositoryInspector::with_runner(&runner)
            .inspect_base(
                &fixture.context,
                &layout,
                &CancellationToken::new(),
                ProviderDeadline::from_budget(Duration::from_secs(1)),
            )
            .unwrap();

        assert_eq!(runner.commands.borrow().len(), 1);
        assert!(matches!(
            inspection.facts.as_slice(),
            [ProjectHealthFact::GitExecutableUnavailable { reason }]
                if reason.contains("No such file")
        ));
        assert!(inspection.observations.iter().any(|observation| {
            observation.id == ProjectCheckId::RepositoryIndex
                && matches!(observation.outcome, ProjectCheckOutcome::NotRun { .. })
        }));
    }

    #[test]
    fn project_health_git_non_not_found_spawn_error_is_inspection_incomplete() {
        let fixture = health_fixture(false);
        let runner =
            SequenceRunner::errors(vec!["process_failed: Permission denied (os error 13)"]);
        let layout = inspect_layout(&fixture.context);

        let inspection = GitRepositoryInspector::with_runner(&runner)
            .inspect_base(
                &fixture.context,
                &layout,
                &CancellationToken::new(),
                ProviderDeadline::from_budget(Duration::from_secs(1)),
            )
            .unwrap();

        assert!(matches!(
            inspection.facts.as_slice(),
            [ProjectHealthFact::GitInspectionIncomplete {
                check: ProjectCheckId::RepositoryDiscovery,
                reason,
                ..
            }] if reason.contains("Permission denied")
        ));
    }

    #[test]
    fn project_health_git_index_error_after_cancellation_cancels_inspection() {
        let fixture = health_fixture(false);
        let cancellation = CancellationToken::new();
        let runner = LateCancellingIndexRunner {
            repository_root: fixture.context.workspace_root.clone(),
            cancellation: cancellation.clone(),
            calls: Cell::new(0),
        };
        let layout = inspect_layout(&fixture.context);

        let result = GitRepositoryInspector::with_runner(&runner).inspect_base(
            &fixture.context,
            &layout,
            &cancellation,
            ProviderDeadline::from_budget(Duration::from_secs(1)),
        );

        assert!(matches!(
            result,
            Err(ProjectHealthInspectionError::Cancelled)
        ));
    }

    #[test]
    fn project_health_git_error_after_cancellation_is_cancelled() {
        let fixture = health_fixture(false);
        let cancellation = CancellationToken::new();
        let runner = CancellingRunner {
            cancellation: cancellation.clone(),
        };
        let layout = inspect_layout(&fixture.context);

        let result = GitRepositoryInspector::with_runner(&runner).inspect_base(
            &fixture.context,
            &layout,
            &cancellation,
            ProviderDeadline::from_budget(Duration::from_secs(1)),
        );

        assert!(matches!(
            result,
            Err(crate::domain::project_health::ProjectHealthInspectionError::Cancelled)
        ));
    }

    #[test]
    fn project_health_git_absent_repository_is_typed_and_stops_after_discovery() {
        let fixture = health_fixture(false);
        let layout = inspect_layout(&fixture.context);

        let inspection = GitRepositoryInspector::new()
            .inspect_base(
                &fixture.context,
                &layout,
                &CancellationToken::new(),
                ProviderDeadline::from_budget(Duration::from_secs(2)),
            )
            .unwrap();

        assert!(matches!(
            inspection.facts.as_slice(),
            [ProjectHealthFact::GitRepositoryAbsent]
        ));
        assert!(inspection.repository_root.is_none());
        assert!(inspection.observations.iter().any(|observation| {
            observation.id == ProjectCheckId::RepositoryIndex
                && matches!(observation.outcome, ProjectCheckOutcome::NotRun { .. })
        }));
    }

    #[test]
    fn project_health_git_literal_replacement_character_is_not_lossy() {
        let fixture = health_fixture(false);
        let runner = SequenceRunner::outputs(vec![
            ProcessOutput {
                status_success: true,
                status: "exit status: 0".into(),
                stdout: format!("{}\ntrue\n", fixture.context.workspace_root.display()),
                stderr: "valid \u{fffd}".into(),
                timed_out: false,
                cancelled: false,
                stdout_truncated: false,
                stderr_truncated: false,
                stdout_had_invalid_utf8: false,
                stderr_had_invalid_utf8: false,
            },
            ProcessOutput {
                status_success: true,
                status: "exit status: 0".into(),
                stdout: String::new(),
                stderr: String::new(),
                timed_out: false,
                cancelled: false,
                stdout_truncated: false,
                stderr_truncated: false,
                stdout_had_invalid_utf8: false,
                stderr_had_invalid_utf8: false,
            },
            ProcessOutput {
                status: "exit status: 1".into(),
                ..process_output(false, "")
            },
            ProcessOutput {
                status_success: true,
                status: "exit status: 0".into(),
                stdout: String::new(),
                stderr: String::new(),
                timed_out: false,
                cancelled: false,
                stdout_truncated: false,
                stderr_truncated: false,
                stdout_had_invalid_utf8: false,
                stderr_had_invalid_utf8: false,
            },
            ProcessOutput {
                status: "exit status: 1".into(),
                ..process_output(false, "")
            },
        ]);
        let layout = inspect_layout(&fixture.context);

        let inspection = GitRepositoryInspector::with_runner(&runner)
            .inspect_base(
                &fixture.context,
                &layout,
                &CancellationToken::new(),
                ProviderDeadline::from_budget(Duration::from_secs(1)),
            )
            .unwrap();

        assert!(!inspection.facts.iter().any(|fact| matches!(
            fact,
            ProjectHealthFact::GitInspectionIncomplete {
                check: ProjectCheckId::RepositoryDiscovery,
                ..
            }
        )));
    }

    #[test]
    fn project_health_git_config_dump_timeout_is_incomplete_not_unclassified() {
        let fixture = health_fixture(false);
        let oid = "a".repeat(40);
        let runner = SequenceRunner::outputs(vec![
            process_output(
                true,
                &format!("{}\ntrue\n", fixture.context.workspace_root.display()),
            ),
            process_output(true, &format!("100644 {oid} 0\tsrc/ConfigDumpInfo.xml\0")),
            ProcessOutput {
                status: "exit status: 1".into(),
                ..process_output(false, "")
            },
            ProcessOutput {
                timed_out: true,
                ..process_output(false, "")
            },
            ProcessOutput {
                status: "exit status: 1".into(),
                ..process_output(false, "")
            },
            ProcessOutput {
                status: "exit status: 1".into(),
                ..process_output(false, "")
            },
        ]);
        let layout = inspect_layout(&fixture.context);

        let inspection = GitRepositoryInspector::with_runner(&runner)
            .inspect_base(
                &fixture.context,
                &layout,
                &CancellationToken::new(),
                ProviderDeadline::from_budget(Duration::from_secs(1)),
            )
            .unwrap();

        assert!(inspection.observations.iter().any(|observation| {
            observation.id == ProjectCheckId::RepositoryConfigDumpInfo
                && observation.source_set.is_none()
                && matches!(observation.outcome, ProjectCheckOutcome::NotRun { .. })
        }));
        assert!(inspection.observations.iter().any(|observation| {
            observation.id == ProjectCheckId::RepositoryConfigDumpInfo
                && observation.source_set.as_deref() == Some("main")
                && matches!(observation.outcome, ProjectCheckOutcome::NotRun { .. })
        }));
        assert!(inspection.facts.iter().any(|fact| matches!(
            fact,
            ProjectHealthFact::GitInspectionTimeout {
                check: ProjectCheckId::RepositoryConfigDumpInfo,
                ..
            }
        )));
        assert!(!inspection.facts.iter().any(|fact| matches!(
            fact,
            ProjectHealthFact::ConfigDumpInfoUnclassified { path, .. }
                if path == "src/ConfigDumpInfo.xml"
        )));
    }

    #[test]
    fn project_health_old_git_partial_clone_never_reads_staged_blobs() {
        let fixture = health_fixture(false);
        let oid = "a".repeat(40);
        let no_match = || ProcessOutput {
            status: "exit status: 1".into(),
            ..process_output(false, "")
        };
        let runner = SequenceRunner::outputs(vec![
            process_output(
                true,
                &format!("{}\ntrue\n", fixture.context.workspace_root.display()),
            ),
            process_output(true, &format!("100644 {oid} 0\tsrc/ConfigDumpInfo.xml\0")),
            process_output(true, "remote.origin.promisor\ntrue\0"),
            process_output(true, "git version 2.40.1\n"),
            no_match(),
            no_match(),
        ]);
        let layout = inspect_layout(&fixture.context);

        let inspection = GitRepositoryInspector::with_runner(&runner)
            .inspect_base(
                &fixture.context,
                &layout,
                &CancellationToken::new(),
                ProviderDeadline::from_budget(Duration::from_secs(1)),
            )
            .unwrap();

        assert!(inspection.observations.iter().any(|observation| {
            observation.id == ProjectCheckId::RepositoryConfigDumpInfo
                && matches!(observation.outcome, ProjectCheckOutcome::NotRun { .. })
        }));
        assert!(inspection.facts.iter().any(|fact| matches!(
            fact,
            ProjectHealthFact::GitInspectionIncomplete {
                check: ProjectCheckId::RepositoryConfigDumpInfo,
                reason,
                ..
            } if reason.contains("Git 2.40.1") && reason.contains("partial clone")
        )));
        assert!(
            runner
                .commands
                .borrow()
                .iter()
                .all(|command| { !command.args.iter().any(|argument| argument == "cat-file") }),
            "{:?}",
            runner.commands.borrow()
        );
        let config_command = &runner.commands.borrow()[2];
        assert!(!config_command.args.iter().any(|arg| arg == "--local"));
    }

    #[test]
    fn project_health_git_parent_repository_uses_tracked_portable_ignore() {
        let fixture = parent_repository_fixture();
        fs::write(
            fixture.repository_root.join(".gitignore"),
            "**/.build/\nConfigDumpInfo.xml\nDumpFilesIndex.txt\n",
        )
        .unwrap();
        git(&fixture.repository_root, &["add", ".gitignore"]);
        let layout = inspect_layout(&fixture.context);

        let inspection = GitRepositoryInspector::new()
            .inspect_base(
                &fixture.context,
                &layout,
                &CancellationToken::new(),
                ProviderDeadline::from_budget(Duration::from_secs(5)),
            )
            .unwrap();

        assert_eq!(
            inspection.repository_root.as_deref(),
            Some(
                crate::infrastructure::source_roots::normalize_path_identity(
                    &fixture.repository_root
                )
                .unwrap()
                .as_path()
            )
        );
        assert!(
            !inspection.facts.iter().any(|fact| matches!(
                fact,
                ProjectHealthFact::IgnoreRuleMissing { .. }
                    | ProjectHealthFact::IgnoreRuleLocalOnly { .. }
            )),
            "{:?}",
            inspection.facts
        );
        assert!(inspection.observations.iter().all(|observation| {
            observation.id != ProjectCheckId::RepositoryIgnore
                || matches!(observation.outcome, ProjectCheckOutcome::Completed)
        }));
    }

    #[test]
    fn project_health_git_discovers_repository_root_containing_newline() {
        if std::path::MAIN_SEPARATOR == '\\' {
            return;
        }
        let temp = TempDir::new().unwrap();
        let root = temp.path().join("repo\nname");
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/Configuration.xml"), "<MetaDataObject/>\n").unwrap();
        fs::write(
            root.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        git(&root, &["init"]);
        let context = WorkspaceContext {
            cwd: root.clone(),
            workspace_root: root.clone(),
            cache_root: root.join(".build/unica"),
            workspace_epoch: 1,
        };
        let layout = inspect_layout(&context);

        let inspection = GitRepositoryInspector::new()
            .inspect_base(
                &context,
                &layout,
                &CancellationToken::new(),
                ProviderDeadline::from_budget(Duration::from_secs(5)),
            )
            .unwrap();

        assert_eq!(
            inspection.repository_root,
            Some(fs::canonicalize(&root).unwrap())
        );

        fs::write(
            root.join(".gitattributes"),
            "*.xml text eol=lf\n*.bsl text eol=lf\n*.bin -text\n",
        )
        .unwrap();
        git(&root, &["add", ".gitattributes", "src/Configuration.xml"]);
        let inspection =
            crate::infrastructure::project_health::resources::SourceResourcePolicyInspector::new()
                .inspect(
                    inspection.repository_root.as_ref().unwrap(),
                    &layout.roots,
                    &GitRepositoryInspector::new()
                        .inspect_base(
                            &context,
                            &layout,
                            &CancellationToken::new(),
                            ProviderDeadline::from_budget(Duration::from_secs(5)),
                        )
                        .unwrap()
                        .entries,
                    &CancellationToken::new(),
                    ProviderDeadline::from_budget(Duration::from_secs(5)),
                )
                .unwrap();
        assert!(
            inspection.observations.iter().all(|observation| {
                observation.id != ProjectCheckId::RepositoryAttributes
                    || matches!(observation.outcome, ProjectCheckOutcome::Completed)
            }),
            "{:?}",
            inspection.facts
        );
    }

    #[test]
    fn project_health_git_info_exclude_is_local_only() {
        let fixture = parent_repository_fixture();
        fs::write(
            fixture.repository_root.join(".git/info/exclude"),
            "**/.build/\nConfigDumpInfo.xml\nDumpFilesIndex.txt\n",
        )
        .unwrap();
        let layout = inspect_layout(&fixture.context);

        let inspection = GitRepositoryInspector::new()
            .inspect_base(
                &fixture.context,
                &layout,
                &CancellationToken::new(),
                ProviderDeadline::from_budget(Duration::from_secs(5)),
            )
            .unwrap();

        assert!(
            inspection.facts.iter().any(|fact| matches!(
                fact,
                ProjectHealthFact::IgnoreRuleLocalOnly { origin, .. }
                    if origin.contains("exclude")
            )),
            "{:?}",
            inspection.facts
        );
    }

    #[test]
    fn project_health_git_ignore_uses_staged_empty_file_not_valid_worktree_file() {
        let fixture = parent_repository_fixture();
        fs::write(fixture.repository_root.join(".gitignore"), "").unwrap();
        git(&fixture.repository_root, &["add", ".gitignore"]);
        fs::write(
            fixture.repository_root.join(".gitignore"),
            "**/.build/\nConfigDumpInfo.xml\nDumpFilesIndex.txt\n",
        )
        .unwrap();
        let layout = inspect_layout(&fixture.context);

        let inspection = GitRepositoryInspector::new()
            .inspect_base(
                &fixture.context,
                &layout,
                &CancellationToken::new(),
                ProviderDeadline::from_budget(Duration::from_secs(5)),
            )
            .unwrap();

        assert!(
            inspection.facts.iter().any(|fact| matches!(
                fact,
                ProjectHealthFact::IgnoreRuleLocalOnly { path, .. }
                    if path == "workspace/src/ConfigDumpInfo.xml"
            )),
            "{:?}",
            inspection.facts
        );
    }

    #[test]
    fn project_health_git_global_excludes_cannot_impersonate_staged_gitignore() {
        let fixture = parent_repository_fixture();
        fs::write(fixture.repository_root.join(".gitignore"), "").unwrap();
        git(&fixture.repository_root, &["add", ".gitignore"]);
        fs::write(
            fixture.repository_root.join(".gitignore"),
            "**/.build/\nConfigDumpInfo.xml\nDumpFilesIndex.txt\n",
        )
        .unwrap();
        let excludes = fixture.repository_root.join(".gitignore");
        git(
            &fixture.repository_root,
            &["config", "core.excludesFile", excludes.to_str().unwrap()],
        );
        let layout = inspect_layout(&fixture.context);

        let inspection = GitRepositoryInspector::new()
            .inspect_base(
                &fixture.context,
                &layout,
                &CancellationToken::new(),
                ProviderDeadline::from_budget(Duration::from_secs(5)),
            )
            .unwrap();

        assert!(
            inspection.facts.iter().any(|fact| matches!(
                fact,
                ProjectHealthFact::IgnoreRuleLocalOnly { path, .. }
                    if path == "workspace/src/ConfigDumpInfo.xml"
            )),
            "{:?}",
            inspection.facts
        );
    }

    #[test]
    fn project_health_git_duplicate_local_ignore_does_not_mask_portable_staged_rule() {
        let fixture = parent_repository_fixture();
        fs::write(
            fixture.repository_root.join(".gitignore"),
            "**/.build/\nConfigDumpInfo.xml\nDumpFilesIndex.txt\n",
        )
        .unwrap();
        git(&fixture.repository_root, &["add", ".gitignore"]);
        fs::write(
            fixture.repository_root.join(".git/info/exclude"),
            "**/.build/\nConfigDumpInfo.xml\nDumpFilesIndex.txt\n",
        )
        .unwrap();
        let layout = inspect_layout(&fixture.context);

        let inspection = GitRepositoryInspector::new()
            .inspect_base(
                &fixture.context,
                &layout,
                &CancellationToken::new(),
                ProviderDeadline::from_budget(Duration::from_secs(5)),
            )
            .unwrap();

        assert!(
            !inspection
                .facts
                .iter()
                .any(|fact| matches!(fact, ProjectHealthFact::IgnoreRuleLocalOnly { .. })),
            "{:?}",
            inspection.facts
        );
    }

    #[test]
    fn project_health_git_ignore_uses_valid_staged_file_not_empty_worktree_file() {
        let fixture = parent_repository_fixture();
        fs::write(
            fixture.repository_root.join(".gitignore"),
            "**/.build/\nConfigDumpInfo.xml\nDumpFilesIndex.txt\n",
        )
        .unwrap();
        git(&fixture.repository_root, &["add", ".gitignore"]);
        fs::write(fixture.repository_root.join(".gitignore"), "").unwrap();
        let layout = inspect_layout(&fixture.context);

        let inspection = GitRepositoryInspector::new()
            .inspect_base(
                &fixture.context,
                &layout,
                &CancellationToken::new(),
                ProviderDeadline::from_budget(Duration::from_secs(5)),
            )
            .unwrap();

        assert!(
            !inspection.facts.iter().any(|fact| matches!(
                fact,
                ProjectHealthFact::IgnoreRuleMissing { path, .. }
                    | ProjectHealthFact::IgnoreRuleLocalOnly { path, .. }
                    if path == "workspace/src/ConfigDumpInfo.xml"
            )),
            "{:?}",
            inspection.facts
        );
    }

    #[test]
    fn project_health_staged_ignore_read_does_not_run_smudge_filter() {
        let fixture = parent_repository_fixture();
        fs::write(
            fixture.repository_root.join(".gitattributes"),
            ".gitignore filter=project-health-probe\n",
        )
        .unwrap();
        fs::write(
            fixture.repository_root.join(".gitignore"),
            "**/.build/\nConfigDumpInfo.xml\nDumpFilesIndex.txt\n",
        )
        .unwrap();
        git(
            &fixture.repository_root,
            &[
                "config",
                "filter.project-health-probe.smudge",
                "unica-project-health-smudge-command-must-not-run",
            ],
        );
        git(
            &fixture.repository_root,
            &["add", ".gitattributes", ".gitignore"],
        );
        let layout = inspect_layout(&fixture.context);

        let inspection = GitRepositoryInspector::new()
            .inspect_base(
                &fixture.context,
                &layout,
                &CancellationToken::new(),
                ProviderDeadline::from_budget(Duration::from_secs(5)),
            )
            .unwrap();

        assert!(
            !inspection.facts.iter().any(|fact| matches!(
                fact,
                ProjectHealthFact::IgnoreRuleMissing { .. }
                    | ProjectHealthFact::IgnoreRuleLocalOnly { .. }
            )),
            "{:?}",
            inspection.facts
        );
    }

    #[test]
    fn project_health_git_untracked_gitignore_is_local_only() {
        let fixture = parent_repository_fixture();
        fs::write(
            fixture.repository_root.join(".gitignore"),
            "**/.build/\nConfigDumpInfo.xml\nDumpFilesIndex.txt\n",
        )
        .unwrap();
        let layout = inspect_layout(&fixture.context);

        let inspection = GitRepositoryInspector::new()
            .inspect_base(
                &fixture.context,
                &layout,
                &CancellationToken::new(),
                ProviderDeadline::from_budget(Duration::from_secs(5)),
            )
            .unwrap();

        assert!(
            inspection.facts.iter().any(|fact| matches!(
                fact,
                ProjectHealthFact::IgnoreRuleLocalOnly { origin, .. }
                    if origin.contains(".gitignore")
            )),
            "{:?}",
            inspection.facts
        );
    }

    #[test]
    fn project_health_git_missing_ignore_rules_do_not_create_probe_files() {
        let fixture = parent_repository_fixture();
        let layout = inspect_layout(&fixture.context);

        let inspection = GitRepositoryInspector::new()
            .inspect_base(
                &fixture.context,
                &layout,
                &CancellationToken::new(),
                ProviderDeadline::from_budget(Duration::from_secs(5)),
            )
            .unwrap();

        assert!(
            inspection.facts.iter().any(|fact| matches!(
                fact,
                ProjectHealthFact::IgnoreRuleMissing { path, .. }
                    if path == "workspace/src/ConfigDumpInfo.xml"
            )),
            "{:?}",
            inspection.facts
        );
        assert!(!fixture
            .workspace_root
            .join("src/ConfigDumpInfo.xml")
            .exists());
        assert!(!fixture
            .workspace_root
            .join("src/.build/.unica-health-probe")
            .exists());
    }

    #[test]
    fn unknown_and_invalid_source_formats_do_not_pass_format_dependent_ignore() {
        for configured_format in [None, Some("UNSUPPORTED")] {
            let fixture = health_fixture(true);
            fs::remove_file(fixture.context.workspace_root.join("src/Configuration.xml")).unwrap();
            let format = configured_format
                .map(|value| format!("format: {value}\n"))
                .unwrap_or_default();
            fs::write(
                fixture.context.workspace_root.join("v8project.yaml"),
                format!(
                    "{format}source-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n"
                ),
            )
            .unwrap();
            fs::write(
                fixture
                    .context
                    .workspace_root
                    .join("src/ConfigDumpInfo.xml"),
                "<ConfigDumpInfo/>",
            )
            .unwrap();
            git(
                &fixture.context.workspace_root,
                &["add", "src/ConfigDumpInfo.xml"],
            );
            let layout = inspect_layout(&fixture.context);
            assert!(matches!(
                layout.roots[0].source_set.source_format,
                SourceFormat::Unknown | SourceFormat::Invalid
            ));

            let inspection = GitRepositoryInspector::new()
                .inspect_base(
                    &fixture.context,
                    &layout,
                    &CancellationToken::new(),
                    ProviderDeadline::from_budget(Duration::from_secs(5)),
                )
                .unwrap();

            assert!(
                inspection.observations.iter().any(|observation| {
                    observation.id == ProjectCheckId::RepositoryIgnore
                        && observation.source_set.as_deref() == Some("main")
                        && matches!(observation.outcome, ProjectCheckOutcome::NotRun { .. })
                }),
                "format={configured_format:?}; observations={:?}",
                inspection.observations
            );
            assert!(
                !inspection.facts.iter().any(|fact| matches!(
                    fact,
                    ProjectHealthFact::IgnoreRuleMissing { path, .. }
                        | ProjectHealthFact::IgnoreRuleLocalOnly { path, .. }
                        if path == "src/ConfigDumpInfo.xml"
                )),
                "format={configured_format:?}; facts={:?}",
                inspection.facts
            );
            let mut observations = layout
                .observations
                .iter()
                .cloned()
                .chain(inspection.observations.iter().cloned())
                .collect::<Vec<_>>();
            observations.extend(
                crate::infrastructure::project_health::resources::resource_observations(
                    layout.source_sets.iter().flatten(),
                    "source-set targets are incomplete",
                    layout.source_targets_complete,
                ),
            );
            let snapshot = crate::domain::project_health::ProjectHealthSnapshot {
                workspace_root: fixture.context.workspace_root.display().to_string(),
                cache_root: fixture.context.cache_root.display().to_string(),
                repository_root: inspection
                    .repository_root
                    .as_ref()
                    .map(|path| path.display().to_string()),
                source_sets: layout.source_sets.clone(),
                source_targets_complete: layout.source_targets_complete,
                observations,
                facts: layout
                    .facts
                    .iter()
                    .cloned()
                    .chain(inspection.facts.iter().cloned())
                    .collect(),
            };
            assert!(
                crate::domain::project_health::evaluate_project_health(snapshot).is_ok(),
                "format={configured_format:?}; facts={:?}",
                inspection.facts
            );
        }
    }

    #[test]
    fn incomplete_format_does_not_suppress_ignore_for_a_proven_sibling_set() {
        let fixture = health_fixture(true);
        fs::create_dir_all(fixture.context.workspace_root.join("unknown")).unwrap();
        fs::write(
            fixture.context.workspace_root.join("v8project.yaml"),
            "source-set:\n  - name: designer\n    type: CONFIGURATION\n    path: src\n  - name: unknown\n    type: CONFIGURATION\n    path: unknown\n",
        )
        .unwrap();
        fs::write(
            fixture.context.workspace_root.join(".gitignore"),
            "**/.build/\nConfigDumpInfo.xml\nDumpFilesIndex.txt\n",
        )
        .unwrap();
        git(&fixture.context.workspace_root, &["add", ".gitignore"]);
        let layout = inspect_layout(&fixture.context);

        let inspection = GitRepositoryInspector::new()
            .inspect_base(
                &fixture.context,
                &layout,
                &CancellationToken::new(),
                ProviderDeadline::from_budget(Duration::from_secs(5)),
            )
            .unwrap();

        assert!(
            inspection.observations.iter().any(|observation| {
                observation.id == ProjectCheckId::RepositoryIgnore
                    && observation.source_set.as_deref() == Some("designer")
                    && matches!(observation.outcome, ProjectCheckOutcome::Completed)
            }),
            "{:?}",
            inspection.observations
        );
        assert!(
            inspection.observations.iter().any(|observation| {
                observation.id == ProjectCheckId::RepositoryIgnore
                    && observation.source_set.as_deref() == Some("unknown")
                    && matches!(observation.outcome, ProjectCheckOutcome::NotRun { .. })
            }),
            "{:?}",
            inspection.observations
        );
        assert!(
            inspection.observations.iter().any(|observation| {
                observation.id == ProjectCheckId::RepositoryIgnore
                    && observation.source_set.is_none()
                    && matches!(observation.outcome, ProjectCheckOutcome::NotRun { .. })
            }),
            "{:?}",
            inspection.observations
        );
    }

    #[test]
    fn project_health_working_ignore_timeout_is_not_reported_as_a_missing_rule() {
        let fixture = health_fixture(true);
        let no_match = || ProcessOutput {
            status: "exit status: 1".into(),
            ..process_output(false, "")
        };
        let runner = SequenceRunner::outputs(vec![
            process_output(
                true,
                &format!("{}\ntrue\n", fixture.context.workspace_root.display()),
            ),
            process_output(true, ""),
            no_match(),
            no_match(),
            ProcessOutput {
                timed_out: true,
                ..process_output(false, "")
            },
        ]);
        let layout = inspect_layout(&fixture.context);

        let inspection = GitRepositoryInspector::with_runner(&runner)
            .inspect_base(
                &fixture.context,
                &layout,
                &CancellationToken::new(),
                ProviderDeadline::from_budget(Duration::from_secs(1)),
            )
            .unwrap();

        assert!(inspection
            .observations
            .iter()
            .filter(|observation| observation.id == ProjectCheckId::RepositoryIgnore)
            .all(|observation| matches!(observation.outcome, ProjectCheckOutcome::NotRun { .. })));
        assert!(inspection.facts.iter().any(|fact| matches!(
            fact,
            ProjectHealthFact::GitInspectionTimeout {
                check: ProjectCheckId::RepositoryIgnore,
                source_set: None,
            }
        )));
        assert!(!inspection.facts.iter().any(|fact| matches!(
            fact,
            ProjectHealthFact::IgnoreRuleMissing { .. }
                | ProjectHealthFact::IgnoreRuleLocalOnly { .. }
        )));
    }

    #[test]
    fn project_health_git_reports_missing_ignore_for_every_equal_root_owner() {
        let fixture = health_fixture(true);
        fs::write(
            fixture.context.workspace_root.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: first\n    type: CONFIGURATION\n    path: src\n  - name: second\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        let layout = inspect_layout(&fixture.context);

        let inspection = GitRepositoryInspector::new()
            .inspect_base(
                &fixture.context,
                &layout,
                &CancellationToken::new(),
                ProviderDeadline::from_budget(Duration::from_secs(5)),
            )
            .unwrap();

        for owner in ["first", "second"] {
            assert!(
                inspection.facts.iter().any(|fact| matches!(
                    fact,
                    ProjectHealthFact::IgnoreRuleMissing {
                        source_set: Some(source_set),
                        path,
                    } if source_set == owner && path == "src/ConfigDumpInfo.xml"
                )),
                "owner={owner}; facts={:?}",
                inspection.facts
            );
        }
    }

    #[test]
    fn project_health_git_runtime_sidecar_and_legitimate_descriptor_are_distinct() {
        let fixture = parent_repository_fixture();
        fs::write(
            fixture.workspace_root.join("src/ConfigDumpInfo.xml"),
            "<ConfigDumpInfo/>",
        )
        .unwrap();
        fs::create_dir_all(fixture.workspace_root.join("epf")).unwrap();
        fs::write(
            fixture.workspace_root.join("epf/ConfigDumpInfo.xml"),
            "<MetaDataObject><ExternalDataProcessor/></MetaDataObject>",
        )
        .unwrap();
        git(
            &fixture.repository_root,
            &[
                "add",
                "workspace/src/ConfigDumpInfo.xml",
                "workspace/epf/ConfigDumpInfo.xml",
            ],
        );
        let layout = inspect_layout(&fixture.context);

        let inspection = GitRepositoryInspector::new()
            .inspect_base(
                &fixture.context,
                &layout,
                &CancellationToken::new(),
                ProviderDeadline::from_budget(Duration::from_secs(5)),
            )
            .unwrap();

        assert!(
            inspection.facts.iter().any(|fact| matches!(
                fact,
                ProjectHealthFact::RuntimeSidecarTracked { path, .. }
                    if path == "workspace/src/ConfigDumpInfo.xml"
            )),
            "{:?}",
            inspection.facts
        );
        assert!(
            !inspection.facts.iter().any(|fact| matches!(
                fact,
                ProjectHealthFact::ConfigDumpInfoUnclassified { path, .. }
                    | ProjectHealthFact::RuntimeSidecarTracked { path, .. }
                    if path == "workspace/epf/ConfigDumpInfo.xml"
            )),
            "{:?}",
            inspection.facts
        );
    }

    #[test]
    fn project_health_git_tracked_generated_paths_are_independent_of_ignore_matches() {
        let fixture = parent_repository_fixture();
        let generated = fixture.workspace_root.join("src/.build/generated.bin");
        fs::create_dir_all(generated.parent().unwrap()).unwrap();
        fs::write(&generated, b"generated").unwrap();
        git(
            &fixture.repository_root,
            &["add", "-f", "workspace/src/.build/generated.bin"],
        );
        fs::write(
            fixture.repository_root.join(".gitignore"),
            "**/.build/\nConfigDumpInfo.xml\nDumpFilesIndex.txt\n",
        )
        .unwrap();
        git(&fixture.repository_root, &["add", ".gitignore"]);
        let layout = inspect_layout(&fixture.context);

        let inspection = GitRepositoryInspector::new()
            .inspect_base(
                &fixture.context,
                &layout,
                &CancellationToken::new(),
                ProviderDeadline::from_budget(Duration::from_secs(5)),
            )
            .unwrap();

        assert!(
            inspection.facts.iter().any(|fact| matches!(
                fact,
                ProjectHealthFact::GeneratedPathTracked { path, .. }
                    if path == "workspace/src/.build/generated.bin"
            )),
            "{:?}",
            inspection.facts
        );
    }

    struct SequenceRunner {
        commands: RefCell<Vec<ProcessCommand>>,
        results: RefCell<Vec<Result<ProcessOutput, String>>>,
    }

    struct CancellingRunner {
        cancellation: CancellationToken,
    }

    impl ProcessRunner for CancellingRunner {
        fn run(&self, _command: &ProcessCommand) -> Result<ProcessOutput, String> {
            self.cancellation.cancel();
            Err("process failed while cancellation was requested".into())
        }
    }

    impl SequenceRunner {
        fn outputs(outputs: Vec<ProcessOutput>) -> Self {
            Self {
                commands: RefCell::new(Vec::new()),
                results: RefCell::new(outputs.into_iter().map(Ok).collect()),
            }
        }

        fn errors(errors: Vec<&str>) -> Self {
            Self {
                commands: RefCell::new(Vec::new()),
                results: RefCell::new(errors.into_iter().map(|error| Err(error.into())).collect()),
            }
        }
    }

    impl ProcessRunner for SequenceRunner {
        fn run(&self, command: &ProcessCommand) -> Result<ProcessOutput, String> {
            self.commands.borrow_mut().push(command.clone());
            self.results.borrow_mut().remove(0)
        }

        fn run_with_input(
            &self,
            command: &ProcessCommand,
            _input: &[u8],
        ) -> Result<ProcessOutput, String> {
            self.run(command)
        }
    }

    fn process_output(status_success: bool, stdout: &str) -> ProcessOutput {
        ProcessOutput {
            status_success,
            status: if status_success {
                "exit status: 0".into()
            } else {
                "exit status: 2".into()
            },
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

    struct HealthFixture {
        _temp: TempDir,
        context: WorkspaceContext,
    }

    struct LateCancellingIndexRunner {
        repository_root: PathBuf,
        cancellation: CancellationToken,
        calls: Cell<usize>,
    }

    impl ProcessRunner for LateCancellingIndexRunner {
        fn run(&self, _command: &ProcessCommand) -> Result<ProcessOutput, String> {
            let call = self.calls.get();
            self.calls.set(call + 1);
            if call == 0 {
                Ok(process_output(
                    true,
                    &format!("{}\ntrue\n", self.repository_root.display()),
                ))
            } else {
                self.cancellation.cancel();
                Err("index process failed after cancellation".into())
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

    fn health_fixture(initialize_git: bool) -> HealthFixture {
        let temp = TempDir::new().unwrap();
        let root = temp.path().to_path_buf();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/Configuration.xml"), "<MetaDataObject/>").unwrap();
        fs::write(
            root.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        if initialize_git {
            git(&root, &["init"]);
        }
        HealthFixture {
            _temp: temp,
            context: WorkspaceContext {
                cwd: root.clone(),
                workspace_root: root.clone(),
                cache_root: root.join(".build/unica"),
                workspace_epoch: 1,
            },
        }
    }

    struct ParentRepositoryFixture {
        _temp: TempDir,
        repository_root: PathBuf,
        workspace_root: PathBuf,
        context: WorkspaceContext,
    }

    fn parent_repository_fixture() -> ParentRepositoryFixture {
        let temp = TempDir::new().unwrap();
        let repository_root = temp.path().to_path_buf();
        git(&repository_root, &["init"]);
        let workspace_root = repository_root.join("workspace");
        fs::create_dir_all(workspace_root.join("src")).unwrap();
        fs::write(
            workspace_root.join("src/Configuration.xml"),
            "<MetaDataObject/>",
        )
        .unwrap();
        fs::write(
            workspace_root.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
        )
        .unwrap();
        ParentRepositoryFixture {
            _temp: temp,
            repository_root: repository_root.clone(),
            workspace_root: workspace_root.clone(),
            context: WorkspaceContext {
                cwd: workspace_root.clone(),
                workspace_root: workspace_root.clone(),
                cache_root: workspace_root.join(".build/unica"),
                workspace_epoch: 1,
            },
        }
    }

    fn inspect_layout(
        context: &WorkspaceContext,
    ) -> crate::infrastructure::project_health::layout::SourceLayoutInspection {
        SourceLayoutInspector::inspect(
            context,
            &CancellationToken::new(),
            ProviderDeadline::from_budget(Duration::from_secs(2)),
        )
        .unwrap()
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
}
