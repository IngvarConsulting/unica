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
use crate::infrastructure::platform::filesystem::path_starts_with_host_root;
use crate::infrastructure::project_health::layout::{InspectedSourceRoot, SourceLayoutInspection};
use crate::infrastructure::source_roots::normalize_path_identity;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GitIndexEntry {
    pub(crate) repo_path: String,
    pub(crate) blob_oid: Option<String>,
    pub(crate) mode: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct IgnoreMatch {
    source: String,
    line: String,
    pattern: String,
    path: String,
}

pub(crate) struct GitRepositoryInspection {
    pub(crate) repository_root: Option<PathBuf>,
    pub(crate) entries: Vec<GitIndexEntry>,
    pub(crate) observations: Vec<ProjectCheckObservation>,
    pub(crate) facts: Vec<ProjectHealthFact>,
}

pub(crate) struct ConfigDumpInfoIndexInspection {
    pub(crate) runtime_paths: Vec<String>,
    pub(crate) inconclusive_paths: Vec<String>,
}

pub(crate) enum ConfigDumpInfoIndexInspectionError {
    Cancelled,
}

pub(crate) struct GitRepositoryInspector<'a> {
    runner: &'a dyn ProcessRunner,
}

#[derive(Debug, Clone)]
struct IgnoreCandidate {
    source_set: Option<String>,
    repo_path: String,
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
            Err(error) if error.starts_with("process_failed:") => {
                return Ok(discovery_failed(
                    ProjectHealthFact::GitExecutableUnavailable { reason: error },
                    "Git executable is unavailable",
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
                ));
            }
            return Ok(discovery_failed(
                ProjectHealthFact::GitInspectionIncomplete {
                    check: ProjectCheckId::RepositoryDiscovery,
                    source_set: None,
                    reason: nonzero_reason(&discovery),
                },
                "Git discovery failed",
            ));
        }
        let mut discovery_lines = discovery.stdout.lines();
        let root_text = discovery_lines.next().unwrap_or_default().trim();
        let inside = discovery_lines.next().unwrap_or_default().trim();
        if inside == "false" {
            return Ok(discovery_failed(
                ProjectHealthFact::GitRepositoryAbsent,
                "Workspace is not inside a Git work tree",
            ));
        }
        if root_text.is_empty() || inside != "true" || discovery_lines.any(|line| !line.is_empty())
        {
            return Ok(discovery_failed(
                ProjectHealthFact::GitInspectionIncomplete {
                    check: ProjectCheckId::RepositoryDiscovery,
                    source_set: None,
                    reason: "git rev-parse returned an unrecognized response".into(),
                },
                "Git discovery returned an unrecognized response",
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
            Err(reason) => {
                observations.push(not_run(ProjectCheckId::RepositoryIndex, &reason));
                facts.push(ProjectHealthFact::GitInspectionIncomplete {
                    check: ProjectCheckId::RepositoryIndex,
                    source_set: None,
                    reason,
                });
                append_not_run_after_index(&mut observations, "Git index inspection failed");
                return Ok(GitRepositoryInspection {
                    repository_root: Some(repository_root),
                    entries: Vec::new(),
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
            append_not_run_after_index(&mut observations, "Git index inspection timed out");
            return Ok(GitRepositoryInspection {
                repository_root: Some(repository_root),
                entries: Vec::new(),
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
            append_not_run_after_index(&mut observations, "Git index output is incomplete");
            return Ok(GitRepositoryInspection {
                repository_root: Some(repository_root),
                entries: Vec::new(),
                observations,
                facts,
            });
        }
        let entries = match parse_git_index_entries(&index.stdout) {
            Ok(entries) => entries,
            Err(reason) => {
                observations.push(not_run(ProjectCheckId::RepositoryIndex, &reason));
                facts.push(ProjectHealthFact::GitInspectionIncomplete {
                    check: ProjectCheckId::RepositoryIndex,
                    source_set: None,
                    reason,
                });
                append_not_run_after_index(&mut observations, "Git index records are malformed");
                return Ok(GitRepositoryInspection {
                    repository_root: Some(repository_root),
                    entries: Vec::new(),
                    observations,
                    facts,
                });
            }
        };
        observations.push(completed(ProjectCheckId::RepositoryIndex));

        observations.push(completed(ProjectCheckId::RepositoryGeneratedPaths));
        append_completed_for_roots(
            &mut observations,
            ProjectCheckId::RepositoryGeneratedPaths,
            &layout.roots,
        );
        append_tracked_generated_facts(
            &mut facts,
            &entries,
            &repository_root,
            context,
            &layout.roots,
        );

        observations.push(completed(ProjectCheckId::RepositoryConfigDumpInfo));
        append_completed_for_roots(
            &mut observations,
            ProjectCheckId::RepositoryConfigDumpInfo,
            &layout.roots,
        );
        self.append_config_dump_info_facts(
            &mut facts,
            &entries,
            &repository_root,
            &layout.roots,
            cancellation,
            deadline,
        )?;

        if !layout.source_targets_complete {
            observations.push(not_run(
                ProjectCheckId::RepositoryIgnore,
                "source-set targets are incomplete",
            ));
            return Ok(GitRepositoryInspection {
                repository_root: Some(repository_root),
                entries,
                observations,
                facts,
            });
        }
        let candidates = ignore_candidates(&repository_root, context, &layout.roots);
        let mut input = Vec::new();
        for candidate in &candidates {
            input.extend_from_slice(candidate.repo_path.as_bytes());
            input.push(0);
        }
        let ignore_command = process_command(
            &repository_root,
            ["check-ignore", "-v", "-z", "--no-index", "--stdin"],
            cancellation,
            deadline,
        );
        let ignore = match self.runner.run_with_input(&ignore_command, &input) {
            Ok(output) => output,
            Err(reason) => {
                observations.push(not_run(ProjectCheckId::RepositoryIgnore, &reason));
                facts.push(ProjectHealthFact::GitInspectionIncomplete {
                    check: ProjectCheckId::RepositoryIgnore,
                    source_set: None,
                    reason,
                });
                return Ok(GitRepositoryInspection {
                    repository_root: Some(repository_root),
                    entries,
                    observations,
                    facts,
                });
            }
        };
        if ignore.cancelled || cancellation.is_cancelled() {
            return Err(ProjectHealthInspectionError::Cancelled);
        }
        if ignore.timed_out {
            observations.push(not_run(
                ProjectCheckId::RepositoryIgnore,
                "Git ignore inspection timed out",
            ));
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
            facts.push(ProjectHealthFact::GitInspectionIncomplete {
                check: ProjectCheckId::RepositoryIgnore,
                source_set: None,
                reason,
            });
        } else {
            match parse_check_ignore_verbose_z(&ignore.stdout) {
                Ok(matches) => {
                    observations.push(completed(ProjectCheckId::RepositoryIgnore));
                    append_completed_for_roots(
                        &mut observations,
                        ProjectCheckId::RepositoryIgnore,
                        &layout.roots,
                    );
                    append_ignore_facts(
                        &mut facts,
                        &candidates,
                        &matches,
                        &entries,
                        &repository_root,
                    );
                }
                Err(reason) => {
                    observations.push(not_run(ProjectCheckId::RepositoryIgnore, &reason));
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
            observations,
            facts,
        })
    }

    fn append_config_dump_info_facts(
        &self,
        facts: &mut Vec<ProjectHealthFact>,
        entries: &[GitIndexEntry],
        repository_root: &Path,
        roots: &[InspectedSourceRoot],
        cancellation: &CancellationToken,
        deadline: ProviderDeadline,
    ) -> Result<(), ProjectHealthInspectionError> {
        let inspection = classify_staged_config_dump_info(
            self.runner,
            repository_root,
            entries,
            cancellation,
            deadline,
        )
        .map_err(|ConfigDumpInfoIndexInspectionError::Cancelled| {
            ProjectHealthInspectionError::Cancelled
        })?;
        for path in inspection.runtime_paths {
            if let Some(source_set) = source_set_for_repo_path(repository_root, roots, &path) {
                facts.push(ProjectHealthFact::RuntimeSidecarTracked {
                    source_set: source_set.to_owned(),
                    path,
                });
            } else {
                facts.push(ProjectHealthFact::ConfigDumpInfoUnclassified {
                    source_set: None,
                    path,
                    reason: "runtime sidecar is outside a proven source root".into(),
                });
            }
        }
        for path in inspection.inconclusive_paths {
            facts.push(ProjectHealthFact::ConfigDumpInfoUnclassified {
                source_set: source_set_for_repo_path(repository_root, roots, &path)
                    .map(str::to_owned),
                path,
                reason: "staged blob classification is inconclusive".into(),
            });
        }
        Ok(())
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
        self.runner
            .run(&process_command(cwd, args, cancellation, deadline))
    }
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
    for entry in entries.iter().filter(|entry| {
        Path::new(&entry.repo_path)
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.eq_ignore_ascii_case("ConfigDumpInfo.xml"))
    }) {
        if cancellation.is_cancelled() {
            return Err(ConfigDumpInfoIndexInspectionError::Cancelled);
        }
        let Some(oid) = entry.blob_oid.as_ref() else {
            inconclusive_paths.push(entry.repo_path.clone());
            continue;
        };
        if deadline.remaining().is_zero() {
            inconclusive_paths.push(entry.repo_path.clone());
            continue;
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
            let classified = match runner.run(&command) {
                Ok(output) => {
                    if output.cancelled || cancellation.is_cancelled() {
                        return Err(ConfigDumpInfoIndexInspectionError::Cancelled);
                    }
                    (!output.timed_out && output.status_success && !output_incomplete(&output))
                        .then(|| config_dump_info_xml_kind(output.stdout.as_bytes()))
                }
                Err(_) if cancellation.is_cancelled() => {
                    return Err(ConfigDumpInfoIndexInspectionError::Cancelled);
                }
                Err(_) => None,
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
    ProcessCommand {
        program: PathBuf::from("git"),
        args: args.into_iter().map(str::to_owned).collect(),
        cwd: cwd.to_path_buf(),
        env: vec![
            (OsString::from("LC_ALL"), OsString::from("C")),
            (OsString::from("LANG"), OsString::from("C")),
        ],
        timeout: Some(deadline.remaining()),
        cancellation: cancellation.clone(),
    }
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
    output.stdout.is_empty() && output.stderr.is_empty() && output.status.contains(": 1")
}

fn completed(id: ProjectCheckId) -> ProjectCheckObservation {
    ProjectCheckObservation {
        id,
        scope: id.scope(),
        source_set: None,
        outcome: ProjectCheckOutcome::Completed,
    }
}

fn append_completed_for_roots(
    observations: &mut Vec<ProjectCheckObservation>,
    id: ProjectCheckId,
    roots: &[InspectedSourceRoot],
) {
    let mut source_sets = roots
        .iter()
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

fn discovery_failed(fact: ProjectHealthFact, reason: &str) -> GitRepositoryInspection {
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
    }
    GitRepositoryInspection {
        repository_root: None,
        entries: Vec::new(),
        observations,
        facts: vec![fact],
    }
}

fn append_not_run_after_index(observations: &mut Vec<ProjectCheckObservation>, reason: &str) {
    for id in [
        ProjectCheckId::RepositoryIgnore,
        ProjectCheckId::RepositoryGeneratedPaths,
        ProjectCheckId::RepositoryConfigDumpInfo,
    ] {
        observations.push(not_run(id, reason));
    }
}

pub(crate) fn parse_git_index_entries(stdout: &str) -> Result<Vec<GitIndexEntry>, String> {
    #[derive(Default)]
    struct State {
        records: usize,
        blob_oid: Option<String>,
        mode: Option<String>,
    }
    if !stdout.is_empty() && !stdout.ends_with('\0') {
        return Err("Git index output is missing its terminal NUL".into());
    }
    let mut entries = BTreeMap::<String, State>::new();
    for record in stdout.split_terminator('\0') {
        let (metadata, path) = record
            .split_once('\t')
            .ok_or_else(|| "Git index record has no tab separator".to_string())?;
        if path.is_empty() {
            return Err("Git index record has an empty path".into());
        }
        let fields = metadata.split_whitespace().collect::<Vec<_>>();
        if fields.len() != 3 {
            return Err("Git index record has invalid metadata".into());
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
    if !stdout.is_empty() && !stdout.ends_with('\0') {
        return Err("Git check-ignore output is missing its terminal NUL".into());
    }
    let fields = stdout.split_terminator('\0').collect::<Vec<_>>();
    if fields.len() % 4 != 0 {
        return Err("Git check-ignore output is not a sequence of quadruples".into());
    }
    Ok(fields
        .chunks_exact(4)
        .map(|fields| IgnoreMatch {
            source: fields[0].into(),
            line: fields[1].into(),
            pattern: fields[2].into(),
            path: fields[3].into(),
        })
        .collect())
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
    candidates.dedup_by(|left, right| left.repo_path == right.repo_path);
    candidates
}

fn append_ignore_facts(
    facts: &mut Vec<ProjectHealthFact>,
    candidates: &[IgnoreCandidate],
    matches: &[IgnoreMatch],
    entries: &[GitIndexEntry],
    repository_root: &Path,
) {
    let matches_by_path = matches
        .iter()
        .map(|matched| (matched.path.as_str(), matched))
        .collect::<BTreeMap<_, _>>();
    let tracked = entries
        .iter()
        .map(|entry| entry.repo_path.as_str())
        .collect::<BTreeSet<_>>();
    for candidate in candidates {
        let Some(matched) = matches_by_path.get(candidate.repo_path.as_str()) else {
            facts.push(ProjectHealthFact::IgnoreRuleMissing {
                source_set: candidate.source_set.clone(),
                path: candidate.repo_path.clone(),
            });
            continue;
        };
        let portable_source = portable_ignore_source(repository_root, &matched.source);
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
}

fn portable_ignore_source(repository_root: &Path, source: &str) -> Option<String> {
    let source_path = Path::new(source);
    let repo_path = if source_path.is_absolute() {
        repo_path(repository_root, source_path)?
    } else {
        normalize_git_path(source)
    };
    (Path::new(&repo_path)
        .file_name()
        .and_then(|name| name.to_str())
        == Some(".gitignore")
        && !repo_path.starts_with(".git/"))
    .then_some(repo_path)
}

fn append_tracked_generated_facts(
    facts: &mut Vec<ProjectHealthFact>,
    entries: &[GitIndexEntry],
    repository_root: &Path,
    context: &WorkspaceContext,
    roots: &[InspectedSourceRoot],
) {
    let cache = repo_path(repository_root, &context.cache_root);
    for entry in entries {
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
                source_set: source_set_for_repo_path(repository_root, roots, &entry.repo_path)
                    .map(str::to_owned),
                path: entry.repo_path.clone(),
            });
        }
    }
}

fn source_set_for_repo_path<'a>(
    repository_root: &Path,
    roots: &'a [InspectedSourceRoot],
    path: &str,
) -> Option<&'a str> {
    let absolute = repository_root.join(path);
    roots
        .iter()
        .filter(|root| path_starts_with_host_root(&absolute, &root.path))
        .max_by_key(|root| root.path.components().count())
        .map(|root| root.source_set.name.as_str())
}

fn repo_path(repository_root: &Path, path: &Path) -> Option<String> {
    let path = normalize_path_identity(path).ok()?;
    let relative = path.strip_prefix(repository_root).ok()?;
    Some(normalize_git_path(&relative.to_string_lossy()))
}

fn normalize_git_path(path: &str) -> String {
    path.replace('\\', "/").trim_start_matches("./").into()
}

#[cfg(test)]
mod tests {
    use super::{
        parse_check_ignore_verbose_z, parse_git_index_entries, GitRepositoryInspector, IgnoreMatch,
    };
    use crate::domain::cancellation::CancellationToken;
    use crate::domain::code_intelligence::ProviderDeadline;
    use crate::domain::project_health::{ProjectCheckId, ProjectCheckOutcome, ProjectHealthFact};
    use crate::domain::workspace::WorkspaceContext;
    use crate::infrastructure::internal_adapters::{ProcessCommand, ProcessOutput, ProcessRunner};
    use crate::infrastructure::project_health::layout::SourceLayoutInspector;
    use std::cell::RefCell;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::time::Duration;
    use tempfile::TempDir;

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
    fn project_health_git_missing_executable_is_typed_and_stops_discovery() {
        let fixture = health_fixture(false);
        let runner = SequenceRunner::errors(vec!["process_failed: git missing"]);
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
                if reason.contains("git missing")
        ));
        assert!(inspection.observations.iter().any(|observation| {
            observation.id == ProjectCheckId::RepositoryIndex
                && matches!(observation.outcome, ProjectCheckOutcome::NotRun { .. })
        }));
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

    struct HealthFixture {
        _temp: TempDir,
        context: WorkspaceContext,
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
