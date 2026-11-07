//! Продуктовая сторона службы LSP: папка редактора допускается тем же
//! допуском, что `workspace_hint` вызова MCP, и модуль читается той же
//! авторизацией чтения.

use super::server::admit_editor_folder;
use crate::domain::source_capabilities::bsl_profile_for_source;
use crate::infrastructure::source_workspace::{WorkspaceActor, WorkspaceActorRegistry};
use crate::infrastructure::v13_read_port::ProviderReadAuthority;
use std::path::Path;
use std::sync::{Arc, Weak};
use std::time::Instant;
use unica_address::source_target::MetadataAddress;
use unica_daemon::protocol_v5::lsp::V5LspFailure;
use unica_execution_contract::invocation::SafeIdentityHash;
use unica_execution_contract::view_error::ViewError;
use unica_lsp::{
    AdmittedWorkspace, LspModuleReads, LspWorkspaces, ModuleSourceRead, SourceSetLayout,
};
use unica_workspace::actor::ProviderRootBinding;

pub(super) struct ProductLspWorkspaces {
    actors: Arc<WorkspaceActorRegistry>,
}

impl ProductLspWorkspaces {
    pub(super) fn new(actors: Arc<WorkspaceActorRegistry>) -> Self {
        Self { actors }
    }
}

impl LspWorkspaces for ProductLspWorkspaces {
    fn admit(&self, folder: &Path, deadline: Instant) -> Option<AdmittedWorkspace> {
        let admitted = admit_editor_folder(folder, &self.actors, deadline)?;
        let (actor, bindings, identity) = (admitted.actor, admitted.bindings, admitted.identity);
        let source_sets = bindings
            .iter()
            .map(|binding| {
                Some(SourceSetLayout {
                    name: binding.source_set_name().to_owned(),
                    kind: binding.source_kind(),
                    root: binding.source_root().to_path_buf(),
                    profile: bsl_profile_for_source(binding.source_profile())?,
                })
            })
            .collect::<Option<Vec<_>>>()?;
        Some(AdmittedWorkspace {
            reads: Box::new(ProductModuleReads {
                actor: Arc::downgrade(&actor),
                bindings,
                identity,
            }),
            source_sets,
            actor,
        })
    }
}

struct ProductModuleReads {
    actor: Weak<WorkspaceActor>,
    bindings: Vec<ProviderRootBinding>,
    identity: SafeIdentityHash,
}

impl LspModuleReads for ProductModuleReads {
    fn read(
        &self,
        session: &str,
        source_set: &str,
    ) -> Result<Box<dyn ModuleSourceRead>, V5LspFailure> {
        let binding = self
            .bindings
            .iter()
            .find(|binding| binding.source_set_name() == source_set)
            .ok_or(V5LspFailure::DocumentUnknown)?;
        let actor = self
            .actor
            .upgrade()
            .ok_or(V5LspFailure::WorkspaceUnavailable)?;
        actor
            .validate_binding(binding)
            .map_err(|_| V5LspFailure::WorkspaceUnavailable)?;
        let artifacts = actor
            .source_artifacts(binding)
            .map_err(|_| V5LspFailure::WorkspaceUnavailable)?;
        let documents = actor
            .documents()
            .session_documents(session, source_set)
            .map_err(|_| V5LspFailure::Unavailable)?;
        Ok(Box::new(
            ProviderReadAuthority::new_local_snapshot(
                binding.source_set_name(),
                format!("{}:{}", self.identity.as_str(), binding.source_set_name()),
                binding.source_kind(),
                binding.retained_root(),
                uuid::Uuid::new_v4().to_string(),
            )
            .with_artifacts(artifacts)
            .with_documents(documents),
        ))
    }
}

impl ModuleSourceRead for ProviderReadAuthority {
    fn source_set(&self) -> &str {
        ProviderReadAuthority::source_set(self)
    }

    fn root_path(&self) -> &Path {
        ProviderReadAuthority::root_path(self)
    }

    fn module_source(&self, target: &MetadataAddress) -> Result<Option<String>, ViewError> {
        ProviderReadAuthority::module_source(self, target)
    }
}

#[cfg(test)]
mod tests;
