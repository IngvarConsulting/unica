//! Trusted composition of the workspace owner and the existing source policies.
use super::revision_artifact_policy::RevisionArtifactPolicy;
use super::v13_large_configuration::RegistrationCache;
use std::path::Path;
use std::sync::Arc;
use unica_source_contract::read::{
    BoundSourceArtifactPolicy, RevisionArtifactDisposition, SourceArtifactClassifier,
};
#[cfg(test)]
use unica_source_contract::workspace::WorkspaceContext;
use unica_source_contract::{SourceFormat, SourceProfile, SourceSetKind};
use unica_source_designer::project_sources::ActorSourceMapDiscovery;
#[cfg(test)]
use unica_workspace::actor::WorkspaceIdentity;
use unica_workspace::policies::WorkspacePolicies;

pub(crate) type WorkspaceActor<R = ()> =
    unica_workspace::actor::WorkspaceActor<R, RegistrationCache>;
pub(crate) type WorkspaceActorRegistry =
    unica_workspace::actor::WorkspaceActorRegistry<RegistrationCache>;

struct SourceArtifacts;

impl BoundSourceArtifactPolicy for RevisionArtifactPolicy {
    fn classify(&self, relative: &Path) -> RevisionArtifactDisposition {
        RevisionArtifactPolicy::classify(*self, relative)
    }
}

impl SourceArtifactClassifier for SourceArtifacts {
    fn bind(
        &self,
        kind: SourceSetKind,
        format: SourceFormat,
        profile: SourceProfile,
    ) -> Result<Arc<dyn BoundSourceArtifactPolicy>, String> {
        RevisionArtifactPolicy::from_source_profile(kind, format, profile)
            .map(|policy| Arc::new(policy) as Arc<dyn BoundSourceArtifactPolicy>)
    }
}

pub(crate) fn policies() -> WorkspacePolicies {
    WorkspacePolicies::new(
        Arc::new(ActorSourceMapDiscovery::new(
            unica_workspace::paths::host_path_text,
        )),
        source_artifact_classifier(),
    )
}

pub(crate) fn actor_registry() -> WorkspaceActorRegistry {
    WorkspaceActorRegistry::with_policies(policies())
}

#[cfg(test)]
pub(crate) fn new_actor(
    identity: WorkspaceIdentity,
    context: WorkspaceContext,
) -> Result<WorkspaceActor, String> {
    WorkspaceActor::new(identity, context).map(|actor| actor.with_policies(policies()))
}

pub(crate) fn discover_project_source_admission(
    workspace: &Path,
    checkpoint: &mut dyn FnMut() -> Result<(), String>,
) -> Result<unica_workspace::selection_evidence::ResolvedProjectSourceAdmission, String> {
    unica_workspace::selection_evidence::discover_project_source_admission(
        workspace,
        Arc::new(ActorSourceMapDiscovery::new(
            unica_workspace::paths::host_path_text,
        )),
        checkpoint,
    )
}

pub(crate) fn source_artifact_classifier() -> Arc<dyn SourceArtifactClassifier> {
    Arc::new(SourceArtifacts)
}
