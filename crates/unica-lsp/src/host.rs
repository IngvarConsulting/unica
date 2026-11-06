//! Служба LSP демона над документами соединения.
//!
//! Рабочую папку допускает продукт, тем же путём, что `workspace_hint` вызова
//! MCP, поэтому редактор и агент приходят к одному актору: буферы лежат в его
//! overlay и видны только чтениям этой сессии.
//!
//! Сессия не держит актора: его держит реестр, пока в overlay есть буферы.
//! Актор, чей корень подменили, сессия при следующем обращении заменяет
//! тем, которого реестр выдаёт и MCP, и переносит в него свои буферы.

use crate::document::ContentChange;
use crate::documents::ModuleSourceRead;
use crate::documents::{LspDocumentError, LspDocuments, SourceSetLayout, SpaceLayout};
use crate::position::Utf16Position;
use crate::uri::WorkspaceFolder;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, RwLock, Weak};
use std::time::Instant;
use unica_daemon::lsp_lane::{LspCallContext, LspClientChannel, LspHost};
use unica_daemon::protocol_v5::lsp::{
    V5LspCall, V5LspCapabilities, V5LspContentChange, V5LspFailure, V5LspNotification,
    V5LspOpening, V5LspOutcome, V5LspSessionId, V5LspTextSync, V5LspWorkspaceFolder,
    V5LspWorkspaceSpace,
};
use unica_workspace::actor::WorkspaceActor;
use unica_workspace::documents::{DocumentOverlay, DocumentOverlayError, DocumentText};

/// Актор рабочей области глазами службы: его буферы и живы ли его корни.
pub trait LspActor: Send + Sync {
    fn documents(&self) -> &DocumentOverlay;
    fn retains_named_roots(&self) -> bool;
}

impl<R, C> LspActor for WorkspaceActor<R, C>
where
    Self: Send + Sync,
{
    fn documents(&self) -> &DocumentOverlay {
        WorkspaceActor::documents(self)
    }

    fn retains_named_roots(&self) -> bool {
        matches!(WorkspaceActor::retains_named_roots(self), Ok(true))
    }
}

/// Чтения модулей одной допущенной рабочей области.
pub trait LspModuleReads: Send + Sync {
    /// Чтение набора `source_set`, которое видит буферы сессии `session`.
    fn read(
        &self,
        session: &str,
        source_set: &str,
    ) -> Result<Box<dyn ModuleSourceRead>, V5LspFailure>;
}

/// Папка редактора, допущенная как рабочая область. Вызывающий держит актора,
/// только пока он ему нужен.
pub struct AdmittedWorkspace {
    pub actor: Arc<dyn LspActor>,
    pub source_sets: Vec<SourceSetLayout>,
    pub reads: Box<dyn LspModuleReads>,
}

/// Допуск папки редактора. Реализует продукт: у папки и у `workspace_hint`
/// вызова MCP одни обнаружение, наборы и реестр акторов.
pub trait LspWorkspaces: Send + Sync + 'static {
    /// `None` — папка не открывается как рабочая область Unica.
    fn admit(&self, folder: &Path, deadline: Instant) -> Option<AdmittedWorkspace>;
}

pub struct UnicaLspHost {
    workspaces: Arc<dyn LspWorkspaces>,
    sessions: Mutex<HashMap<V5LspSessionId, Arc<RwLock<Session>>>>,
}

struct Session {
    documents: LspDocuments,
    spaces: BTreeMap<String, AdmittedSpace>,
}

struct AdmittedSpace {
    folder: PathBuf,
    actor: Weak<dyn LspActor>,
    source_sets: Vec<SourceSetLayout>,
    reads: Box<dyn LspModuleReads>,
}

impl UnicaLspHost {
    pub fn new(workspaces: Arc<dyn LspWorkspaces>) -> Self {
        Self {
            workspaces,
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
                let (space, _) = admit(self.workspaces.as_ref(), &root, deadline)?;
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
                    source_sets: space.source_sets.clone(),
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
            Arc::new(RwLock::new(Session { documents, spaces })),
        );
        Ok(V5LspOpening {
            capabilities: V5LspCapabilities {
                text_document_sync: V5LspTextSync::Incremental,
                hover: false,
                text_document_content: true,
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
        let uri = notification.document_uri().to_owned();
        let Ok(location) = session.documents.locate(&uri) else {
            // Not a module of this connection: the editor may open anything,
            // the service keeps only modules.
            return Ok(());
        };
        let actor = match session.current_actor(
            self.workspaces.as_ref(),
            &location.document.space,
            deadline,
        ) {
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
        let space = call
            .document_uri()
            .and_then(|uri| read(&session).space_of(uri));
        if let Some(space) = space {
            if !read(&session).is_current(&space) {
                if let Err(failure) = write(&session).current_actor(
                    self.workspaces.as_ref(),
                    &space,
                    context.deadline(),
                ) {
                    return failed(failure);
                }
            }
        }
        if context.is_cancelled() {
            return V5LspOutcome::Cancelled {};
        }
        let session = read(&session);
        answer(&session, call).unwrap_or_else(failed)
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

impl Session {
    /// The space of the document `uri` names, if this session has it.
    fn space_of(&self, uri: &str) -> Option<String> {
        Some(self.documents.locate(uri).ok()?.document.space)
    }

    fn is_current(&self, space: &str) -> bool {
        self.spaces
            .get(space)
            .and_then(|admitted| admitted.actor.upgrade())
            .is_some_and(|actor| actor.retains_named_roots())
    }

    /// The actor of `space`, the one the registry gives MCP. An actor gone or
    /// with a replaced root is admitted again, and the session's buffers of
    /// the space move into the new one.
    fn current_actor(
        &mut self,
        workspaces: &dyn LspWorkspaces,
        space: &str,
        deadline: Instant,
    ) -> Result<Arc<dyn LspActor>, V5LspFailure> {
        let admitted = self
            .spaces
            .get_mut(space)
            .ok_or(V5LspFailure::DocumentUnknown)?;
        if let Some(actor) = admitted.actor.upgrade() {
            if actor.retains_named_roots() {
                return Ok(actor);
            }
        }
        let (renewed, actor) = admit(workspaces, &admitted.folder, deadline)
            .ok_or(V5LspFailure::WorkspaceUnavailable)?;
        // Every buffer reaches the new actor before the session switches to
        // it; a failure keeps the old one and the next request tries again.
        for set in &renewed.source_sets {
            for uri in self.documents.changed_documents(space, &set.name, None) {
                let Ok(location) = self.documents.locate(&uri) else {
                    continue;
                };
                let Some(document) = self.documents.open_document(&location.document) else {
                    continue;
                };
                match actor.documents().put(
                    self.documents.session(),
                    &set.name,
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

/// The answer to one editor request, from the document as this session
/// sees it: its open buffer, otherwise the file.
fn answer(session: &Session, call: V5LspCall) -> Result<V5LspOutcome, V5LspFailure> {
    match call {
        V5LspCall::TextDocumentContent { uri } => Ok(V5LspOutcome::Text {
            text: document_text(session, &uri)?,
        }),
        V5LspCall::Hover { .. } => Err(V5LspFailure::NotSupported),
    }
}

fn document_text(session: &Session, uri: &str) -> Result<String, V5LspFailure> {
    let location = session
        .documents
        .locate(uri)
        .map_err(|_| V5LspFailure::DocumentUnknown)?;
    let space = session
        .spaces
        .get(&location.document.space)
        .ok_or(V5LspFailure::DocumentUnknown)?;
    let read = space
        .reads
        .read(session.documents.session(), &location.source_set)?;
    session
        .documents
        .content(uri, read.as_ref())
        .map_err(|error| match error {
            LspDocumentError::Read(_) | LspDocumentError::Overlay(_) => V5LspFailure::Unavailable,
            _ => V5LspFailure::DocumentUnknown,
        })?
        .ok_or(V5LspFailure::DocumentUnknown)
}

/// The admitted space and its actor, held only while the caller needs it.
fn admit(
    workspaces: &dyn LspWorkspaces,
    folder: &Path,
    deadline: Instant,
) -> Option<(AdmittedSpace, Arc<dyn LspActor>)> {
    let admitted = workspaces.admit(folder, deadline)?;
    Some((
        AdmittedSpace {
            folder: folder.to_path_buf(),
            actor: Arc::downgrade(&admitted.actor),
            source_sets: admitted.source_sets,
            reads: admitted.reads,
        },
        admitted.actor,
    ))
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
