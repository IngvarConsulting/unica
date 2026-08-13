pub(crate) mod git;
pub(crate) mod layout;
pub(crate) mod resources;

use crate::domain::cancellation::CancellationToken;
use crate::domain::code_intelligence::ProviderDeadline;
use crate::domain::project_health::{
    ProjectCheckId, ProjectCheckObservation, ProjectCheckOutcome, ProjectHealthInspectionError,
    ProjectHealthSnapshot,
};
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::internal_adapters::{system_process_runner, ProcessRunner};
use git::GitRepositoryInspector;
use layout::SourceLayoutInspector;
use resources::SourceResourcePolicyInspector;

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
    let mut observations = layout.observations;
    observations.extend(git.observations);
    let mut facts = layout.facts;
    facts.extend(git.facts);
    if let Some(repository_root) = git.repository_root.as_ref() {
        let resources = SourceResourcePolicyInspector::with_process_runner(runner).inspect(
            repository_root,
            &layout.roots,
            &git.entries,
            cancellation,
            deadline,
        )?;
        observations.extend(resources.observations);
        facts.extend(resources.facts);
    } else {
        for id in [
            ProjectCheckId::RepositoryAttributes,
            ProjectCheckId::RepositoryIndexEol,
            ProjectCheckId::RepositoryWorkingEol,
            ProjectCheckId::RepositoryLfs,
        ] {
            observations.push(ProjectCheckObservation {
                id,
                scope: id.scope(),
                source_set: None,
                outcome: ProjectCheckOutcome::NotRun {
                    reason: "Git repository root is unavailable".into(),
                },
            });
        }
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
