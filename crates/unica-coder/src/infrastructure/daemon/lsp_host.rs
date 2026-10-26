//! Служба LSP демона над документами соединения (#2173, #2174).
//!
//! Рабочая папка редактора допускается так же, как `workspace_hint` вызова
//! MCP, и получает того же актора из общего реестра: буферы редактора
//! лежат в его overlay и видны только чтениям этой сессии. Чтение строится
//! из привязки, которую выдал актор, а не из путей редактора.

use super::server::closed_daemon_source_relative_path;
use crate::application::lsp::document::ContentChange;
use crate::application::lsp::position::Utf16Position;
use crate::application::lsp::uri::WorkspaceFolder;
use crate::infrastructure::lsp_documents::{
    LspDocumentError, LspDocuments, SourceSetLayout, SpaceLayout,
};
use crate::infrastructure::source_workspace::{
    discover_project_source_admission, WorkspaceActor, WorkspaceActorRegistry,
};
use crate::infrastructure::v13_read_port::ProviderReadAuthority;
use crate::infrastructure::workspace::discover_workspace;
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Instant;
use unica_daemon::lsp_lane::{LspCallContext, LspClientChannel, LspHost};
use unica_daemon::protocol_v5::lsp::{
    V5LspCall, V5LspCapabilities, V5LspContentChange, V5LspFailure, V5LspNotification,
    V5LspOpening, V5LspOutcome, V5LspSessionId, V5LspTextSync, V5LspWorkspaceFolder,
    V5LspWorkspaceSpace,
};
use unica_execution_contract::invocation::SafeIdentityHash;
use unica_source_contract::{SourceFormat, SourceProfile};
use unica_workspace::actor::{ProviderRootBinding, WorkspaceSourceSetInput};

/// The scope string under which MCP admits a workspace. The actor of a
/// workspace is the same only for the same inputs (`actor-identity`).
const ACTOR_SCOPE: &str = "canonical-v0.13";

pub(crate) struct UnicaLspHost {
    actors: Arc<WorkspaceActorRegistry>,
    sessions: Mutex<HashMap<V5LspSessionId, Arc<Mutex<Session>>>>,
}

struct Session {
    documents: LspDocuments,
    spaces: BTreeMap<String, AdmittedSpace>,
}

/// A workspace folder admitted as a workspace: its actor and the bindings
/// of its source sets.
struct AdmittedSpace {
    actor: Arc<WorkspaceActor>,
    bindings: Vec<ProviderRootBinding>,
    identity: SafeIdentityHash,
}

impl UnicaLspHost {
    pub(crate) fn new(actors: Arc<WorkspaceActorRegistry>) -> Self {
        Self {
            actors,
            sessions: Mutex::new(HashMap::new()),
        }
    }

    fn session(&self, session: &V5LspSessionId) -> Result<Arc<Mutex<Session>>, V5LspFailure> {
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
        _: Instant,
    ) -> Result<V5LspOpening, V5LspFailure> {
        // A folder without admitted sources takes no space name.
        let admitted = folders
            .iter()
            .map(|folder| {
                let root = folder_path(&folder.uri)?;
                let space = admit(&self.actors, &root)?;
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
            Arc::new(Mutex::new(Session { documents, spaces })),
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
        _: Instant,
    ) -> Result<(), V5LspFailure> {
        let session = self.session(session)?;
        let mut session = lock(&session);
        let session = &mut *session;
        let Ok(uri) = document_uri(&session.documents, notification.document_uri()) else {
            // A file that is no module of this connection: the editor may
            // open anything, the service keeps only modules.
            return Ok(());
        };
        let Ok(location) = session.documents.locate(&uri) else {
            return Ok(());
        };
        let Some(space) = session.spaces.get(&location.document.space) else {
            return Ok(());
        };
        let overlay = space.actor.documents();
        let applied = match notification {
            V5LspNotification::DidOpen { version, text, .. } => session
                .documents
                .open(overlay, &uri, version, text)
                .map(drop),
            V5LspNotification::DidChange {
                version, changes, ..
            } => {
                let changes = changes.into_iter().map(content_change).collect::<Vec<_>>();
                session
                    .documents
                    .change(overlay, &uri, version, &changes)
                    .map(drop)
            }
            V5LspNotification::DidClose { .. } => session.documents.close(overlay, &uri).map(drop),
        };
        match applied {
            Err(LspDocumentError::Overlay(_)) => Err(V5LspFailure::Unavailable),
            // A change the buffer cannot take leaves the last good version;
            // the editor's next full sync replaces it.
            _ => Ok(()),
        }
    }

    fn call(&self, session: &V5LspSessionId, call: V5LspCall, _: &LspCallContext) -> V5LspOutcome {
        let failed = |failure| V5LspOutcome::Failed { failure };
        let V5LspCall::TextDocumentContent { uri } = call else {
            return failed(V5LspFailure::NotSupported);
        };
        let session = match self.session(session) {
            Ok(session) => session,
            Err(failure) => return failed(failure),
        };
        let session = lock(&session);
        match content(&session, &uri) {
            Ok(Some(text)) => V5LspOutcome::Text { text },
            Ok(None) => failed(V5LspFailure::DocumentUnknown),
            Err(failure) => failed(failure),
        }
    }

    fn close(&self, session: &V5LspSessionId) {
        let Some(session) = lock(&self.sessions).remove(session) else {
            return;
        };
        let session = lock(&session);
        // An open buffer keeps its actor from eviction: every actor of the
        // session lets go of this session's buffers.
        for space in session.spaces.values() {
            let _ = space
                .actor
                .documents()
                .close_session(session.documents.session());
        }
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
        self.actor
            .validate_binding(binding)
            .map_err(|_| V5LspFailure::WorkspaceUnavailable)?;
        let artifacts = self
            .actor
            .source_artifacts(binding)
            .map_err(|_| V5LspFailure::WorkspaceUnavailable)?;
        let documents = self
            .actor
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

fn content(session: &Session, uri: &str) -> Result<Option<String>, V5LspFailure> {
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
    session
        .documents
        .content(&uri, &read)
        .map_err(|error| match error {
            LspDocumentError::Read(_) | LspDocumentError::Overlay(_) => V5LspFailure::Unavailable,
            _ => V5LspFailure::DocumentUnknown,
        })
}

/// Admit a workspace folder the way MCP admits its `workspace_hint`: the
/// same discovery, the same platform-XML source sets in name order and the
/// same actor scope, so both reach one actor.
fn admit(actors: &WorkspaceActorRegistry, folder: &std::path::Path) -> Option<AdmittedSpace> {
    let context = discover_workspace(Some(folder.to_path_buf())).ok()?;
    let admission =
        discover_project_source_admission(&context.workspace_root, &mut || Ok(())).ok()?;
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
    Some(AdmittedSpace {
        actor,
        bindings,
        identity,
    })
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

#[cfg(test)]
#[path = "../../../tests/platform/lsp_host.rs"]
mod tests;
