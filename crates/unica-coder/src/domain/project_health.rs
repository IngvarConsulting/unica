use crate::domain::project_sources::ProjectSourceSet;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

pub(crate) const MAX_PROJECT_DIAGNOSTIC_PATHS: usize = 20;
pub(crate) const MAX_PROJECT_DIAGNOSTIC_EVIDENCE: usize = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum DiagnosticScope {
    Workspace,
    Repository,
    SourceSet,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum DiagnosticSeverity {
    Error,
    Warning,
    Info,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum ProjectCheckStatus {
    Passed,
    Failed,
    NotRun,
    NotApplicable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum ProjectCheckId {
    SourceDiscovery,
    SourceLayout,
    SourceFormat,
    SourceGeneratedPaths,
    RepositoryDiscovery,
    RepositoryIndex,
    RepositoryIgnore,
    RepositoryGeneratedPaths,
    RepositoryConfigDumpInfo,
    RepositoryAttributes,
    RepositoryIndexEol,
    RepositoryWorkingEol,
    RepositoryLfs,
}

impl ProjectCheckId {
    pub(crate) const ALL: [Self; 13] = [
        Self::SourceDiscovery,
        Self::SourceLayout,
        Self::SourceFormat,
        Self::SourceGeneratedPaths,
        Self::RepositoryDiscovery,
        Self::RepositoryIndex,
        Self::RepositoryIgnore,
        Self::RepositoryGeneratedPaths,
        Self::RepositoryConfigDumpInfo,
        Self::RepositoryAttributes,
        Self::RepositoryIndexEol,
        Self::RepositoryWorkingEol,
        Self::RepositoryLfs,
    ];

    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::SourceDiscovery => "source.discovery",
            Self::SourceLayout => "source.layout",
            Self::SourceFormat => "source.format",
            Self::SourceGeneratedPaths => "source.generated_paths",
            Self::RepositoryDiscovery => "repository.discovery",
            Self::RepositoryIndex => "repository.index",
            Self::RepositoryIgnore => "repository.ignore",
            Self::RepositoryGeneratedPaths => "repository.generated_paths",
            Self::RepositoryConfigDumpInfo => "repository.config_dump_info",
            Self::RepositoryAttributes => "repository.attributes",
            Self::RepositoryIndexEol => "repository.index_eol",
            Self::RepositoryWorkingEol => "repository.working_eol",
            Self::RepositoryLfs => "repository.lfs",
        }
    }

    pub(crate) const fn scope(self) -> DiagnosticScope {
        match self {
            Self::SourceDiscovery => DiagnosticScope::Workspace,
            Self::SourceLayout | Self::SourceFormat | Self::SourceGeneratedPaths => {
                DiagnosticScope::SourceSet
            }
            Self::RepositoryDiscovery
            | Self::RepositoryIndex
            | Self::RepositoryIgnore
            | Self::RepositoryGeneratedPaths
            | Self::RepositoryConfigDumpInfo
            | Self::RepositoryAttributes
            | Self::RepositoryIndexEol
            | Self::RepositoryWorkingEol
            | Self::RepositoryLfs => DiagnosticScope::Repository,
        }
    }

    const fn is_source_scoped(self) -> bool {
        matches!(
            self,
            Self::SourceLayout | Self::SourceFormat | Self::SourceGeneratedPaths
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ProjectCheckOutcome {
    Completed,
    NotRun { reason: String },
    NotApplicable { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProjectCheckObservation {
    pub(crate) id: ProjectCheckId,
    pub(crate) scope: DiagnosticScope,
    pub(crate) source_set: Option<String>,
    pub(crate) outcome: ProjectCheckOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RemediationCommand {
    pub(crate) program: String,
    pub(crate) args: Vec<String>,
    pub(crate) cwd: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Remediation {
    pub(crate) summary: String,
    pub(crate) steps: Vec<String>,
    pub(crate) commands: Vec<RemediationCommand>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProjectCheck {
    pub(crate) id: String,
    pub(crate) scope: DiagnosticScope,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) source_set: Option<String>,
    pub(crate) status: ProjectCheckStatus,
    pub(crate) reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProjectDiagnostic {
    pub(crate) code: String,
    pub(crate) severity: DiagnosticSeverity,
    pub(crate) scope: DiagnosticScope,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) source_set: Option<String>,
    pub(crate) paths: Vec<String>,
    pub(crate) count: usize,
    pub(crate) message: String,
    pub(crate) evidence: Vec<String>,
    pub(crate) remediation: Remediation,
}

pub(crate) struct ProjectHealthSnapshot {
    pub(crate) workspace_root: String,
    pub(crate) cache_root: String,
    pub(crate) repository_root: Option<String>,
    pub(crate) source_sets: Option<Vec<ProjectSourceSet>>,
    pub(crate) source_targets_complete: bool,
    pub(crate) observations: Vec<ProjectCheckObservation>,
    pub(crate) facts: Vec<ProjectHealthFact>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ProjectHealthInspectionError {
    Cancelled,
    Fatal(String),
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProjectHealthReport {
    pub(crate) workspace_root: String,
    pub(crate) cache_root: String,
    pub(crate) ready: bool,
    pub(crate) repository_ready: bool,
    pub(crate) checks: Vec<ProjectCheck>,
    pub(crate) source_sets: Option<Vec<ProjectSourceSet>>,
    pub(crate) diagnostics: Vec<ProjectDiagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ProjectHealthFact {
    SourceInspectionIncomplete {
        reason: String,
    },
    NoSourceSets,
    SourceRootIsWorkspace {
        source_set: String,
        path: String,
        evidence: Vec<String>,
    },
    SourcePathMissing {
        source_set: String,
        path: String,
    },
    SourcePathUnsafe {
        source_set: String,
        path: String,
        reason: String,
    },
    SourceNameAmbiguous {
        name: String,
        count: usize,
    },
    SourceFormatInvalid {
        source_set: String,
        evidence: Vec<String>,
    },
    SourceFormatUnknown {
        source_set: String,
        evidence: Vec<String>,
    },
    CacheInsideSourceSet {
        source_set: String,
        source_root: String,
        cache_root: String,
    },
    GeneratedBuildPresent {
        source_set: String,
        path: String,
    },
    GitRepositoryAbsent,
    GitExecutableUnavailable {
        reason: String,
    },
    GitInspectionTimeout {
        check: ProjectCheckId,
        source_set: Option<String>,
    },
    GitInspectionIncomplete {
        check: ProjectCheckId,
        source_set: Option<String>,
        reason: String,
    },
    IgnoreRuleMissing {
        source_set: Option<String>,
        path: String,
    },
    IgnoreRuleLocalOnly {
        source_set: Option<String>,
        path: String,
        origin: String,
    },
    GeneratedPathTracked {
        source_set: Option<String>,
        path: String,
    },
    RuntimeSidecarTracked {
        source_set: String,
        path: String,
    },
    ConfigDumpInfoUnclassified {
        source_set: Option<String>,
        path: String,
        reason: String,
    },
    AttributesLocalOnly {
        source_set: String,
        path: String,
        evidence: Vec<String>,
    },
    TextPolicyMissing {
        source_set: String,
        path: String,
    },
    BinaryPolicyMissing {
        source_set: String,
        path: String,
    },
    TextResourceMarkedBinary {
        source_set: String,
        path: String,
    },
    IndexEolNotLf {
        source_set: String,
        path: String,
        observed: String,
    },
    MixedEol {
        source_set: String,
        path: String,
    },
    WorkingEolUnsupported {
        source_set: String,
        path: String,
        observed: String,
    },
    LfsConsider {
        source_set: String,
        count: usize,
        total_bytes: u64,
        largest_path: String,
        largest_bytes: u64,
        single_threshold_bytes: u64,
        aggregate_threshold_bytes: u64,
        paths: Vec<String>,
    },
}

pub(crate) fn evaluate_project_health(
    snapshot: ProjectHealthSnapshot,
) -> Result<ProjectHealthReport, String> {
    validate_snapshot(&snapshot)?;
    let mut groups = BTreeMap::<DiagnosticGroupKey, DiagnosticGroup>::new();
    let mut failed_checks = BTreeMap::<CheckKey, String>::new();
    for fact in &snapshot.facts {
        let seed = diagnostic_seed(fact);
        let key = DiagnosticGroupKey {
            code: seed.code,
            severity: seed.severity,
            scope: seed.check.scope,
            source_set: seed.check.source_set.clone(),
            message: seed.message.clone(),
        };
        let group = groups.entry(key).or_insert_with(|| DiagnosticGroup {
            paths: Vec::new(),
            count: 0,
            evidence: Vec::new(),
            remediation_kind: seed.remediation_kind,
        });
        group.count = group.count.saturating_add(seed.count);
        group.paths.extend(seed.paths);
        group.evidence.extend(seed.evidence);
        if seed.severity == DiagnosticSeverity::Error {
            failed_checks
                .entry(seed.check)
                .or_insert_with(|| seed.message);
        }
    }
    let mut diagnostics = groups
        .into_iter()
        .map(|(key, mut group)| {
            group.paths.sort();
            group.paths.dedup();
            group.evidence.sort();
            group.evidence.dedup();
            let remediation = remediation_for(
                group.remediation_kind,
                group.count,
                &group.paths,
                snapshot.repository_root.as_deref(),
            );
            group.paths.truncate(MAX_PROJECT_DIAGNOSTIC_PATHS);
            group.evidence.truncate(MAX_PROJECT_DIAGNOSTIC_EVIDENCE);
            ProjectDiagnostic {
                code: key.code.into(),
                severity: key.severity,
                scope: key.scope,
                source_set: key.source_set,
                paths: group.paths,
                count: group.count,
                message: key.message,
                evidence: group.evidence,
                remediation,
            }
        })
        .collect::<Vec<_>>();
    diagnostics.sort_by(|left, right| {
        (
            severity_rank(left.severity),
            left.scope,
            left.source_set.as_deref(),
            left.code.as_str(),
            left.paths.first().map(String::as_str),
        )
            .cmp(&(
                severity_rank(right.severity),
                right.scope,
                right.source_set.as_deref(),
                right.code.as_str(),
                right.paths.first().map(String::as_str),
            ))
    });
    let mut checks = snapshot
        .observations
        .iter()
        .map(|observation| {
            let key = CheckKey::from_observation(observation);
            let (status, reason) = match &observation.outcome {
                ProjectCheckOutcome::Completed if failed_checks.contains_key(&key) => {
                    (ProjectCheckStatus::Failed, failed_checks.get(&key).cloned())
                }
                ProjectCheckOutcome::Completed => (ProjectCheckStatus::Passed, None),
                ProjectCheckOutcome::NotRun { reason } => {
                    (ProjectCheckStatus::NotRun, Some(reason.clone()))
                }
                ProjectCheckOutcome::NotApplicable { reason } => {
                    (ProjectCheckStatus::NotApplicable, Some(reason.clone()))
                }
            };
            ProjectCheck {
                id: observation.id.as_str().into(),
                scope: observation.scope,
                source_set: observation.source_set.clone(),
                status,
                reason,
            }
        })
        .collect::<Vec<_>>();
    checks.sort_by(|left, right| {
        (left.scope, left.id.as_str(), left.source_set.as_deref()).cmp(&(
            right.scope,
            right.id.as_str(),
            right.source_set.as_deref(),
        ))
    });
    let ready = !checks.iter().any(|check| {
        matches!(
            check.scope,
            DiagnosticScope::Workspace | DiagnosticScope::SourceSet
        ) && matches!(
            check.status,
            ProjectCheckStatus::Failed | ProjectCheckStatus::NotRun
        )
    });
    let repository_ready = !checks.iter().any(|check| {
        check.scope == DiagnosticScope::Repository
            && check.id != ProjectCheckId::RepositoryLfs.as_str()
            && matches!(
                check.status,
                ProjectCheckStatus::Failed | ProjectCheckStatus::NotRun
            )
    });
    Ok(ProjectHealthReport {
        workspace_root: snapshot.workspace_root,
        cache_root: snapshot.cache_root,
        ready,
        repository_ready,
        checks,
        source_sets: snapshot.source_sets,
        diagnostics,
    })
}

fn incomplete_fact_reason(fact: &ProjectHealthFact) -> Option<&str> {
    match fact {
        ProjectHealthFact::GitInspectionTimeout { .. } => Some("Git inspection timed out"),
        ProjectHealthFact::GitInspectionIncomplete { reason, .. } => Some(reason),
        ProjectHealthFact::SourceInspectionIncomplete { reason } => Some(reason),
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct CheckKey {
    id: ProjectCheckId,
    scope: DiagnosticScope,
    source_set: Option<String>,
}

impl CheckKey {
    fn from_observation(observation: &ProjectCheckObservation) -> Self {
        Self {
            id: observation.id,
            scope: observation.scope,
            source_set: observation.source_set.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct DiagnosticGroupKey {
    code: &'static str,
    severity: DiagnosticSeverity,
    scope: DiagnosticScope,
    source_set: Option<String>,
    message: String,
}

struct DiagnosticGroup {
    paths: Vec<String>,
    count: usize,
    evidence: Vec<String>,
    remediation_kind: RemediationKind,
}

struct DiagnosticSeed {
    code: &'static str,
    severity: DiagnosticSeverity,
    check: CheckKey,
    paths: Vec<String>,
    count: usize,
    message: String,
    evidence: Vec<String>,
    remediation_kind: RemediationKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RemediationKind {
    Explain,
    RuntimeSidecar,
    ManualReview,
    Advisory,
}

fn validate_snapshot(snapshot: &ProjectHealthSnapshot) -> Result<(), String> {
    let mut observation_keys = BTreeSet::new();
    for observation in &snapshot.observations {
        if observation.scope != observation.id.scope() {
            return snapshot_error(format!(
                "{} has scope {:?}, expected {:?}",
                observation.id.as_str(),
                observation.scope,
                observation.id.scope()
            ));
        }
        if observation.id.is_source_scoped() && observation.source_set.is_none() {
            return snapshot_error(format!("{} requires sourceSet", observation.id.as_str()));
        }
        let key = CheckKey::from_observation(observation);
        if !observation_keys.insert(key) {
            return snapshot_error(format!(
                "duplicate observation for {}",
                observation.id.as_str()
            ));
        }
    }
    for required in [
        ProjectCheckId::SourceDiscovery,
        ProjectCheckId::RepositoryDiscovery,
        ProjectCheckId::RepositoryIndex,
        ProjectCheckId::RepositoryIgnore,
        ProjectCheckId::RepositoryGeneratedPaths,
        ProjectCheckId::RepositoryConfigDumpInfo,
        ProjectCheckId::RepositoryAttributes,
        ProjectCheckId::RepositoryIndexEol,
        ProjectCheckId::RepositoryWorkingEol,
        ProjectCheckId::RepositoryLfs,
    ] {
        if !snapshot
            .observations
            .iter()
            .any(|observation| observation.id == required)
        {
            return snapshot_error(format!("missing observation for {}", required.as_str()));
        }
    }
    if snapshot.source_targets_complete {
        let Some(source_sets) = snapshot.source_sets.as_ref() else {
            return snapshot_error(
                "source targets are complete but sourceSets are unavailable".into(),
            );
        };
        for source_set in source_sets {
            for required in [
                ProjectCheckId::SourceLayout,
                ProjectCheckId::SourceFormat,
                ProjectCheckId::SourceGeneratedPaths,
            ] {
                if !snapshot.observations.iter().any(|observation| {
                    observation.id == required
                        && observation.source_set.as_deref() == Some(source_set.name.as_str())
                }) {
                    return snapshot_error(format!(
                        "missing {} observation for source set {}",
                        required.as_str(),
                        source_set.name
                    ));
                }
            }
        }
    }
    for fact in &snapshot.facts {
        if let ProjectHealthFact::GitInspectionTimeout { check, .. }
        | ProjectHealthFact::GitInspectionIncomplete { check, .. } = fact
        {
            if check.scope() != DiagnosticScope::Repository {
                return snapshot_error(format!(
                    "git.inspection_timeout cannot target non-repository check {}",
                    check.as_str()
                ));
            }
        }
        let seed = diagnostic_seed(fact);
        let Some(observation) = snapshot
            .observations
            .iter()
            .find(|observation| CheckKey::from_observation(observation) == seed.check)
        else {
            return snapshot_error(format!(
                "fact {} has no matching {} observation",
                seed.code,
                seed.check.id.as_str()
            ));
        };
        let outcome_matches_fact = if incomplete_fact_reason(fact).is_some() {
            matches!(observation.outcome, ProjectCheckOutcome::NotRun { .. })
        } else {
            matches!(observation.outcome, ProjectCheckOutcome::Completed)
        };
        if !outcome_matches_fact {
            return snapshot_error(format!(
                "fact {} has an incompatible outcome for check {}",
                seed.code,
                seed.check.id.as_str()
            ));
        }
        if seed.remediation_kind == RemediationKind::RuntimeSidecar
            && snapshot.repository_root.is_none()
        {
            return snapshot_error(
                "git.runtime_sidecar_tracked requires a proven repository root".into(),
            );
        }
    }
    Ok(())
}

fn snapshot_error<T>(reason: String) -> Result<T, String> {
    Err(format!("project_health_snapshot_invalid: {reason}"))
}

fn diagnostic_seed(fact: &ProjectHealthFact) -> DiagnosticSeed {
    use ProjectHealthFact::*;
    match fact {
        SourceInspectionIncomplete { reason } => seed(
            "source_set.inspection_incomplete",
            DiagnosticSeverity::Error,
            ProjectCheckId::SourceDiscovery,
            None,
            Vec::new(),
            1,
            "Source-set configuration could not be inspected completely",
            vec![reason.clone()],
            RemediationKind::Explain,
        ),
        NoSourceSets => seed(
            "source_set.none_found",
            DiagnosticSeverity::Error,
            ProjectCheckId::SourceDiscovery,
            None,
            Vec::new(),
            1,
            "No source sets were discovered",
            Vec::new(),
            RemediationKind::Explain,
        ),
        SourceRootIsWorkspace {
            source_set,
            path,
            evidence,
        } => seed(
            "source_set.root_is_workspace",
            DiagnosticSeverity::Error,
            ProjectCheckId::SourceLayout,
            Some(source_set.clone()),
            vec![path.clone()],
            1,
            "Source root resolves to the workspace root",
            evidence.clone(),
            RemediationKind::Explain,
        ),
        SourcePathMissing { source_set, path } => seed(
            "source_set.path_missing",
            DiagnosticSeverity::Error,
            ProjectCheckId::SourceLayout,
            Some(source_set.clone()),
            vec![path.clone()],
            1,
            "Declared source root does not exist",
            Vec::new(),
            RemediationKind::Explain,
        ),
        SourcePathUnsafe {
            source_set,
            path,
            reason,
        } => seed(
            "source_set.path_unsafe",
            DiagnosticSeverity::Error,
            ProjectCheckId::SourceLayout,
            Some(source_set.clone()),
            vec![path.clone()],
            1,
            "Declared source root is not safely contained in the workspace",
            vec![reason.clone()],
            RemediationKind::Explain,
        ),
        SourceNameAmbiguous { name, count } => seed(
            "source_set.name_ambiguous",
            DiagnosticSeverity::Error,
            ProjectCheckId::SourceDiscovery,
            None,
            Vec::new(),
            *count,
            "Source-set name is not unique",
            vec![format!("name: {name}")],
            RemediationKind::Explain,
        ),
        SourceFormatInvalid {
            source_set,
            evidence,
        } => seed(
            "source_set.format_invalid",
            DiagnosticSeverity::Error,
            ProjectCheckId::SourceFormat,
            Some(source_set.clone()),
            Vec::new(),
            1,
            "Source format evidence is contradictory",
            evidence.clone(),
            RemediationKind::Explain,
        ),
        SourceFormatUnknown {
            source_set,
            evidence,
        } => seed(
            "source_set.format_unknown",
            DiagnosticSeverity::Error,
            ProjectCheckId::SourceFormat,
            Some(source_set.clone()),
            Vec::new(),
            1,
            "Source format could not be proven",
            evidence.clone(),
            RemediationKind::Explain,
        ),
        CacheInsideSourceSet {
            source_set,
            source_root,
            cache_root,
        } => seed(
            "cache.inside_source_set",
            DiagnosticSeverity::Error,
            ProjectCheckId::SourceLayout,
            Some(source_set.clone()),
            vec![cache_root.clone()],
            1,
            "Cache root is inside the source root",
            vec![format!("source root: {source_root}")],
            RemediationKind::Explain,
        ),
        GeneratedBuildPresent { source_set, path } => seed(
            "source_set.generated_build_present",
            DiagnosticSeverity::Error,
            ProjectCheckId::SourceGeneratedPaths,
            Some(source_set.clone()),
            vec![path.clone()],
            1,
            "Generated .build content is present inside the source root",
            Vec::new(),
            RemediationKind::Explain,
        ),
        GitRepositoryAbsent => seed(
            "git.repository_absent",
            DiagnosticSeverity::Error,
            ProjectCheckId::RepositoryDiscovery,
            None,
            Vec::new(),
            1,
            "Workspace is not inside a Git work tree",
            Vec::new(),
            RemediationKind::Explain,
        ),
        GitExecutableUnavailable { reason } => seed(
            "git.executable_unavailable",
            DiagnosticSeverity::Error,
            ProjectCheckId::RepositoryDiscovery,
            None,
            Vec::new(),
            1,
            "Git executable is unavailable",
            vec![reason.clone()],
            RemediationKind::Explain,
        ),
        GitInspectionTimeout { check, source_set } => seed(
            "git.inspection_timeout",
            incomplete_severity(*check),
            *check,
            source_set.clone(),
            Vec::new(),
            1,
            "Git inspection exceeded its deadline",
            vec![format!("check: {}", check.as_str())],
            RemediationKind::Explain,
        ),
        GitInspectionIncomplete {
            check,
            source_set,
            reason,
        } => seed(
            "git.inspection_incomplete",
            incomplete_severity(*check),
            *check,
            source_set.clone(),
            Vec::new(),
            1,
            "Git inspection did not produce a complete trustworthy result",
            vec![reason.clone()],
            RemediationKind::Explain,
        ),
        IgnoreRuleMissing { source_set, path } => seed(
            "git.ignore_rule_missing",
            DiagnosticSeverity::Error,
            ProjectCheckId::RepositoryIgnore,
            source_set.clone(),
            vec![path.clone()],
            1,
            "Required generated path is not covered by a tracked .gitignore",
            Vec::new(),
            RemediationKind::Explain,
        ),
        IgnoreRuleLocalOnly {
            source_set,
            path,
            origin,
        } => seed(
            "git.ignore_rule_local_only",
            DiagnosticSeverity::Error,
            ProjectCheckId::RepositoryIgnore,
            source_set.clone(),
            vec![path.clone()],
            1,
            "Required generated path is ignored only by a local rule",
            vec![format!("origin: {origin}")],
            RemediationKind::Explain,
        ),
        GeneratedPathTracked { source_set, path } => seed(
            "git.generated_path_tracked",
            DiagnosticSeverity::Error,
            ProjectCheckId::RepositoryGeneratedPaths,
            source_set.clone(),
            vec![path.clone()],
            1,
            "Generated path is tracked in the Git index",
            Vec::new(),
            RemediationKind::ManualReview,
        ),
        RuntimeSidecarTracked { source_set, path } => seed(
            "git.runtime_sidecar_tracked",
            DiagnosticSeverity::Error,
            ProjectCheckId::RepositoryConfigDumpInfo,
            Some(source_set.clone()),
            vec![path.clone()],
            1,
            "Platform-generated ConfigDumpInfo.xml is tracked",
            vec!["staged root element: ConfigDumpInfo".into()],
            RemediationKind::RuntimeSidecar,
        ),
        ConfigDumpInfoUnclassified {
            source_set,
            path,
            reason,
        } => seed(
            "git.config_dump_info_unclassified",
            DiagnosticSeverity::Error,
            ProjectCheckId::RepositoryConfigDumpInfo,
            source_set.clone(),
            vec![path.clone()],
            1,
            "Tracked ConfigDumpInfo.xml could not be classified safely",
            vec![reason.clone()],
            RemediationKind::ManualReview,
        ),
        AttributesLocalOnly {
            source_set,
            path,
            evidence,
        } => seed(
            "git.attributes_local_only",
            DiagnosticSeverity::Error,
            ProjectCheckId::RepositoryAttributes,
            Some(source_set.clone()),
            vec![path.clone()],
            1,
            "Required attributes are supplied only by local Git policy",
            evidence.clone(),
            RemediationKind::Explain,
        ),
        TextPolicyMissing { source_set, path } => seed(
            "git.text_policy_missing",
            DiagnosticSeverity::Error,
            ProjectCheckId::RepositoryAttributes,
            Some(source_set.clone()),
            vec![path.clone()],
            1,
            "Text resource has no portable text attributes",
            Vec::new(),
            RemediationKind::Explain,
        ),
        BinaryPolicyMissing { source_set, path } => seed(
            "git.binary_policy_missing",
            DiagnosticSeverity::Error,
            ProjectCheckId::RepositoryAttributes,
            Some(source_set.clone()),
            vec![path.clone()],
            1,
            "Binary resource has no portable -text attribute",
            Vec::new(),
            RemediationKind::Explain,
        ),
        TextResourceMarkedBinary { source_set, path } => seed(
            "git.text_resource_marked_binary",
            DiagnosticSeverity::Error,
            ProjectCheckId::RepositoryAttributes,
            Some(source_set.clone()),
            vec![path.clone()],
            1,
            "Text resource is marked as binary",
            Vec::new(),
            RemediationKind::Explain,
        ),
        IndexEolNotLf {
            source_set,
            path,
            observed,
        } => seed(
            "git.index_eol_not_lf",
            DiagnosticSeverity::Error,
            ProjectCheckId::RepositoryIndexEol,
            Some(source_set.clone()),
            vec![path.clone()],
            1,
            "Text blob in the Git index is not normalized to LF",
            vec![format!("observed: {observed}")],
            RemediationKind::Explain,
        ),
        MixedEol { source_set, path } => seed(
            "git.mixed_eol",
            DiagnosticSeverity::Error,
            ProjectCheckId::RepositoryWorkingEol,
            Some(source_set.clone()),
            vec![path.clone()],
            1,
            "Working text file contains mixed line endings",
            Vec::new(),
            RemediationKind::Explain,
        ),
        WorkingEolUnsupported {
            source_set,
            path,
            observed,
        } => seed(
            "git.working_eol_unsupported",
            DiagnosticSeverity::Error,
            ProjectCheckId::RepositoryWorkingEol,
            Some(source_set.clone()),
            vec![path.clone()],
            1,
            "Working text file uses unsupported line endings",
            vec![format!("observed: {observed}")],
            RemediationKind::Explain,
        ),
        LfsConsider {
            source_set,
            count,
            total_bytes,
            largest_path,
            largest_bytes,
            single_threshold_bytes,
            aggregate_threshold_bytes,
            paths,
        } => seed(
            "git.lfs_consider",
            DiagnosticSeverity::Info,
            ProjectCheckId::RepositoryLfs,
            Some(source_set.clone()),
            paths.clone(),
            *count,
            "Large binary resources may benefit from Git LFS",
            vec![
                format!("total bytes: {total_bytes}"),
                format!("largest: {largest_path} ({largest_bytes} bytes)"),
                format!("single-file threshold: {single_threshold_bytes}"),
                format!("aggregate threshold: {aggregate_threshold_bytes}"),
            ],
            RemediationKind::Advisory,
        ),
    }
}

const fn incomplete_severity(check: ProjectCheckId) -> DiagnosticSeverity {
    if matches!(check, ProjectCheckId::RepositoryLfs) {
        DiagnosticSeverity::Info
    } else {
        DiagnosticSeverity::Error
    }
}

#[allow(clippy::too_many_arguments)]
fn seed(
    code: &'static str,
    severity: DiagnosticSeverity,
    id: ProjectCheckId,
    source_set: Option<String>,
    paths: Vec<String>,
    count: usize,
    message: &str,
    evidence: Vec<String>,
    remediation_kind: RemediationKind,
) -> DiagnosticSeed {
    DiagnosticSeed {
        code,
        severity,
        check: CheckKey {
            id,
            scope: id.scope(),
            source_set,
        },
        paths,
        count,
        message: message.into(),
        evidence,
        remediation_kind,
    }
}

fn remediation_for(
    kind: RemediationKind,
    count: usize,
    paths: &[String],
    repository_root: Option<&str>,
) -> Remediation {
    match kind {
        RemediationKind::RuntimeSidecar => {
            let commands = if count <= MAX_PROJECT_DIAGNOSTIC_PATHS && count == paths.len() {
                let mut args = vec!["rm".into(), "--cached".into(), "--".into()];
                args.extend(paths.iter().cloned());
                vec![RemediationCommand {
                    program: "git".into(),
                    args,
                    cwd: repository_root
                        .expect("runtime sidecar snapshot validation proves repository root")
                        .into(),
                }]
            } else {
                Vec::new()
            };
            Remediation {
                summary: "Stop tracking the proven runtime sidecar and add a portable ignore rule"
                    .into(),
                steps: vec![
                    format!("Review all {count} proven runtime sidecar path(s)"),
                    "Remove only the proven paths from the index and add tracked ignore rules"
                        .into(),
                    "Run unica.project.status again".into(),
                ],
                commands,
            }
        }
        RemediationKind::ManualReview => Remediation {
            summary: "Review the staged path manually before changing the Git index".into(),
            steps: vec![
                "Inspect the exact staged content and repository policy".into(),
                "Run unica.project.status again after an approved correction".into(),
            ],
            commands: Vec::new(),
        },
        RemediationKind::Advisory => Remediation {
            summary: "Consider Git LFS for the exact proven binary resources".into(),
            steps: vec![
                "Review the listed exact binary paths and repository hosting policy".into(),
                "Do not add a broad *.bin LFS rule because XDTO Package.bin is text".into(),
            ],
            commands: Vec::new(),
        },
        RemediationKind::Explain => Remediation {
            summary: "Correct the reported project policy without automatic mutation".into(),
            steps: vec![
                "Review the diagnostic evidence and update the project or tracked Git policy"
                    .into(),
                "Run unica.project.status again".into(),
            ],
            commands: Vec::new(),
        },
    }
}

const fn severity_rank(severity: DiagnosticSeverity) -> u8 {
    match severity {
        DiagnosticSeverity::Error => 0,
        DiagnosticSeverity::Warning => 1,
        DiagnosticSeverity::Info => 2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::project_sources::{ProjectSourceSet, SourceFormat, SourceSetKind};

    #[test]
    fn project_health_serializes_independent_source_and_repository_readiness() {
        let report = evaluate_project_health(snapshot_with(
            vec![ProjectHealthFact::GitRepositoryAbsent],
            vec![],
        ))
        .unwrap();
        let value = serde_json::to_value(report).unwrap();

        assert_eq!(value["ready"], true);
        assert_eq!(value["repositoryReady"], false);
        assert_eq!(value["diagnostics"][0]["code"], "git.repository_absent");
        assert_eq!(value["diagnostics"][0]["count"], 1);
        assert!(value["diagnostics"][0]["remediation"]["commands"].is_array());
    }

    #[test]
    fn not_run_closes_only_its_scope_and_not_applicable_closes_neither() {
        let report = evaluate_project_health(snapshot_with(
            Vec::new(),
            vec![
                observation(
                    ProjectCheckId::SourceGeneratedPaths,
                    Some("main"),
                    ProjectCheckOutcome::NotApplicable {
                        reason: "the profile has no generated paths".into(),
                    },
                ),
                observation(
                    ProjectCheckId::RepositoryIgnore,
                    None,
                    ProjectCheckOutcome::NotRun {
                        reason: "Git is absent".into(),
                    },
                ),
            ],
        ))
        .unwrap();

        assert!(report.ready);
        assert!(!report.repository_ready);
    }

    #[test]
    fn incomplete_repository_fact_serializes_its_check_as_not_run() {
        let report = evaluate_project_health(snapshot_with(
            vec![ProjectHealthFact::GitInspectionIncomplete {
                check: ProjectCheckId::RepositoryIndex,
                source_set: None,
                reason: "Git index output was truncated".into(),
            }],
            vec![observation(
                ProjectCheckId::RepositoryIndex,
                None,
                ProjectCheckOutcome::NotRun {
                    reason: "Git index output was truncated".into(),
                },
            )],
        ))
        .unwrap();

        let check = report
            .checks
            .iter()
            .find(|check| check.id == "repository.index" && check.source_set.is_none())
            .unwrap();
        assert_eq!(check.status, ProjectCheckStatus::NotRun);
        assert_eq!(
            check.reason.as_deref(),
            Some("Git index output was truncated")
        );
        assert!(!report.repository_ready);
        assert_eq!(report.diagnostics[0].code, "git.inspection_incomplete");
    }

    #[test]
    fn lfs_inspection_incomplete_is_advisory_and_does_not_close_readiness() {
        let report = evaluate_project_health(snapshot_with(
            vec![ProjectHealthFact::GitInspectionIncomplete {
                check: ProjectCheckId::RepositoryLfs,
                source_set: None,
                reason: "binary size could not be inspected".into(),
            }],
            vec![observation(
                ProjectCheckId::RepositoryLfs,
                None,
                ProjectCheckOutcome::NotRun {
                    reason: "binary size could not be inspected".into(),
                },
            )],
        ))
        .unwrap();

        let check = report
            .checks
            .iter()
            .find(|check| check.id == "repository.lfs" && check.source_set.is_none())
            .unwrap();
        assert_eq!(check.status, ProjectCheckStatus::NotRun);
        assert!(report.repository_ready);
        assert_eq!(report.diagnostics[0].severity, DiagnosticSeverity::Info);
    }

    #[test]
    fn incomplete_source_discovery_closes_source_readiness_with_a_typed_cause() {
        let report = evaluate_project_health(snapshot_with(
            vec![ProjectHealthFact::SourceInspectionIncomplete {
                reason: "v8project.yaml is malformed".into(),
            }],
            vec![observation(
                ProjectCheckId::SourceDiscovery,
                None,
                ProjectCheckOutcome::NotRun {
                    reason: "v8project.yaml is malformed".into(),
                },
            )],
        ))
        .unwrap();

        assert!(!report.ready);
        assert!(report.repository_ready);
        assert_eq!(
            report.diagnostics[0].code,
            "source_set.inspection_incomplete"
        );
        assert_eq!(report.diagnostics[0].scope, DiagnosticScope::Workspace);
    }

    #[test]
    fn lfs_advice_is_informational_and_does_not_close_readiness() {
        let report = evaluate_project_health(snapshot_with(
            vec![ProjectHealthFact::LfsConsider {
                source_set: "main".into(),
                count: 2,
                total_bytes: 30 * 1024 * 1024,
                largest_path: "src/Templates/large.bin".into(),
                largest_bytes: 20 * 1024 * 1024,
                single_threshold_bytes: 10 * 1024 * 1024,
                aggregate_threshold_bytes: 100 * 1024 * 1024,
                paths: vec![
                    "src/Templates/large.bin".into(),
                    "src/Templates/other.bin".into(),
                ],
            }],
            Vec::new(),
        ))
        .unwrap();

        assert!(report.ready);
        assert!(report.repository_ready);
        assert_eq!(report.diagnostics[0].severity, DiagnosticSeverity::Info);
        assert_eq!(report.diagnostics[0].count, 2);
    }

    #[test]
    fn runtime_sidecar_remediation_keeps_unusual_path_in_one_argv_item() {
        let mut snapshot = snapshot_with(
            vec![ProjectHealthFact::RuntimeSidecarTracked {
                source_set: "main".into(),
                path: "src/line\nbreak/ConfigDumpInfo.xml".into(),
            }],
            Vec::new(),
        );
        snapshot.repository_root = Some("/repo".into());

        let report = evaluate_project_health(snapshot).unwrap();
        let command = &report.diagnostics[0].remediation.commands[0];

        assert_eq!(command.program, "git");
        assert_eq!(command.cwd, "/repo");
        assert_eq!(
            command.args,
            ["rm", "--cached", "--", "src/line\nbreak/ConfigDumpInfo.xml"]
        );
    }

    #[test]
    fn runtime_sidecar_aggregation_never_publishes_a_partial_command() {
        let facts = (0..21)
            .map(|index| ProjectHealthFact::RuntimeSidecarTracked {
                source_set: "main".into(),
                path: format!("src/runtime-{index:02}/ConfigDumpInfo.xml"),
            })
            .collect();
        let mut snapshot = snapshot_with(facts, Vec::new());
        snapshot.repository_root = Some("/repo".into());

        let report = evaluate_project_health(snapshot).unwrap();
        let diagnostic = &report.diagnostics[0];

        assert_eq!(diagnostic.count, 21);
        assert_eq!(diagnostic.paths.len(), MAX_PROJECT_DIAGNOSTIC_PATHS);
        assert!(diagnostic.remediation.commands.is_empty());
    }

    #[test]
    fn ambiguous_config_dump_info_never_has_a_removal_command() {
        let report = evaluate_project_health(snapshot_with(
            vec![ProjectHealthFact::ConfigDumpInfoUnclassified {
                source_set: Some("main".into()),
                path: "src/ConfigDumpInfo.xml".into(),
                reason: "staged blob is malformed".into(),
            }],
            Vec::new(),
        ))
        .unwrap();

        assert_eq!(
            report.diagnostics[0].code,
            "git.config_dump_info_unclassified"
        );
        assert!(report.diagnostics[0].remediation.commands.is_empty());
    }

    #[test]
    fn ordinary_fact_without_a_completed_check_is_a_fatal_snapshot_error() {
        let snapshot = snapshot_with(
            vec![ProjectHealthFact::IgnoreRuleMissing {
                source_set: None,
                path: ".build/probe".into(),
            }],
            vec![observation(
                ProjectCheckId::RepositoryIgnore,
                None,
                ProjectCheckOutcome::NotRun {
                    reason: "ignore inspection did not run".into(),
                },
            )],
        );

        let error = evaluate_project_health(snapshot).unwrap_err();

        assert!(
            error.starts_with("project_health_snapshot_invalid:"),
            "{error}"
        );
        assert!(error.contains("repository.ignore"), "{error}");
    }

    #[test]
    fn incomplete_fact_requires_a_not_run_observation() {
        let snapshot = snapshot_with(
            vec![ProjectHealthFact::GitInspectionIncomplete {
                check: ProjectCheckId::RepositoryIndex,
                source_set: None,
                reason: "Git index output was truncated".into(),
            }],
            Vec::new(),
        );

        let error = evaluate_project_health(snapshot).unwrap_err();

        assert!(error.contains("incompatible outcome"), "{error}");
        assert!(error.contains("repository.index"), "{error}");
    }

    #[test]
    fn complete_source_targets_require_every_source_check() {
        let mut snapshot = snapshot_with(Vec::new(), Vec::new());
        snapshot.observations.retain(|observation| {
            !(observation.id == ProjectCheckId::SourceFormat
                && observation.source_set.as_deref() == Some("main"))
        });

        let error = evaluate_project_health(snapshot).unwrap_err();

        assert!(
            error.starts_with("project_health_snapshot_invalid:"),
            "{error}"
        );
        assert!(error.contains("source.format"), "{error}");
        assert!(error.contains("main"), "{error}");
    }

    #[test]
    fn git_inspection_fact_rejects_a_source_scoped_check_id() {
        let snapshot = snapshot_with(
            vec![ProjectHealthFact::GitInspectionTimeout {
                check: ProjectCheckId::SourceLayout,
                source_set: Some("main".into()),
            }],
            Vec::new(),
        );

        let error = evaluate_project_health(snapshot).unwrap_err();

        assert!(
            error.starts_with("project_health_snapshot_invalid:"),
            "{error}"
        );
        assert!(error.contains("git.inspection_timeout"), "{error}");
        assert!(error.contains("source.layout"), "{error}");
    }

    #[test]
    fn diagnostics_are_sorted_and_aggregated_without_losing_count() {
        let report = evaluate_project_health(snapshot_with(
            vec![
                ProjectHealthFact::IgnoreRuleMissing {
                    source_set: Some("main".into()),
                    path: "src/.build/probe-b".into(),
                },
                ProjectHealthFact::SourceRootIsWorkspace {
                    source_set: "main".into(),
                    path: ".".into(),
                    evidence: vec!["normalized identity: /workspace".into()],
                },
                ProjectHealthFact::IgnoreRuleMissing {
                    source_set: Some("main".into()),
                    path: "src/.build/probe-a".into(),
                },
            ],
            Vec::new(),
        ))
        .unwrap();

        assert_eq!(report.diagnostics.len(), 2);
        assert_eq!(report.diagnostics[0].scope, DiagnosticScope::Repository);
        assert_eq!(report.diagnostics[0].code, "git.ignore_rule_missing");
        assert_eq!(report.diagnostics[0].count, 2);
        assert_eq!(
            report.diagnostics[0].paths,
            ["src/.build/probe-a", "src/.build/probe-b"]
        );
        assert_eq!(report.diagnostics[1].scope, DiagnosticScope::SourceSet);
    }

    fn snapshot_with(
        facts: Vec<ProjectHealthFact>,
        replacements: Vec<ProjectCheckObservation>,
    ) -> ProjectHealthSnapshot {
        let repository_absent = facts
            .iter()
            .any(|fact| matches!(fact, ProjectHealthFact::GitRepositoryAbsent));
        let mut observations = ProjectCheckId::ALL
            .into_iter()
            .map(|id| {
                let source_set = (id.scope() == DiagnosticScope::SourceSet).then_some("main");
                let outcome = if repository_absent
                    && id.scope() == DiagnosticScope::Repository
                    && id != ProjectCheckId::RepositoryDiscovery
                {
                    ProjectCheckOutcome::NotRun {
                        reason: "Git is absent".into(),
                    }
                } else {
                    ProjectCheckOutcome::Completed
                };
                observation(id, source_set, outcome)
            })
            .collect::<Vec<_>>();
        observations.extend(
            [
                ProjectCheckId::RepositoryIgnore,
                ProjectCheckId::RepositoryConfigDumpInfo,
                ProjectCheckId::RepositoryAttributes,
                ProjectCheckId::RepositoryIndexEol,
                ProjectCheckId::RepositoryWorkingEol,
                ProjectCheckId::RepositoryLfs,
            ]
            .into_iter()
            .map(|id| observation(id, Some("main"), ProjectCheckOutcome::Completed)),
        );
        for replacement in replacements {
            observations.retain(|existing| {
                (existing.id, existing.source_set.as_deref())
                    != (replacement.id, replacement.source_set.as_deref())
            });
            observations.push(replacement);
        }
        ProjectHealthSnapshot {
            workspace_root: "/workspace".into(),
            cache_root: "/workspace/.build/unica".into(),
            repository_root: (!repository_absent).then(|| "/workspace".into()),
            source_sets: Some(vec![ProjectSourceSet {
                name: "main".into(),
                kind: SourceSetKind::Configuration,
                path: "src".into(),
                source_format: SourceFormat::PlatformXml,
                format_evidence: vec!["Configuration.xml".into()],
            }]),
            source_targets_complete: true,
            observations,
            facts,
        }
    }

    fn observation(
        id: ProjectCheckId,
        source_set: Option<&str>,
        outcome: ProjectCheckOutcome,
    ) -> ProjectCheckObservation {
        ProjectCheckObservation {
            id,
            scope: id.scope(),
            source_set: source_set.map(str::to_owned),
            outcome,
        }
    }
}
