//! Служба LSP демона над документами соединения (#2173, #2174).
//!
//! Рабочая папка редактора допускается так же, как `workspace_hint` вызова
//! MCP, и получает того же актора из общего реестра: буферы редактора
//! лежат в его overlay и видны только чтениям этой сессии. Чтение строится
//! из привязки, которую выдал актор, а не из путей редактора.
//!
//! Сессия не держит актора: его держит реестр, пока в overlay есть буферы.
//! Актор, чей корень подменили, сессия при следующем обращении заменяет
//! тем, которого реестр выдаёт и MCP, и переносит в него свои буферы.

use super::server::closed_daemon_source_relative_path;
use crate::application::lsp::document::ContentChange;
use crate::application::lsp::position::Utf16Position;
use crate::application::lsp::uri::WorkspaceFolder;
use crate::infrastructure::lsp_diagnostics;
use crate::infrastructure::lsp_documents::{
    LspDocumentError, LspDocuments, SourceSetLayout, SpaceLayout,
};
use crate::infrastructure::lsp_features;
use crate::infrastructure::source_workspace::{
    discover_project_source_admission, WorkspaceActor, WorkspaceActorRegistry,
};
use crate::infrastructure::v13_read_port::ProviderReadAuthority;
use crate::infrastructure::workspace::discover_workspace;
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, RwLock, Weak};
use std::time::Instant;
use unica_daemon::lsp_lane::{LspCallContext, LspClientChannel, LspHost};
use unica_daemon::protocol_v5::lsp::{
    V5LspCall, V5LspCapabilities, V5LspContentChange, V5LspDiagnosticsDelivery, V5LspFailure,
    V5LspNotification, V5LspOpening, V5LspOutcome, V5LspServerNotification, V5LspSessionId,
    V5LspTextSync, V5LspWorkspaceFolder, V5LspWorkspaceSpace,
};
use unica_execution_contract::invocation::SafeIdentityHash;
use unica_source_contract::{SourceFormat, SourceProfile};
use unica_workspace::actor::{ProviderRootBinding, WorkspaceSourceSetInput};
use unica_workspace::documents::{DocumentOverlay, DocumentOverlayError, DocumentText};

/// The scope string under which MCP admits a workspace. The actor of a
/// workspace is the same only for the same inputs (`actor-identity`).
const ACTOR_SCOPE: &str = "canonical-v0.13";

pub(crate) struct UnicaLspHost {
    actors: Arc<WorkspaceActorRegistry>,
    sessions: Mutex<HashMap<V5LspSessionId, Arc<RwLock<Session>>>>,
    diagnostics: EditorDiagnostics,
}

struct Session {
    documents: LspDocuments,
    spaces: BTreeMap<String, AdmittedSpace>,
    /// Where the service tells the editor of finished diagnostics.
    client: LspClientChannel,
    /// Held while diagnostics are built and sent, so that an older state
    /// never reaches the editor after a newer one.
    publishing: Arc<Mutex<()>>,
}

/// The diagnostic providers of the editor's code analysis.
type DiagnosticRegistry =
    dyn Fn() -> Result<unica_diagnostics::model::DiagnosticProviderRegistry, String> + Send + Sync;

/// Diagnostics of documents (#2176): Unica's own checks over the buffer, the
/// provider's analysis of the saved file in the background, in Unica's words.
#[derive(Clone)]
struct EditorDiagnostics {
    analysis: Arc<lsp_diagnostics::EditorAnalysis>,
    registry: Arc<DiagnosticRegistry>,
}

impl EditorDiagnostics {
    fn production() -> Self {
        Self {
            analysis: Arc::default(),
            registry: Arc::new(|| {
                unica_diagnostics::model::DiagnosticProviderRegistry::new(vec![Arc::new(
                    crate::infrastructure::analysis_composition::diagnostic_provider(),
                )])
                .map_err(|error| error.to_string())
            }),
        }
    }

    /// The document's text, version and diagnostics as this session sees
    /// them. A provider analysis not yet finished is started once; when it
    /// finishes, the editor gets `publishDiagnostics` for `client_uri`.
    fn report(
        &self,
        session: &Arc<RwLock<Session>>,
        client_uri: &str,
    ) -> Result<DocumentReport, V5LspFailure> {
        let guard = read(session);
        let uri = document_uri(&guard.documents, client_uri)
            .map_err(|_| V5LspFailure::DocumentUnknown)?;
        let location = guard
            .documents
            .locate(&uri)
            .map_err(|_| V5LspFailure::DocumentUnknown)?;
        let space = guard
            .spaces
            .get(&location.document.space)
            .ok_or(V5LspFailure::DocumentUnknown)?;
        let buffer_read = space.read(guard.documents.session(), &location.source_set)?;
        let text = document_text(&guard, &uri, &buffer_read)?;
        let version = guard
            .documents
            .open_document(&location.document)
            .map(|document| document.version());
        let (_, saved_read) = space.saved_read(&location.source_set)?;
        let saved = guard
            .documents
            .saved_content(&uri, &saved_read)
            .map_err(document_failure)?;
        let own = lsp_diagnostics::own_findings(&text);
        let Some(saved) = saved else {
            // A module not yet on disk has nothing the provider could read.
            let provider = lsp_diagnostics::DocumentDiagnostics {
                findings: Vec::new(),
                gaps: vec![lsp_diagnostics::DiagnosticGap::UnsavedBuffer],
            };
            return Ok(DocumentReport {
                diagnostics: lsp_diagnostics::merge(own, provider),
                text,
                version,
            });
        };
        let key = lsp_diagnostics::AnalysisKey::new(
            space.identity.as_str(),
            location.document.address.clone(),
            saved.as_bytes(),
        );
        let profile = space.profile(&location.source_set);
        let folder = space.folder.clone();
        let address = location.document.address.clone();
        let registry = Arc::clone(&self.registry);
        // One waiter per document of a session, however often it is asked.
        let waiter = format!("{}\n{client_uri}", guard.documents.session());
        let ready: lsp_diagnostics::AnalysisWaiter = {
            let service = self.clone();
            let session = Arc::downgrade(session);
            let client_uri = client_uri.to_owned();
            Arc::new(move || {
                if let Some(session) = session.upgrade() {
                    service.publish(&session, &client_uri, Published::Analysed);
                }
            })
        };
        drop(guard);
        let analysed = self.analysis.findings(
            key,
            &waiter,
            move |cancellation| {
                analyse(
                    registry.as_ref(),
                    saved_read,
                    profile,
                    &folder,
                    &address,
                    cancellation,
                )
            },
            ready,
        );
        let mut provider = analysed.unwrap_or_else(|| lsp_diagnostics::DocumentDiagnostics {
            findings: Vec::new(),
            gaps: vec![lsp_diagnostics::DiagnosticGap::AnalysisNotReady],
        });
        if text != saved {
            // The analysis read the saved file: its findings may stand on
            // lines the buffer moved. Unica's own checks follow the buffer.
            provider.findings.clear();
            provider
                .gaps
                .push(lsp_diagnostics::DiagnosticGap::UnsavedBuffer);
        }
        Ok(DocumentReport {
            diagnostics: lsp_diagnostics::merge(own, provider),
            text,
            version,
        })
    }

    /// Tell the editor the current diagnostics of `client_uri`. A closed
    /// document is told it has none; a finished analysis of a document
    /// closed since is not told at all. An editor that asks for diagnostics
    /// itself is told only of finished analyses, so that it asks again.
    fn publish(&self, session: &Arc<RwLock<Session>>, client_uri: &str, cause: Published) {
        let (client, publishing) = {
            let guard = read(session);
            (guard.client.clone(), Arc::clone(&guard.publishing))
        };
        if cause != Published::Analysed
            && client.diagnostics_delivery() == V5LspDiagnosticsDelivery::Pull
        {
            return;
        }
        let _publishing = lock(&publishing);
        // Read under the publishing lock: a close published meanwhile is not
        // followed by the closed document's findings.
        let open = {
            let guard = read(session);
            document_uri(&guard.documents, client_uri)
                .ok()
                .and_then(|uri| guard.documents.locate(&uri).ok())
                .map(|location| guard.documents.open_document(&location.document).is_some())
        };
        let params = match (cause, open) {
            (Published::Closed, Some(false)) => lsp_diagnostics::closed_json(client_uri),
            (_, Some(true)) if cause != Published::Closed => {
                let Ok(report) = self.report(session, client_uri) else {
                    return;
                };
                lsp_diagnostics::publish_json(
                    client_uri,
                    report.version,
                    &report.text,
                    &report.diagnostics,
                )
            }
            // Not a module, closed before its analysis ended, or opened
            // again since it closed: that opening publishes for itself.
            _ => return,
        };
        let _ = client.push(V5LspServerNotification::PublishDiagnostics {
            uri: client_uri.to_owned(),
            params,
            analysed: cause == Published::Analysed,
        });
    }
}

/// What the editor is told diagnostics about.
struct DocumentReport {
    text: String,
    version: Option<i32>,
    diagnostics: lsp_diagnostics::DocumentDiagnostics,
}

/// Why diagnostics of a document are published.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Published {
    /// The editor opened, changed or saved it.
    Edited,
    /// The background analysis of its saved text finished.
    Analysed,
    /// The editor closed it.
    Closed,
}

/// The provider's analysis of one saved module, as `check` runs it: the same
/// mapping and coordinator, `Analyze` on the module alone and no deadline
/// (#2189). A failure to prove the module is a gap, not an answer.
fn analyse(
    registry: &DiagnosticRegistry,
    saved_read: ProviderReadAuthority,
    profile: Option<unica_bsl::profile::PlatformProfile>,
    folder: &std::path::Path,
    address: &str,
    cancellation: &unica_work_control::cancellation::CancellationToken,
) -> lsp_diagnostics::DocumentDiagnostics {
    use unica_diagnostics::coordinator::DiagnosticCoordinator;
    use unica_diagnostics::model::{DiagnosticAction, DiagnosticFilter, DiagnosticRequest};
    use unica_work_control::provider_deadline::ProviderDeadline;
    let unavailable = || lsp_diagnostics::DocumentDiagnostics {
        findings: Vec::new(),
        gaps: vec![lsp_diagnostics::DiagnosticGap::AnalysisUnavailable],
    };
    let (Some(profile), Ok(context), Ok(at)) = (
        profile,
        discover_workspace(Some(folder.to_path_buf())),
        unica_address::address::QualifiedAddress::parse(address),
    ) else {
        return unavailable();
    };
    let logical = crate::infrastructure::v13_read::LogicalViewReadAuthority::with_read_authority(
        cancellation,
        saved_read,
        profile,
        ProviderDeadline::no_deadline(),
    );
    let Ok(mapping) = logical.diagnostic_mapping(&at, &context) else {
        return unavailable();
    };
    // The provider is given this module alone; without its scope it would
    // analyse the whole workspace.
    let Some(scope) = mapping.module_scope() else {
        return unavailable();
    };
    let (Ok(coordinated), Ok(translated)) = (registry(), registry()) else {
        return unavailable();
    };
    let request = DiagnosticRequest {
        action: DiagnosticAction::Analyze,
        source_set: at.source_set().to_string(),
        metadata_path: mapping.target().metadata_path.clone(),
        // The editor shows every level and lets its user filter them; `check`
        // keeps its default of warnings and above.
        filter: DiagnosticFilter {
            min_severity: None,
            ..DiagnosticFilter::default()
        },
        range: None,
        limit: usize::MAX,
        timeout: None,
    };
    match DiagnosticCoordinator::new(coordinated, &mapping).execute_scoped(
        &request,
        &context,
        cancellation,
        Some((scope, ProviderDeadline::no_deadline())),
    ) {
        Ok(result) => lsp_diagnostics::provider_diagnostics(&result, &translated),
        Err(_) => unavailable(),
    }
}

/// A workspace folder admitted as a workspace: its actor and the bindings
/// of its source sets.
struct AdmittedSpace {
    folder: PathBuf,
    actor: Weak<WorkspaceActor>,
    bindings: Vec<ProviderRootBinding>,
    identity: SafeIdentityHash,
}

impl UnicaLspHost {
    pub(crate) fn new(actors: Arc<WorkspaceActorRegistry>) -> Self {
        Self {
            actors,
            sessions: Mutex::new(HashMap::new()),
            diagnostics: EditorDiagnostics::production(),
        }
    }

    /// The service with these diagnostic providers in place of the bundled
    /// analyzer.
    #[cfg(test)]
    pub(crate) fn with_diagnostic_providers_for_test(
        mut self,
        providers: impl Fn() -> Vec<Arc<dyn unica_diagnostics::model::DiagnosticProvider>>
            + Send
            + Sync
            + 'static,
    ) -> Self {
        self.diagnostics.registry = Arc::new(move || {
            unica_diagnostics::model::DiagnosticProviderRegistry::new(providers())
                .map_err(|error| error.to_string())
        });
        self
    }

    fn session(&self, session: &V5LspSessionId) -> Result<Arc<RwLock<Session>>, V5LspFailure> {
        lock(&self.sessions)
            .get(session)
            .cloned()
            .ok_or(V5LspFailure::SessionUnknown)
    }
}

impl LspHost for UnicaLspHost {
    fn open(
        &self,
        session: &V5LspSessionId,
        folders: &[V5LspWorkspaceFolder],
        client: LspClientChannel,
        deadline: Instant,
    ) -> Result<V5LspOpening, V5LspFailure> {
        // A folder without admitted sources takes no space name.
        let admitted = folders
            .iter()
            .map(|folder| {
                let root = folder_path(&folder.uri)?;
                let space = admit(&self.actors, &root, deadline)?;
                Some((root, space))
            })
            .collect::<Vec<_>>();
        let layouts = folders
            .iter()
            .zip(&admitted)
            .filter_map(|(folder, admitted)| {
                let (root, space) = admitted.as_ref()?;
                Some(SpaceLayout {
                    folder: WorkspaceFolder {
                        name: folder.name.clone(),
                        root: root.clone(),
                    },
                    source_sets: space.layouts(),
                })
            })
            .collect();
        let documents = LspDocuments::new(session.as_str(), layouts);
        let mut spaces = BTreeMap::new();
        let mut workspaces = Vec::with_capacity(folders.len());
        for (folder, admitted) in folders.iter().zip(admitted) {
            let named = admitted.and_then(|(root, space)| {
                Some((documents.spaces().space_of_root(&root)?.to_owned(), space))
            });
            match named {
                Some((name, space)) => {
                    workspaces.push(V5LspWorkspaceSpace::opened(&folder.uri, &name));
                    spaces.insert(name, space);
                }
                None => workspaces.push(V5LspWorkspaceSpace::refused(
                    &folder.uri,
                    V5LspFailure::WorkspaceUnavailable,
                )),
            }
        }
        lock(&self.sessions).insert(
            session.clone(),
            Arc::new(RwLock::new(Session {
                documents,
                spaces,
                client,
                publishing: Arc::default(),
            })),
        );
        Ok(V5LspOpening {
            capabilities: V5LspCapabilities {
                text_document_sync: V5LspTextSync::Incremental,
                hover: false,
                text_document_content: true,
                document_symbols: true,
                folding_ranges: true,
                signature_help: true,
                diagnostics: true,
                semantic_tokens_legend: Some(lsp_features::semantic_tokens_legend()),
            },
            workspaces,
        })
    }

    fn notify(
        &self,
        session: &V5LspSessionId,
        notification: V5LspNotification,
        deadline: Instant,
    ) -> Result<(), V5LspFailure> {
        let session_arc = self.session(session)?;
        let client_uri = notification.document_uri().to_owned();
        let cause = match notification {
            V5LspNotification::DidClose { .. } => Published::Closed,
            _ => Published::Edited,
        };
        let changed = self.apply_notification(&session_arc, notification, deadline);
        // A new version or a new saved text is told its diagnostics: Unica's
        // own at once, the provider's when its analysis of the saved file is
        // ready (#2176).
        if changed.is_ok() {
            self.diagnostics.publish(&session_arc, &client_uri, cause);
        }
        changed
    }

    fn call(
        &self,
        session: &V5LspSessionId,
        call: V5LspCall,
        context: &LspCallContext,
    ) -> V5LspOutcome {
        self.answer_call(session, call, context)
    }

    fn close(&self, session: &V5LspSessionId) {
        self.close_session(session);
    }
}

impl UnicaLspHost {
    fn apply_notification(
        &self,
        session: &Arc<RwLock<Session>>,
        notification: V5LspNotification,
        deadline: Instant,
    ) -> Result<(), V5LspFailure> {
        let mut session = write(session);
        let Ok(uri) = document_uri(&session.documents, notification.document_uri()) else {
            // A file that is no module of this connection: the editor may
            // open anything, the service keeps only modules.
            return Ok(());
        };
        let Ok(location) = session.documents.locate(&uri) else {
            return Ok(());
        };
        let actor = match session.current_actor(&self.actors, &location.document.space, deadline) {
            Ok(actor) => actor,
            Err(failure) => {
                // Without its actor the session cannot keep the editor's
                // text: the document is dropped here and refused, so no
                // answer is given from a buffer that missed this change.
                let _ = session.documents.close(&DocumentOverlay::default(), &uri);
                return match notification {
                    V5LspNotification::DidClose { .. } => Ok(()),
                    _ => Err(failure),
                };
            }
        };
        let overlay = actor.documents();
        let documents = &mut session.documents;
        let applied = match notification {
            V5LspNotification::DidOpen { version, text, .. } => {
                documents.open(overlay, &uri, version, text).map(drop)
            }
            V5LspNotification::DidChange {
                version, changes, ..
            } => {
                let changes = changes.into_iter().map(content_change).collect::<Vec<_>>();
                documents.change(overlay, &uri, version, &changes).map(drop)
            }
            // The saved text is read from disk when diagnostics are asked.
            V5LspNotification::DidSave { .. } => Ok(()),
            V5LspNotification::DidClose { .. } => {
                let _ = documents.close(overlay, &uri);
                return Ok(());
            }
        };
        // A text the buffer cannot take would leave it apart from the
        // editor's: the document is closed here and the refusal makes the
        // frontend hold its requests until the editor opens it again.
        applied.map_err(|error| {
            let _ = documents.close(overlay, &uri);
            match error {
                LspDocumentError::Overlay(_) => V5LspFailure::Unavailable,
                _ => V5LspFailure::DocumentUnknown,
            }
        })
    }

    fn answer_call(
        &self,
        session: &V5LspSessionId,
        call: V5LspCall,
        context: &LspCallContext,
    ) -> V5LspOutcome {
        let failed = |failure| V5LspOutcome::Failed { failure };
        let session = match self.session(session) {
            Ok(session) => session,
            Err(failure) => return failed(failure),
        };
        if context.is_cancelled() {
            return V5LspOutcome::Cancelled {};
        }
        // A replaced actor is swapped under the write lock; the answer itself
        // reads under the read lock, beside other requests of the session.
        let space = call
            .document_uri()
            .and_then(|uri| read(&session).space_of(uri));
        if let Some(space) = space {
            if !read(&session).is_current(&space) {
                if let Err(failure) =
                    write(&session).current_actor(&self.actors, &space, context.deadline())
                {
                    return failed(failure);
                }
            }
        }
        if context.is_cancelled() {
            return V5LspOutcome::Cancelled {};
        }
        if let V5LspCall::Diagnostic {
            uri,
            previous_result_id,
        } = &call
        {
            return match self.diagnostics.report(&session, uri) {
                Ok(report) => V5LspOutcome::Json {
                    value: lsp_diagnostics::document_report_json(
                        &report.text,
                        &report.diagnostics,
                        previous_result_id.as_deref(),
                    ),
                },
                Err(failure) => failed(failure),
            };
        }
        let session = read(&session);
        answer(&session, call).unwrap_or_else(failed)
    }

    fn close_session(&self, session: &V5LspSessionId) {
        let Some(session) = lock(&self.sessions).remove(session) else {
            return;
        };
        let session = read(&session);
        // An open buffer keeps its actor from eviction: every actor of the
        // session lets go of this session's buffers.
        for actor in session
            .spaces
            .values()
            .filter_map(|space| space.actor.upgrade())
        {
            let _ = actor.documents().close_session(session.documents.session());
        }
    }
}

impl Session {
    /// The space of the document `uri` names, if this session has it.
    fn space_of(&self, uri: &str) -> Option<String> {
        let uri = document_uri(&self.documents, uri).ok()?;
        Some(self.documents.locate(&uri).ok()?.document.space)
    }

    fn is_current(&self, space: &str) -> bool {
        self.spaces
            .get(space)
            .and_then(|admitted| admitted.actor.upgrade())
            .is_some_and(|actor| matches!(actor.retains_named_roots(), Ok(true)))
    }

    /// The actor of `space`, the one the registry gives MCP. An actor gone or
    /// with a replaced root is admitted again, and the session's buffers of
    /// the space move into the new one.
    fn current_actor(
        &mut self,
        actors: &WorkspaceActorRegistry,
        space: &str,
        deadline: Instant,
    ) -> Result<Arc<WorkspaceActor>, V5LspFailure> {
        let admitted = self
            .spaces
            .get_mut(space)
            .ok_or(V5LspFailure::DocumentUnknown)?;
        if let Some(actor) = admitted.actor.upgrade() {
            if matches!(actor.retains_named_roots(), Ok(true)) {
                return Ok(actor);
            }
        }
        let (renewed, actor) = admit_actor(actors, &admitted.folder, deadline)
            .ok_or(V5LspFailure::WorkspaceUnavailable)?;
        // Every buffer reaches the new actor before the session switches to
        // it; a failure keeps the old one and the next request tries again.
        for binding in &renewed.bindings {
            let set = binding.source_set_name();
            for uri in self.documents.changed_documents(space, set, None) {
                let Ok(location) = self.documents.locate(&uri) else {
                    continue;
                };
                let Some(document) = self.documents.open_document(&location.document) else {
                    continue;
                };
                match actor.documents().put(
                    self.documents.session(),
                    set,
                    &location.relative,
                    DocumentText {
                        version: document.version(),
                        text: Arc::from(document.text()),
                    },
                ) {
                    // The registry gave back an actor still held by a call:
                    // it already has this buffer.
                    Ok(()) | Err(DocumentOverlayError::StaleVersion { .. }) => {}
                    Err(_) => return Err(V5LspFailure::Unavailable),
                }
            }
        }
        if let Some(admitted) = self.spaces.get_mut(space) {
            *admitted = renewed;
        }
        Ok(actor)
    }
}

impl AdmittedSpace {
    fn layouts(&self) -> Vec<SourceSetLayout> {
        self.bindings
            .iter()
            .filter_map(|binding| {
                Some(SourceSetLayout {
                    name: binding.source_set_name().to_owned(),
                    kind: binding.source_kind(),
                    root: binding.source_root().to_path_buf(),
                    profile: crate::domain::source_capabilities::bsl_profile_for_source(
                        binding.source_profile(),
                    )?,
                })
            })
            .collect()
    }

    /// A read of `set` that sees this session's buffers in place of files.
    fn read(&self, session: &str, set: &str) -> Result<ProviderReadAuthority, V5LspFailure> {
        let (actor, read) = self.saved_read(set)?;
        let documents = actor
            .documents()
            .session_documents(session, set)
            .map_err(|_| V5LspFailure::Unavailable)?;
        Ok(read.with_documents(documents))
    }

    /// A read of `set` as saved on disk, without any editor's buffers, and
    /// its actor.
    fn saved_read(
        &self,
        set: &str,
    ) -> Result<(Arc<WorkspaceActor>, ProviderReadAuthority), V5LspFailure> {
        let binding = self
            .bindings
            .iter()
            .find(|binding| binding.source_set_name() == set)
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
        let read = ProviderReadAuthority::new_local_snapshot(
            binding.source_set_name(),
            format!("{}:{}", self.identity.as_str(), binding.source_set_name()),
            binding.source_kind(),
            binding.retained_root(),
            uuid::Uuid::new_v4().to_string(),
        )
        .with_artifacts(artifacts);
        Ok((actor, read))
    }

    /// The platform profile of `set`, which its logical reader serves.
    fn profile(&self, set: &str) -> Option<unica_bsl::profile::PlatformProfile> {
        let binding = self
            .bindings
            .iter()
            .find(|binding| binding.source_set_name() == set)?;
        crate::domain::source_capabilities::bsl_profile_for_source(binding.source_profile())
    }
}

/// The answer to one editor request, from the document as this session
/// sees it: its open buffer, otherwise the file.
fn answer(session: &Session, call: V5LspCall) -> Result<V5LspOutcome, V5LspFailure> {
    let json = |value| V5LspOutcome::Json { value };
    let text_with = |uri: &str, build: fn(&str) -> serde_json::Value| {
        let (uri, read) = document_read(session, uri)?;
        Ok(json(build(&document_text(session, &uri, &read)?)))
    };
    match call {
        V5LspCall::TextDocumentContent { uri } => {
            let (uri, read) = document_read(session, &uri)?;
            Ok(V5LspOutcome::Text {
                text: document_text(session, &uri, &read)?,
            })
        }
        V5LspCall::DocumentSymbol { uri } => text_with(&uri, lsp_features::document_symbols),
        V5LspCall::FoldingRange { uri } => text_with(&uri, lsp_features::folding_ranges),
        V5LspCall::SemanticTokensFull { uri } => text_with(&uri, lsp_features::semantic_tokens),
        V5LspCall::SignatureHelp { uri, position } => {
            let (uri, read) = document_read(session, &uri)?;
            let position = Utf16Position {
                line: position.line,
                character: position.character,
            };
            let help = session
                .documents
                .signature_help(&uri, position, &read)
                .map_err(document_failure)?;
            Ok(json(help.unwrap_or(serde_json::Value::Null)))
        }
        V5LspCall::Hover { .. } => Err(V5LspFailure::NotSupported),
        // Answered with the session's diagnostics service before this point.
        V5LspCall::Diagnostic { .. } => Err(V5LspFailure::NotSupported),
    }
}

/// The `unica://` URI of a document and a read of its source set that sees
/// this session's buffers.
fn document_read(
    session: &Session,
    uri: &str,
) -> Result<(String, ProviderReadAuthority), V5LspFailure> {
    let uri = document_uri(&session.documents, uri).map_err(|_| V5LspFailure::DocumentUnknown)?;
    let location = session
        .documents
        .locate(&uri)
        .map_err(|_| V5LspFailure::DocumentUnknown)?;
    let space = session
        .spaces
        .get(&location.document.space)
        .ok_or(V5LspFailure::DocumentUnknown)?;
    let read = space.read(session.documents.session(), &location.source_set)?;
    Ok((uri, read))
}

fn document_text(
    session: &Session,
    uri: &str,
    read: &ProviderReadAuthority,
) -> Result<String, V5LspFailure> {
    session
        .documents
        .content(uri, read)
        .map_err(document_failure)?
        .ok_or(V5LspFailure::DocumentUnknown)
}

fn document_failure(error: LspDocumentError) -> V5LspFailure {
    match error {
        LspDocumentError::Read(_) | LspDocumentError::Overlay(_) => V5LspFailure::Unavailable,
        _ => V5LspFailure::DocumentUnknown,
    }
}

/// Admit a workspace folder the way MCP admits its `workspace_hint`: the
/// same discovery, the same platform-XML source sets in name order and the
/// same actor scope, so both reach one actor.
fn admit(
    actors: &WorkspaceActorRegistry,
    folder: &std::path::Path,
    deadline: Instant,
) -> Option<AdmittedSpace> {
    admit_actor(actors, folder, deadline).map(|(space, _)| space)
}

/// The admitted space and its actor, held only while the caller needs it.
fn admit_actor(
    actors: &WorkspaceActorRegistry,
    folder: &std::path::Path,
    deadline: Instant,
) -> Option<(AdmittedSpace, Arc<WorkspaceActor>)> {
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
    let actor = actors
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
    let identity = actor.safe_identity_hash().ok()?;
    Some((
        AdmittedSpace {
            folder: folder.to_path_buf(),
            actor: Arc::downgrade(&actor),
            bindings,
            identity,
        },
        actor,
    ))
}

/// The `unica://` URI of a document the editor may name by its file.
fn document_uri(documents: &LspDocuments, uri: &str) -> Result<String, LspDocumentError> {
    if uri.starts_with("file:") {
        documents.uri_of_file(uri)
    } else {
        Ok(uri.to_owned())
    }
}

fn folder_path(uri: &str) -> Option<PathBuf> {
    url::Url::parse(uri)
        .ok()
        .filter(|url| url.scheme() == "file")?
        .to_file_path()
        .ok()
}

fn content_change(change: V5LspContentChange) -> ContentChange {
    match change.range {
        None => ContentChange::Full(change.text),
        Some(range) => ContentChange::Range {
            start: Utf16Position {
                line: range.start.line,
                character: range.start.character,
            },
            end: Utf16Position {
                line: range.end.line,
                character: range.end.character,
            },
            text: change.text,
        },
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn read<T>(lock: &RwLock<T>) -> std::sync::RwLockReadGuard<'_, T> {
    lock.read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn write<T>(lock: &RwLock<T>) -> std::sync::RwLockWriteGuard<'_, T> {
    lock.write()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
#[path = "../../../tests/platform/lsp_host.rs"]
mod tests;

#[cfg(test)]
#[path = "../../../tests/platform/lsp_host_diagnostics.rs"]
mod diagnostics_tests;
