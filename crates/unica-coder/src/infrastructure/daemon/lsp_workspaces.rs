//! Продуктовая сторона службы LSP: папка редактора допускается тем же
//! допуском, что `workspace_hint` вызова MCP, модуль читается той же
//! авторизацией чтения, а символы отвечает та же служба символов.

use super::server::admit_editor_folder;
use super::v13_service::{CanonicalV13ReadService, SymbolOperation, SymbolOutcome, SymbolRequest};
use crate::domain::source_capabilities::bsl_profile_for_source;
use crate::infrastructure::source_workspace::{WorkspaceActor, WorkspaceActorRegistry};
use crate::infrastructure::v13_large_configuration::RegistrationCache;
use crate::infrastructure::v13_read::LogicalViewReadAuthority;
use crate::infrastructure::v13_read_port::ProviderReadAuthority;
use std::path::Path;
use std::sync::{Arc, Weak};
use std::time::Instant;
use unica_address::address::QualifiedAddress;
use unica_address::source_target::MetadataAddress;
use unica_code_intelligence::symbols::SymbolRequestContext;
use unica_diagnostics::model::DiagnosticProviderRegistry;
use unica_execution_contract::invocation::SafeIdentityHash;
use unica_execution_contract::view_error::ViewError;
use unica_lsp::{
    AdmittedWorkspace, LspAnalysis, LspDiagnosticGap, LspDiagnostics, LspFinding, LspModuleReads,
    LspSymbolAnswer, LspSymbolOperation, LspSymbolRequest, LspSymbols, LspWorkspaces,
    ModuleSourceRead, SourceSetLayout,
};
use unica_lsp_contract::protocol::V5LspFailure;
use unica_refusal::RefusalCode;
use unica_work_control::cancellation::CancellationToken;
use unica_work_control::provider_deadline::ProviderDeadline;
use unica_workspace::actor::ProviderRootBinding;

pub(super) struct ProductLspWorkspaces {
    actors: Arc<WorkspaceActorRegistry>,
    service: Arc<CanonicalV13ReadService>,
}

impl ProductLspWorkspaces {
    pub(super) fn new(
        actors: Arc<WorkspaceActorRegistry>,
        service: Arc<CanonicalV13ReadService>,
    ) -> Self {
        Self { actors, service }
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
        let space = Arc::new(ProductSpace {
            actor: Arc::downgrade(&actor),
            bindings,
            identity,
        });
        Some(AdmittedWorkspace {
            reads: Box::new(ProductModuleReads(Arc::clone(&space))),
            diagnostics: Arc::new(ProductDiagnostics {
                space: Arc::clone(&space),
                registry: Arc::new(super::lsp_diagnostics::registry_for(Arc::clone(
                    &self.service,
                ))),
            }),
            symbols: Arc::new(ProductSymbols {
                space,
                service: Arc::clone(&self.service),
            }),
            source_sets,
            actor,
        })
    }
}

/// An admitted workspace as the session holds it: its actor only weakly.
struct ProductSpace {
    actor: Weak<WorkspaceActor>,
    bindings: Vec<ProviderRootBinding>,
    identity: SafeIdentityHash,
}

impl ProductSpace {
    /// The live actor, the binding of `source_set` it still holds, and the
    /// set's registration cache.
    fn bound(
        &self,
        source_set: &str,
    ) -> Result<
        (
            Arc<WorkspaceActor>,
            &ProviderRootBinding,
            Arc<RegistrationCache>,
        ),
        V5LspFailure,
    > {
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
        Ok((actor, binding, artifacts))
    }

    /// The saved sources of `binding`: no buffer of any session.
    fn saved_read(&self, binding: &ProviderRootBinding) -> ProviderReadAuthority {
        ProviderReadAuthority::new_local_snapshot(
            binding.source_set_name(),
            format!("{}:{}", self.identity.as_str(), binding.source_set_name()),
            binding.source_kind(),
            binding.retained_root(),
            uuid::Uuid::new_v4().to_string(),
        )
    }
}

struct ProductModuleReads(Arc<ProductSpace>);

impl LspModuleReads for ProductModuleReads {
    fn saved(&self, source_set: &str) -> Result<Box<dyn ModuleSourceRead>, V5LspFailure> {
        let (_, binding, artifacts) = self.0.bound(source_set)?;
        Ok(Box::new(
            self.0.saved_read(binding).with_artifacts(artifacts),
        ))
    }

    fn read(
        &self,
        session: &str,
        source_set: &str,
    ) -> Result<Box<dyn ModuleSourceRead>, V5LspFailure> {
        let (actor, binding, artifacts) = self.0.bound(source_set)?;
        let documents = actor
            .documents()
            .session_documents(session, source_set)
            .map_err(|_| V5LspFailure::Unavailable)?;
        Ok(Box::new(
            self.0
                .saved_read(binding)
                .with_artifacts(artifacts)
                .with_documents(documents),
        ))
    }
}

type Registry = dyn Fn() -> Result<DiagnosticProviderRegistry, String> + Send + Sync;

/// The checks and the analysis provider `check` uses, in Unica's words.
struct ProductDiagnostics {
    space: Arc<ProductSpace>,
    registry: Arc<Registry>,
}

impl LspDiagnostics for ProductDiagnostics {
    fn own_findings(&self, text: &str) -> Vec<LspFinding> {
        super::lsp_diagnostics::own_findings(text)
    }

    fn analyse(
        &self,
        source_set: &str,
        module: &str,
        cancellation: &CancellationToken,
    ) -> LspAnalysis {
        let unavailable = || LspAnalysis {
            findings: Vec::new(),
            gaps: vec![LspDiagnosticGap::AnalysisUnavailable],
        };
        let Ok((actor, binding, artifacts)) = self.space.bound(source_set) else {
            return unavailable();
        };
        let (Some(profile), Ok(module)) = (
            bsl_profile_for_source(binding.source_profile()),
            QualifiedAddress::parse(&format!("{module}.Body")),
        ) else {
            return unavailable();
        };
        let authority = LogicalViewReadAuthority::with_read_authority_and_registration_cache(
            cancellation,
            self.space.saved_read(binding),
            profile,
            ProviderDeadline::no_deadline(),
            artifacts,
        );
        super::lsp_diagnostics::analyse(
            self.registry.as_ref(),
            &authority,
            actor.context(),
            &module,
            cancellation,
        )
    }
}

/// The symbol service MCP's symbols use, over the saved sources; the
/// session's buffer is given beside them.
struct ProductSymbols {
    space: Arc<ProductSpace>,
    service: Arc<CanonicalV13ReadService>,
}

impl LspSymbols for ProductSymbols {
    fn ask(
        &self,
        request: LspSymbolRequest<'_>,
        context: &SymbolRequestContext<'_>,
    ) -> Result<LspSymbolAnswer, V5LspFailure> {
        let (actor, binding, artifacts) = self.space.bound(request.source_set)?;
        let profile = bsl_profile_for_source(binding.source_profile())
            .ok_or(V5LspFailure::WorkspaceUnavailable)?;
        let module =
            QualifiedAddress::parse(request.module).map_err(|_| V5LspFailure::DocumentUnknown)?;
        let authority = LogicalViewReadAuthority::with_read_authority_and_registration_cache(
            context.cancellation,
            self.space.saved_read(binding),
            profile,
            context.deadline,
            artifacts,
        );
        let operation = match request.operation {
            LspSymbolOperation::Definition => SymbolOperation::Definition,
            LspSymbolOperation::Hover => SymbolOperation::Hover,
            LspSymbolOperation::References => SymbolOperation::References,
        };
        let (_, outcome) = self
            .service
            .symbols_with(
                &authority,
                actor.context(),
                SymbolRequest {
                    module,
                    buffer: request.buffer,
                    position: request.position,
                    operation,
                },
                context,
                true,
            )
            .map_err(|error| match error.code() {
                RefusalCode::NotFound => V5LspFailure::DocumentUnknown,
                _ => V5LspFailure::Unavailable,
            })?;
        Ok(match outcome {
            SymbolOutcome::Places(places) => LspSymbolAnswer::Places(places),
            SymbolOutcome::Info(info) => LspSymbolAnswer::Info(info),
        })
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
