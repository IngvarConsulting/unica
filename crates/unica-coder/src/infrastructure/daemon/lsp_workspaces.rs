//! Продуктовая сторона службы LSP: папка редактора допускается тем же
//! допуском, что `workspace_hint` вызова MCP, модуль читается той же
//! авторизацией чтения, а символы отвечает та же служба символов.

use super::lsp_completion::{PlatformContexts, SessionSources};
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
use unica_code_intelligence::completion::{CompletionMethod, PlatformMember};
use unica_code_intelligence::symbols::SymbolRequestContext;
use unica_diagnostics::model::DiagnosticProviderRegistry;
use unica_execution_contract::invocation::SafeIdentityHash;
use unica_lsp::{
    AdmittedWorkspace, LspAnalysis, LspDiagnosticGap, LspDiagnostics, LspFinding, LspModuleReads,
    LspSymbolAnswer, LspSymbolOperation, LspSymbolRequest, LspSymbols, LspWorkspaces,
    ModuleSourceRead, SourceSetLayout,
};
use unica_lsp::{LspCompletion, LspCompletionAnswer, LspCompletionRequest};
use unica_lsp_contract::protocol::V5LspFailure;
use unica_refusal::RefusalCode;
use unica_work_control::cancellation::CancellationToken;
use unica_work_control::provider_deadline::ProviderDeadline;
use unica_workspace::actor::ProviderRootBinding;

type Installation = dyn Fn(&unica_source_contract::workspace::WorkspaceContext) -> Option<std::path::PathBuf>
    + Send
    + Sync;

pub(super) struct ProductLspWorkspaces {
    actors: Arc<WorkspaceActorRegistry>,
    service: Arc<CanonicalV13ReadService>,
    platform: Arc<PlatformContexts>,
    /// The platform installation whose global context a workspace sees.
    installation: Arc<Installation>,
}

impl ProductLspWorkspaces {
    pub(super) fn new(
        actors: Arc<WorkspaceActorRegistry>,
        service: Arc<CanonicalV13ReadService>,
    ) -> Self {
        Self {
            actors,
            service,
            platform: Arc::default(),
            installation: Arc::new(
                crate::infrastructure::application_ports::platform_installation_root,
            ),
        }
    }

    /// A fixed installation, whose global context the test has loaded.
    #[cfg(test)]
    pub(super) fn with_platform_for_test(
        mut self,
        root: std::path::PathBuf,
        members: Vec<PlatformMember>,
    ) -> Self {
        self.platform.ready_for_test(root.clone(), members);
        self.installation = Arc::new(move |_| Some(root.clone()));
        self
    }

    /// A fixed installation whose global context is still loading.
    #[cfg(test)]
    pub(super) fn with_platform_loading_for_test(mut self, root: std::path::PathBuf) -> Self {
        self.platform.loading_for_test(root.clone());
        self.installation = Arc::new(move |_| Some(root.clone()));
        self
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
        // The global context loads from the session's start, not with its
        // first completion: a cold help index takes longer than a request.
        let installation = (self.installation)(actor.context());
        if let Some(root) = &installation {
            let _ = self.platform.members(root);
        }
        Some(AdmittedWorkspace {
            reads: Box::new(ProductModuleReads(Arc::clone(&space))),
            completion: Arc::new(ProductCompletion {
                space: Arc::clone(&space),
                platform: Arc::clone(&self.platform),
                installation,
            }),
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

/// Completion over the session's view of a source set and the global
/// context of the project's platform installation.
struct ProductCompletion {
    space: Arc<ProductSpace>,
    platform: Arc<PlatformContexts>,
    /// `None`: the project has no installation, so nothing is to wait for.
    installation: Option<std::path::PathBuf>,
}

impl ProductCompletion {
    /// The members of the global context, or `None` while they load.
    fn platform(&self) -> Option<Arc<[PlatformMember]>> {
        match &self.installation {
            None => Some(Arc::from(Vec::new())),
            Some(root) => self.platform.members(root),
        }
    }

    fn sources<'a>(
        &self,
        session: &str,
        source_set: &'a str,
        module: &'a str,
        platform: Arc<[PlatformMember]>,
        cancellation: &'a CancellationToken,
    ) -> Result<SessionSources<'a>, V5LspFailure> {
        let (actor, binding, artifacts) = self.space.bound(source_set)?;
        let profile = bsl_profile_for_source(binding.source_profile())
            .ok_or(V5LspFailure::WorkspaceUnavailable)?;
        let documents = actor
            .documents()
            .session_documents(session, source_set)
            .map_err(|_| V5LspFailure::Unavailable)?;
        let descriptors = self
            .space
            .saved_read(binding)
            .with_artifacts(Arc::clone(&artifacts));
        let view = LogicalViewReadAuthority::with_read_authority_and_registration_cache(
            cancellation,
            self.space.saved_read(binding).with_documents(documents),
            profile,
            ProviderDeadline::no_deadline(),
            artifacts,
        );
        Ok(SessionSources {
            source_set,
            module,
            registrations: std::cell::OnceCell::new(),
            descriptors,
            view,
            platform,
            cancellation,
            missed: std::cell::Cell::new(false),
        })
    }
}

impl LspCompletion for ProductCompletion {
    fn complete(
        &self,
        request: LspCompletionRequest<'_>,
        cancellation: &CancellationToken,
    ) -> Result<LspCompletionAnswer, V5LspFailure> {
        let platform = self.platform();
        let sources = self.sources(
            request.session,
            request.source_set,
            request.module,
            platform.clone().unwrap_or_else(|| Arc::from(Vec::new())),
            cancellation,
        )?;
        let completion =
            unica_code_intelligence::completion::completion(request.text, request.offset, &sources);
        Ok(LspCompletionAnswer {
            complete: platform.is_some() && !sources.missed(),
            completion,
        })
    }

    fn describe(
        &self,
        session: &str,
        source_set: &str,
        module: &str,
        method: &CompletionMethod,
        cancellation: &CancellationToken,
    ) -> Option<String> {
        use unica_code_intelligence::completion::CompletionSources;
        let sources = self
            .sources(
                session,
                source_set,
                module,
                Arc::from(Vec::new()),
                cancellation,
            )
            .ok()?;
        let text = match &method.common_module {
            Some(common) => sources.common_module_text(common)?,
            None => sources
                .view
                .module_text(&QualifiedAddress::parse(module).ok()?)
                .ok()?,
        };
        unica_code_intelligence::completion::method_description(&text, method)
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
        saved: &str,
        cancellation: &CancellationToken,
    ) -> LspAnalysis {
        let unavailable = || LspAnalysis {
            findings: Vec::new(),
            gaps: vec![LspDiagnosticGap::AnalysisUnavailable],
        };
        let Ok((actor, binding, artifacts)) = self.space.bound(source_set) else {
            return unavailable();
        };
        let (Some(profile), Ok(body), Ok(module)) = (
            bsl_profile_for_source(binding.source_profile()),
            QualifiedAddress::parse(&format!("{module}.Body")),
            QualifiedAddress::parse(module),
        ) else {
            return unavailable();
        };
        let authority = |artifacts| {
            LogicalViewReadAuthority::with_read_authority_and_registration_cache(
                cancellation,
                self.space.saved_read(binding),
                profile,
                ProviderDeadline::no_deadline(),
                artifacts,
            )
        };
        let analysed = super::lsp_diagnostics::analyse(
            self.registry.as_ref(),
            &authority(Arc::clone(&artifacts)),
            actor.context(),
            &body,
            cancellation,
        );
        // The analyzer reads the file when it runs: findings of a file saved
        // anew meanwhile stand on another text's lines.
        let unchanged = authority(artifacts)
            .module_text(&module)
            .is_ok_and(|text| text.trim_start_matches('\u{feff}') == saved);
        if unchanged {
            analysed
        } else {
            LspAnalysis {
                findings: Vec::new(),
                gaps: vec![LspDiagnosticGap::AnalysisNotReady],
            }
        }
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

#[cfg(test)]
mod tests;
