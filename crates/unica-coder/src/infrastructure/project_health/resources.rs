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
use crate::infrastructure::project_health::git::GitIndexEntry;
use crate::infrastructure::project_health::layout::InspectedSourceRoot;
use crate::infrastructure::source_roots::normalize_path_identity;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

pub(crate) const LFS_SINGLE_FILE_THRESHOLD_BYTES: u64 = 10 * 1024 * 1024;
pub(crate) const LFS_AGGREGATE_THRESHOLD_BYTES: u64 = 100 * 1024 * 1024;
pub(crate) const MAX_WORKING_EOL_FILE_BYTES: u64 = 32 * 1024 * 1024;
pub(crate) const MAX_WORKING_EOL_TOTAL_BYTES: u64 = 256 * 1024 * 1024;

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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResourceOwnershipError {
    pub(crate) repo_path: String,
    pub(crate) source_sets: Vec<String>,
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
    ) -> Result<Vec<RepositoryResource>, ResourceOwnershipError> {
        let mut prefixes = Vec::<(&InspectedSourceRoot, String)>::new();
        for root in roots
            .iter()
            .filter(|root| root.source_set.source_format == SourceFormat::PlatformXml)
        {
            if let Some(prefix) = repo_path(repository_root, &root.path) {
                prefixes.push((root, prefix));
            }
        }
        let mut resources = Vec::new();
        for entry in entries {
            let mut owners = prefixes
                .iter()
                .filter(|(_, prefix)| path_is_within(&entry.repo_path, prefix))
                .collect::<Vec<_>>();
            let Some(max_depth) = owners.iter().map(|(_, prefix)| path_depth(prefix)).max() else {
                continue;
            };
            owners.retain(|(_, prefix)| path_depth(prefix) == max_depth);
            if owners.len() > 1 {
                let mut source_sets = owners
                    .iter()
                    .map(|(root, _)| root.source_set.name.clone())
                    .collect::<Vec<_>>();
                source_sets.sort();
                source_sets.dedup();
                return Err(ResourceOwnershipError {
                    repo_path: entry.repo_path.clone(),
                    source_sets,
                });
            }
            let (root, prefix) = owners[0];
            let relative = entry
                .repo_path
                .strip_prefix(prefix.as_str())
                .unwrap_or_default()
                .trim_start_matches('/');
            if let Some(kind) = classify_platform_xml_relative_path(relative) {
                resources.push(RepositoryResource {
                    source_set: root.source_set.name.clone(),
                    repo_path: entry.repo_path.clone(),
                    worktree_path: repository_root.join(&entry.repo_path),
                    kind,
                });
            }
        }
        resources.sort_by(|left, right| left.repo_path.cmp(&right.repo_path));
        Ok(resources)
    }

    pub(crate) fn inspect(
        &self,
        repository_root: &Path,
        roots: &[InspectedSourceRoot],
        entries: &[GitIndexEntry],
        cancellation: &CancellationToken,
        deadline: ProviderDeadline,
    ) -> Result<RepositoryPolicyInspection, ProjectHealthInspectionError> {
        if cancellation.is_cancelled() {
            return Err(ProjectHealthInspectionError::Cancelled);
        }
        let observations = resource_observations(roots);
        let resources = match Self::classify(repository_root, roots, entries) {
            Ok(resources) => resources,
            Err(error) => {
                let reason = format!(
                    "resource {} has ambiguous source-set owners: {}",
                    error.repo_path,
                    error.source_sets.join(", ")
                );
                return Ok(RepositoryPolicyInspection {
                    observations,
                    facts: resource_check_ids()
                        .into_iter()
                        .map(|check| ProjectHealthFact::GitInspectionIncomplete {
                            check,
                            source_set: None,
                            reason: reason.clone(),
                        })
                        .collect(),
                });
            }
        };
        if resources.is_empty() {
            return Ok(RepositoryPolicyInspection {
                observations,
                facts: Vec::new(),
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
                match parse_attribute_records(&output.stdout, &paths) {
                    Ok(records) => records,
                    Err(reason) => {
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

        let empty_tree = match self.run_with_input(
            repository_root,
            ["hash-object", "-t", "tree", "--stdin"],
            &[],
            cancellation,
            deadline,
        ) {
            Ok(output) if semantic_success(&output) => output.stdout.trim().to_string(),
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
        if empty_tree.is_empty() || !empty_tree.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Ok(incomplete_policy(
                observations,
                ProjectCheckId::RepositoryAttributes,
                "Git returned an invalid empty-tree object id".into(),
            ));
        }
        let source_arg = format!("--source={empty_tree}");
        let local = match self.run_with_input_vec(
            repository_root,
            vec![
                "check-attr".into(),
                "-z".into(),
                source_arg,
                "text".into(),
                "eol".into(),
                "filter".into(),
                "--stdin".into(),
            ],
            &input,
            cancellation,
            deadline,
        ) {
            Ok(output) if semantic_success(&output) => {
                match parse_attribute_records(&output.stdout, &paths) {
                    Ok(records) => records,
                    Err(reason) => {
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

        let mut facts = Vec::new();
        let mut local_only = BTreeSet::new();
        let mut text_policy_missing = BTreeSet::new();
        for resource in &resources {
            let attributes = &effective[&resource.repo_path];
            let local_attributes = &local[&resource.repo_path];
            if attribute_is_local(local_attributes) {
                local_only.insert(resource.repo_path.clone());
                facts.push(ProjectHealthFact::AttributesLocalOnly {
                    source_set: resource.source_set.clone(),
                    path: resource.repo_path.clone(),
                    evidence: vec![format!(
                        "local-only text={} eol={} filter={}",
                        local_attributes.text, local_attributes.eol, local_attributes.filter
                    )],
                });
                continue;
            }
            match resource.kind {
                RepositoryResourceKind::Text if attributes.text == "unset" => {
                    facts.push(ProjectHealthFact::TextResourceMarkedBinary {
                        source_set: resource.source_set.clone(),
                        path: resource.repo_path.clone(),
                    });
                }
                RepositoryResourceKind::Text if !text_policy_satisfied(attributes) => {
                    text_policy_missing.insert(resource.repo_path.clone());
                    facts.push(ProjectHealthFact::TextPolicyMissing {
                        source_set: resource.source_set.clone(),
                        path: resource.repo_path.clone(),
                    });
                }
                RepositoryResourceKind::Binary if attributes.text != "unset" => {
                    facts.push(ProjectHealthFact::BinaryPolicyMissing {
                        source_set: resource.source_set.clone(),
                        path: resource.repo_path.clone(),
                    });
                }
                RepositoryResourceKind::Text | RepositoryResourceKind::Binary => {}
            }
        }

        let eol_output = match self.run(
            repository_root,
            ["ls-files", "--eol", "-z"],
            cancellation,
            deadline,
        ) {
            Ok(output) if semantic_success(&output) => output,
            Ok(output) if output.cancelled => return Err(ProjectHealthInspectionError::Cancelled),
            Ok(output) => {
                facts.push(ProjectHealthFact::GitInspectionIncomplete {
                    check: ProjectCheckId::RepositoryIndexEol,
                    source_set: None,
                    reason: output_reason(&output),
                });
                return Ok(RepositoryPolicyInspection {
                    observations,
                    facts,
                });
            }
            Err(reason) => {
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
        let eol = match parse_eol_records(&eol_output.stdout) {
            Ok(records) => records,
            Err(reason) => {
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
        let mut total_working_bytes = 0_u64;
        let mut working_incomplete = None;
        let mut binary_by_source = BTreeMap::<String, Vec<(String, u64)>>::new();
        for resource in &resources {
            if local_only.contains(&resource.repo_path) {
                continue;
            }
            let attributes = &effective[&resource.repo_path];
            match resource.kind {
                RepositoryResourceKind::Text => {
                    let Some(observed) = eol.get(&resource.repo_path) else {
                        facts.push(ProjectHealthFact::GitInspectionIncomplete {
                            check: ProjectCheckId::RepositoryIndexEol,
                            source_set: None,
                            reason: format!("git ls-files --eol omitted {}", resource.repo_path),
                        });
                        continue;
                    };
                    if !text_policy_missing.contains(&resource.repo_path)
                        && !matches!(observed.index.as_str(), "lf" | "none")
                    {
                        facts.push(ProjectHealthFact::IndexEolNotLf {
                            source_set: resource.source_set.clone(),
                            path: resource.repo_path.clone(),
                            observed: observed.index.clone(),
                        });
                    }
                    if cancellation.is_cancelled() {
                        return Err(ProjectHealthInspectionError::Cancelled);
                    }
                    if deadline.remaining().is_zero() {
                        working_incomplete.get_or_insert_with(|| {
                            "deadline expired during working EOL inspection".to_string()
                        });
                        continue;
                    }
                    match inspect_working_eol(&resource.worktree_path, &mut total_working_bytes) {
                        Ok(Some(WorkingEol::Mixed)) => {
                            facts.push(ProjectHealthFact::MixedEol {
                                source_set: resource.source_set.clone(),
                                path: resource.repo_path.clone(),
                            });
                        }
                        Ok(Some(WorkingEol::BareCr)) => {
                            facts.push(ProjectHealthFact::WorkingEolUnsupported {
                                source_set: resource.source_set.clone(),
                                path: resource.repo_path.clone(),
                                observed: "cr".into(),
                            });
                        }
                        Ok(Some(WorkingEol::Supported) | None) => {}
                        Err(reason) => {
                            working_incomplete
                                .get_or_insert_with(|| format!("{}: {reason}", resource.repo_path));
                        }
                    }
                }
                RepositoryResourceKind::Binary => {
                    if attributes.filter == "lfs" {
                        continue;
                    }
                    match regular_file_size(&resource.worktree_path) {
                        Ok(Some(size)) => binary_by_source
                            .entry(resource.source_set.clone())
                            .or_default()
                            .push((resource.repo_path.clone(), size)),
                        Ok(None) => {}
                        Err(reason) => {
                            facts.push(ProjectHealthFact::GitInspectionIncomplete {
                                check: ProjectCheckId::RepositoryLfs,
                                source_set: None,
                                reason: format!("{}: {reason}", resource.repo_path),
                            });
                        }
                    }
                }
            }
        }
        if let Some(reason) = working_incomplete {
            facts.push(ProjectHealthFact::GitInspectionIncomplete {
                check: ProjectCheckId::RepositoryWorkingEol,
                source_set: None,
                reason,
            });
        }
        for (source_set, mut files) in binary_by_source {
            files.sort_by(|left, right| left.0.cmp(&right.0));
            let total_bytes = files
                .iter()
                .fold(0_u64, |total, (_, size)| total.saturating_add(*size));
            let largest = files.iter().max_by_key(|(_, size)| *size);
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
                    paths: files.into_iter().map(|(path, _)| path).collect(),
                });
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
        self.runner.run(&process_command(
            cwd,
            args.into_iter().map(str::to_owned).collect(),
            cancellation,
            deadline,
        ))
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
        self.runner
            .run_with_input(&process_command(cwd, args, cancellation, deadline), input)
    }
}

pub(crate) fn classify_platform_xml_relative_path(
    normalized_relative_path: &str,
) -> Option<RepositoryResourceKind> {
    let normalized = normalized_relative_path.replace('\\', "/");
    let segments = normalized.split('/').collect::<Vec<_>>();
    if segments.len() == 4
        && segments[0] == "XDTOPackages"
        && !segments[1].is_empty()
        && segments[2] == "Ext"
        && segments[3] == "Package.bin"
    {
        return Some(RepositoryResourceKind::Text);
    }
    let extension = Path::new(&normalized)
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
    let fields = nul_fields(stdout, "Git check-attr")?;
    if fields.len() % 3 != 0 {
        return Err("Git check-attr output is not a sequence of triples".into());
    }
    let expected = expected_paths.iter().collect::<BTreeSet<_>>();
    let mut raw = BTreeMap::<String, BTreeMap<String, String>>::new();
    for fields in fields.chunks_exact(3) {
        if !expected.contains(&fields[0]) {
            return Err(format!(
                "Git check-attr returned unexpected path {}",
                fields[0]
            ));
        }
        let attributes = raw.entry(fields[0].clone()).or_default();
        if attributes
            .insert(fields[1].clone(), fields[2].clone())
            .is_some()
        {
            return Err("Git check-attr returned a duplicate attribute".into());
        }
    }
    let mut records = BTreeMap::new();
    for path in expected_paths {
        let attributes = raw
            .remove(path)
            .ok_or_else(|| format!("Git check-attr omitted {path}"))?;
        if attributes.len() != 3 {
            return Err(format!(
                "Git check-attr returned incomplete attributes for {path}"
            ));
        }
        records.insert(
            path.clone(),
            AttributeValues {
                text: required_attribute(&attributes, "text", path)?,
                eol: required_attribute(&attributes, "eol", path)?,
                filter: required_attribute(&attributes, "filter", path)?,
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

fn parse_eol_records(stdout: &str) -> Result<BTreeMap<String, EolValues>, String> {
    let records = nul_fields(stdout, "git ls-files --eol")?;
    let mut result = BTreeMap::new();
    for record in records {
        let (metadata, path) = record
            .split_once('\t')
            .ok_or_else(|| "git ls-files --eol record has no tab separator".to_string())?;
        if path.is_empty() {
            return Err("git ls-files --eol record has an empty path".into());
        }
        let fields = metadata.split_whitespace().collect::<Vec<_>>();
        if fields.len() < 2 || !fields[0].starts_with("i/") || !fields[1].starts_with("w/") {
            return Err("git ls-files --eol record has invalid metadata".into());
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
            return Err("git ls-files --eol returned a duplicate path".into());
        }
    }
    Ok(result)
}

fn nul_fields(stdout: &str, command: &str) -> Result<Vec<String>, String> {
    if !stdout.is_empty() && !stdout.ends_with('\0') {
        return Err(format!("{command} output is missing its terminal NUL"));
    }
    Ok(stdout.split_terminator('\0').map(str::to_owned).collect())
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
        ],
        timeout: Some(deadline.remaining()),
        cancellation: cancellation.clone(),
    }
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
    observations: Vec<ProjectCheckObservation>,
    check: ProjectCheckId,
    reason: String,
) -> RepositoryPolicyInspection {
    RepositoryPolicyInspection {
        observations,
        facts: vec![ProjectHealthFact::GitInspectionIncomplete {
            check,
            source_set: None,
            reason,
        }],
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

fn resource_observations(roots: &[InspectedSourceRoot]) -> Vec<ProjectCheckObservation> {
    let mut source_sets = roots
        .iter()
        .filter(|root| root.source_set.source_format == SourceFormat::PlatformXml)
        .map(|root| root.source_set.name.clone())
        .collect::<Vec<_>>();
    source_sets.sort();
    source_sets.dedup();
    if source_sets.is_empty() {
        return resource_check_ids()
            .into_iter()
            .map(|id| ProjectCheckObservation {
                id,
                scope: id.scope(),
                source_set: None,
                outcome: ProjectCheckOutcome::NotApplicable {
                    reason: "no Platform XML source roots were proven".into(),
                },
            })
            .collect();
    }
    let mut observations = Vec::new();
    for id in resource_check_ids() {
        observations.push(ProjectCheckObservation {
            id,
            scope: id.scope(),
            source_set: None,
            outcome: ProjectCheckOutcome::Completed,
        });
        observations.extend(
            source_sets
                .iter()
                .map(|source_set| ProjectCheckObservation {
                    id,
                    scope: id.scope(),
                    source_set: Some(source_set.clone()),
                    outcome: ProjectCheckOutcome::Completed,
                }),
        );
    }
    observations
}

fn text_policy_satisfied(attributes: &AttributeValues) -> bool {
    matches!(attributes.text.as_str(), "set" | "auto")
        || matches!(attributes.eol.as_str(), "lf" | "crlf")
}

fn attribute_is_local(attributes: &AttributeValues) -> bool {
    attributes.text != "unspecified"
        || attributes.eol != "unspecified"
        || attributes.filter != "unspecified"
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
    Some(relative.to_string_lossy().replace('\\', "/"))
}

fn path_is_within(path: &str, prefix: &str) -> bool {
    prefix.is_empty()
        || path == prefix
        || path
            .strip_prefix(prefix)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

fn path_depth(path: &str) -> usize {
    path.split('/')
        .filter(|segment| !segment.is_empty())
        .count()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorkingEol {
    Supported,
    Mixed,
    BareCr,
}

fn inspect_working_eol(
    path: &Path,
    total_working_bytes: &mut u64,
) -> Result<Option<WorkingEol>, String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("working path is not a regular file".into());
    }
    if metadata.len() > MAX_WORKING_EOL_FILE_BYTES {
        return Err(format!(
            "working file exceeds {} bytes",
            MAX_WORKING_EOL_FILE_BYTES
        ));
    }
    *total_working_bytes = total_working_bytes.saturating_add(metadata.len());
    if *total_working_bytes > MAX_WORKING_EOL_TOTAL_BYTES {
        return Err(format!(
            "working text total exceeds {} bytes",
            MAX_WORKING_EOL_TOTAL_BYTES
        ));
    }
    let mut file = File::open(path).map_err(|error| error.to_string())?;
    let mut buffer = [0_u8; 64 * 1024];
    let mut previous_cr = false;
    let mut lf = false;
    let mut crlf = false;
    let mut bare_cr = false;
    loop {
        let count = file.read(&mut buffer).map_err(|error| error.to_string())?;
        if count == 0 {
            break;
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
    let styles = usize::from(lf) + usize::from(crlf) + usize::from(bare_cr);
    Ok(Some(if styles > 1 {
        WorkingEol::Mixed
    } else if bare_cr {
        WorkingEol::BareCr
    } else {
        WorkingEol::Supported
    }))
}

fn regular_file_size(path: &Path) -> Result<Option<u64>, String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("binary working path is not a regular file".into());
    }
    Ok(Some(metadata.len()))
}

#[cfg(test)]
mod tests {
    use super::{
        classify_platform_xml_relative_path, parse_attribute_records, parse_eol_records,
        RepositoryResourceKind, SourceResourcePolicyInspector, LFS_SINGLE_FILE_THRESHOLD_BYTES,
    };
    use crate::domain::cancellation::CancellationToken;
    use crate::domain::code_intelligence::ProviderDeadline;
    use crate::domain::project_health::{ProjectCheckId, ProjectHealthFact};
    use crate::domain::workspace::WorkspaceContext;
    use crate::infrastructure::project_health::git::GitRepositoryInspector;
    use crate::infrastructure::project_health::layout::SourceLayoutInspector;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::time::Duration;
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

        let eol = parse_eol_records("i/lf w/crlf attr/text eol=crlf\tsrc/A.xml\0").unwrap();
        assert_eq!(eol["src/A.xml"].index, "lf");
        assert_eq!(eol["src/A.xml"].worktree, "crlf");
        assert!(parse_eol_records("i/lf w/lf without-tab\0").is_err());
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
}
