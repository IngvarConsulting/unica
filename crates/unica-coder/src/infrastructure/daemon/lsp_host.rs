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
use std::time::{Duration, Instant};
use unica_daemon::lsp_lane::{LspCallContext, LspClientChannel, LspHost};
use unica_daemon::protocol_v5::lsp::{
    V5LspCall, V5LspCapabilities, V5LspContentChange, V5LspFailure, V5LspNotification,
    V5LspOpening, V5LspOutcome, V5LspSessionId, V5LspTextSync, V5LspWorkspaceFolder,
    V5LspWorkspaceSpace,
};
use unica_documentation_sources::platform_help::provider::GlobalContextMember;
use unica_execution_contract::invocation::SafeIdentityHash;
use unica_source_contract::workspace::WorkspaceContext;
use unica_source_contract::{SourceFormat, SourceProfile};
use unica_workspace::actor::{ProviderRootBinding, WorkspaceSourceSetInput};
use unica_workspace::documents::{DocumentOverlay, DocumentOverlayError, DocumentText};

/// The scope string under which MCP admits a workspace. The actor of a
/// workspace is the same only for the same inputs (`actor-identity`).
const ACTOR_SCOPE: &str = "canonical-v0.13";

pub(crate) struct UnicaLspHost {
    actors: Arc<WorkspaceActorRegistry>,
    platform: Arc<PlatformContexts>,
    sessions: Mutex<HashMap<V5LspSessionId, Arc<RwLock<Session>>>>,
}

struct Session {
    documents: LspDocuments,
    spaces: BTreeMap<String, AdmittedSpace>,
}

/// A workspace folder admitted as a workspace: its actor and the bindings
/// of its source sets.
struct AdmittedSpace {
    folder: PathBuf,
    /// The platform installation whose global context completion offers.
    platform_root: Option<PathBuf>,
    actor: Weak<WorkspaceActor>,
    bindings: Vec<ProviderRootBinding>,
    identity: SafeIdentityHash,
}

impl UnicaLspHost {
    pub(crate) fn new(actors: Arc<WorkspaceActorRegistry>) -> Self {
        Self {
            actors,
            platform: Arc::new(PlatformContexts::default()),
            sessions: Mutex::new(HashMap::new()),
        }
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
        _: LspClientChannel,
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
        // The platform's global context starts loading now, not on the
        // first completion: the editor's first answers should be whole.
        for (_, space) in admitted.iter().flatten() {
            if let Some(root) = &space.platform_root {
                self.platform.members(root);
            }
        }
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
            Arc::new(RwLock::new(Session { documents, spaces })),
        );
        Ok(V5LspOpening {
            capabilities: V5LspCapabilities {
                text_document_sync: V5LspTextSync::Incremental,
                hover: false,
                text_document_content: true,
                document_symbols: true,
                folding_ranges: true,
                signature_help: true,
                completion: true,
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
        let session = self.session(session)?;
        let mut session = write(&session);
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

    fn call(
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
        let document = match &call {
            V5LspCall::CompletionResolve { item } => item["data"][COMPLETION_DOCUMENT]
                .as_str()
                .map(str::to_owned),
            call => call.document_uri().map(str::to_owned),
        };
        let space = document.and_then(|uri| read(&session).space_of(&uri));
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
        let session = read(&session);
        answer(&session, call, context, &self.platform).unwrap_or_else(failed)
    }

    fn close(&self, session: &V5LspSessionId) {
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

/// The global context of each platform installation, loaded once in the
/// background: a cold syntax-help index takes far longer than a request.
#[derive(Default)]
struct PlatformContexts {
    loaded: Mutex<HashMap<PathBuf, PlatformContext>>,
}

/// How long a failed load of a platform's global context waits before the
/// next ask tries again: the installation may still be being written.
const PLATFORM_CONTEXT_RETRY_DELAY: Duration = Duration::from_secs(60);

#[derive(Clone)]
enum PlatformContext {
    Loading,
    Ready(Arc<[GlobalContextMember]>),
    Unavailable(Instant),
}

impl PlatformContexts {
    /// The members of `root`, or `None` while they are not known; the first
    /// ask starts the load, and a failed one is retried after a delay.
    fn members(self: &Arc<Self>, root: &std::path::Path) -> Option<Arc<[GlobalContextMember]>> {
        let mut loaded = lock(&self.loaded);
        match loaded.get(root) {
            Some(PlatformContext::Ready(members)) => return Some(Arc::clone(members)),
            Some(PlatformContext::Loading) => return None,
            Some(PlatformContext::Unavailable(at))
                if at.elapsed() < PLATFORM_CONTEXT_RETRY_DELAY =>
            {
                return None
            }
            Some(PlatformContext::Unavailable(_)) | None => {}
        }
        loaded.insert(root.to_path_buf(), PlatformContext::Loading);
        drop(loaded);
        let (contexts, owned) = (Arc::clone(self), root.to_path_buf());
        let spawned = std::thread::Builder::new()
            .name("unica-lsp-global-context".into())
            .spawn(move || {
                // A panic in the help index must not leave the root loading
                // for the life of the daemon.
                let loaded = std::panic::catch_unwind(|| {
                    unica_documentation_sources::platform_help::provider::global_context_members(
                        &owned,
                    )
                });
                let context = match loaded {
                    Ok(Ok(members)) => PlatformContext::Ready(Arc::from(members)),
                    Ok(Err(error)) => {
                        eprintln!("unica lsp: platform global context is unavailable: {error}");
                        PlatformContext::Unavailable(Instant::now())
                    }
                    Err(_) => PlatformContext::Unavailable(Instant::now()),
                };
                lock(&contexts.loaded).insert(owned, context);
            });
        if spawned.is_err() {
            lock(&self.loaded).remove(root);
        }
        None
    }

    #[cfg(test)]
    fn ready_for_test(&self, root: PathBuf, members: Vec<GlobalContextMember>) {
        lock(&self.loaded).insert(root, PlatformContext::Ready(Arc::from(members)));
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
        let (mut renewed, actor, _) = admit_actor(actors, &admitted.folder, deadline)
            .ok_or(V5LspFailure::WorkspaceUnavailable)?;
        renewed.platform_root = admitted.platform_root.clone();
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
        let documents = actor
            .documents()
            .session_documents(session, set)
            .map_err(|_| V5LspFailure::Unavailable)?;
        Ok(ProviderReadAuthority::new_local_snapshot(
            binding.source_set_name(),
            format!("{}:{}", self.identity.as_str(), binding.source_set_name()),
            binding.source_kind(),
            binding.retained_root(),
            uuid::Uuid::new_v4().to_string(),
        )
        .with_artifacts(artifacts)
        .with_documents(documents))
    }
}

/// The answer to one editor request, from the document as this session
/// sees it: its open buffer, otherwise the file.
fn answer(
    session: &Session,
    call: V5LspCall,
    context: &LspCallContext,
    platform: &Arc<PlatformContexts>,
) -> Result<V5LspOutcome, V5LspFailure> {
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
            let (uri, space, read) = document_space(session, &uri)?;
            let position = Utf16Position {
                line: position.line,
                character: position.character,
            };
            // Platform methods are signed once the help is indexed.
            let members = match &space.platform_root {
                None => None,
                Some(root) => platform.members(root),
            };
            let help = session
                .documents
                .signature_help(
                    &uri,
                    position,
                    &read,
                    members.as_deref().unwrap_or_default(),
                )
                .map_err(document_failure)?;
            Ok(json(help.unwrap_or(serde_json::Value::Null)))
        }
        V5LspCall::Completion { uri, position } => {
            let (uri, space, read) = document_space(session, &uri)?;
            let actor = space
                .actor
                .upgrade()
                .ok_or(V5LspFailure::WorkspaceUnavailable)?;
            // Without the platform's global context the list lacks its
            // methods: it is marked incomplete, so the editor asks again.
            // No installation at all: nothing to wait for, the list is whole.
            let members = match &space.platform_root {
                None => Some(Arc::from(Vec::new())),
                Some(root) => platform.members(root),
            };
            let list = session
                .documents
                .completion(
                    actor.documents(),
                    &uri,
                    Utf16Position {
                        line: position.line,
                        character: position.character,
                    },
                    &read,
                    members.as_deref().unwrap_or_default(),
                    &|| context.is_cancelled(),
                )
                .map_err(document_failure)?;
            if context.is_cancelled() {
                return Ok(V5LspOutcome::Cancelled {});
            }
            Ok(json(match list {
                Some(list) => completion_list(list, &uri, members.is_none()),
                None => serde_json::Value::Null,
            }))
        }
        V5LspCall::CompletionResolve { item } => {
            // An item this service did not give comes back as it was.
            let Some(uri) = item["data"][COMPLETION_DOCUMENT]
                .as_str()
                .map(str::to_owned)
            else {
                return Ok(json(item));
            };
            let (uri, read) = document_read(session, &uri)?;
            session
                .documents
                .resolve_completion(&uri, item, &read)
                .map(json)
                .map_err(document_failure)
        }
        V5LspCall::Hover { .. } => Err(V5LspFailure::NotSupported),
    }
}

/// Where a completion item keeps its document for `completionItem/resolve`:
/// the editor returns the item as it got it.
const COMPLETION_DOCUMENT: &str = "unicaDocument";

fn completion_list(mut list: serde_json::Value, uri: &str, incomplete: bool) -> serde_json::Value {
    if incomplete {
        list["isIncomplete"] = serde_json::Value::Bool(true);
    }
    if let Some(items) = list["items"].as_array_mut() {
        for item in items {
            if !item["data"].is_object() {
                item["data"] = serde_json::json!({});
            }
            item["data"][COMPLETION_DOCUMENT] = serde_json::Value::String(uri.to_owned());
        }
    }
    list
}

/// The `unica://` URI of a document and a read of its source set that sees
/// this session's buffers.
fn document_read(
    session: &Session,
    uri: &str,
) -> Result<(String, ProviderReadAuthority), V5LspFailure> {
    document_space(session, uri).map(|(uri, _, read)| (uri, read))
}

fn document_space<'a>(
    session: &'a Session,
    uri: &str,
) -> Result<(String, &'a AdmittedSpace, ProviderReadAuthority), V5LspFailure> {
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
    Ok((uri, space, read))
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
    let (mut space, _, context) = admit_actor(actors, folder, deadline)?;
    space.platform_root =
        crate::infrastructure::application_ports::platform_installation_root(&context);
    Some(space)
}

/// The admitted space and its actor, held only while the caller needs it.
fn admit_actor(
    actors: &WorkspaceActorRegistry,
    folder: &std::path::Path,
    deadline: Instant,
) -> Option<(AdmittedSpace, Arc<WorkspaceActor>, WorkspaceContext)> {
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
            platform_root: None,
            actor: Arc::downgrade(&actor),
            bindings,
            identity,
        },
        actor,
        context,
    ))
}

/// The `unica://` URI of a document: a file path never names one.
fn document_uri(documents: &LspDocuments, uri: &str) -> Result<String, LspDocumentError> {
    documents.locate(uri).map(|_| uri.to_owned())
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
