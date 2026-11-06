//! Продуктовая сторона службы LSP: папка редактора допускается так же, как
//! `workspace_hint` вызова MCP, и модуль читается той же авторизацией чтения.

use super::server::closed_daemon_source_relative_path;
use crate::domain::source_capabilities::bsl_profile_for_source;
use crate::infrastructure::source_workspace::{
    discover_project_source_admission, WorkspaceActor, WorkspaceActorRegistry,
};
use crate::infrastructure::v13_read_port::ProviderReadAuthority;
use crate::infrastructure::workspace::discover_workspace;
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
use unica_source_contract::{SourceFormat, SourceProfile};
use unica_workspace::actor::{ProviderRootBinding, WorkspaceSourceSetInput};

/// The scope under which MCP admits a workspace. The actor of a workspace is
/// the same only for the same inputs (`actor-identity`).
const ACTOR_SCOPE: &str = "canonical-v0.13";

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
        let context = discover_workspace(Some(folder.to_path_buf())).ok()?;
        // The walk stops at the editor's deadline instead of outliving it.
        let mut checkpoint = || {
            if Instant::now() < deadline {
                Ok(())
            } else {
                Err("LSP workspace admission passed its deadline".to_string())
            }
        };
        let admission =
            discover_project_source_admission(&context.workspace_root, &mut checkpoint).ok()?;
        let mut sources = admission
            .map()
            .source_sets
            .iter()
            .filter(|source| source.source_format == SourceFormat::PlatformXml)
            .map(|source| {
                let relative = closed_daemon_source_relative_path(&source.path).ok()?;
                let identity = admission.source_root_identity(&relative)?;
                Some((
                    source.name.clone(),
                    source.kind,
                    context.workspace_root.join(&relative),
                    identity,
                ))
            })
            .collect::<Option<Vec<_>>>()?;
        if sources.is_empty() {
            return None;
        }
        sources.sort_by(|left, right| left.0.cmp(&right.0));
        let actor = self
            .actors
            .get_or_create(
                &context,
                sources.iter().map(|(name, kind, root, _)| {
                    WorkspaceSourceSetInput::new(
                        name.clone(),
                        root.clone(),
                        *kind,
                        SourceFormat::PlatformXml,
                        SourceProfile::platform_xml_8_3_27_format_2_20(),
                    )
                }),
                ACTOR_SCOPE,
            )
            .ok()?;
        let bindings = sources
            .iter()
            .map(|(name, _, root, identity)| {
                let binding = actor.bind_provider_root(name, root).ok()?;
                (binding.retained_root().identity() == *identity).then_some(binding)
            })
            .collect::<Option<Vec<_>>>()?;
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
        let identity = actor.safe_identity_hash().ok()?;
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
